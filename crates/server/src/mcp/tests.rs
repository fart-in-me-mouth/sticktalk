use crate::{audit_tests::fixture, state::App};
use axum::{
    body::{to_bytes, Body},
    http::{Request, Response},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};
use tower::ServiceExt;

const VERIFIER: &str = "verifier-verifier-verifier-verifier-verifier";
const REDIRECT: &str = "http://127.0.0.1:4567/callback";

async fn send(app: &App, r: Request<Body>) -> Response<Body> {
    crate::http::router(app.clone()).oneshot(r).await.unwrap()
}
async fn text(r: Response<Body>) -> String {
    String::from_utf8(to_bytes(r.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap()
}
async fn json_body(r: Response<Body>) -> Value {
    serde_json::from_str(&text(r).await).unwrap()
}
fn form(pairs: &[(&str, &str)]) -> String {
    serde_urlencoded::to_string(pairs).unwrap()
}
fn challenge() -> String {
    URL_SAFE_NO_PAD.encode(crate::state::hash(VERIFIER.as_bytes()))
}
/// A browser session cookie for `user`, as the PWA would hold.
async fn cookie(app: &App, user: i64) -> String {
    let token = crate::state::token();
    sqlx::query("INSERT INTO sessions VALUES(?,?,?,?,?)")
        .bind(crate::state::hash(token.as_bytes()))
        .bind(user)
        .bind(crate::state::now() as i64)
        .bind((crate::state::now() + 86_400_000) as i64)
        .bind(crate::state::now() as i64)
        .execute(&app.0.pool)
        .await
        .unwrap();
    format!("__Host-sticktalk_session={token}")
}
async fn register(app: &App) -> String {
    let r = send(
        app,
        Request::post("/oauth/register")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"client_name": "Test Agent", "redirect_uris": [REDIRECT], "grant_types": ["authorization_code", "refresh_token"], "token_endpoint_auth_method": "none"}).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(r.status(), 201);
    json_body(r).await["client_id"].as_str().unwrap().to_owned()
}
fn authorize_query(client_id: &str, scope: &str) -> String {
    form(&[
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", "http://127.0.0.1:9999/callback"),
        ("code_challenge", &challenge()),
        ("code_challenge_method", "S256"),
        ("state", "xyz"),
        ("scope", scope),
        ("resource", "http://localhost/mcp"),
    ])
}
fn hidden(page: &str, name: &str) -> String {
    let marker = format!("name=\"{name}\" value=\"");
    let start = page
        .find(&marker)
        .unwrap_or_else(|| panic!("{name} in {page}"))
        + marker.len();
    page[start..start + page[start..].find('"').unwrap()]
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
}
fn query_param(location: &str, key: &str) -> Option<String> {
    reqwest::Url::parse(location)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}
/// Browser consent as the signed-in user; returns the redirect Location.
async fn approve(
    app: &App,
    client_id: &str,
    cookie: &str,
    access: &str,
    extra: &[(&str, &str)],
) -> String {
    let query = authorize_query(client_id, "chat:read chat:write");
    // Cross-site arrival: no cookie, so the page reloads itself once.
    let r = send(
        app,
        Request::get(format!("/oauth/authorize?{query}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(r.status(), 200);
    assert!(text(r).await.contains("st_hop=1"));
    let r = send(
        app,
        Request::get(format!("/oauth/authorize?{query}&st_hop=1"))
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(r.status(), 200);
    let csp = r.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        csp.contains("form-action 'self' http://127.0.0.1:9999"),
        "{csp}"
    );
    let page = text(r).await;
    // Without this, real browsers send `Origin: null` and the POST is refused.
    assert!(page.contains(r#"<meta name="referrer" content="same-origin">"#));
    assert!(
        page.contains("Test Agent") && page.contains("Signed in as"),
        "{page}"
    );
    let mut fields: Vec<(&str, String)> = [
        "response_type",
        "client_id",
        "redirect_uri",
        "code_challenge",
        "code_challenge_method",
        "state",
        "scope",
        "resource",
        "csrf",
    ]
    .into_iter()
    .map(|k| (k, hidden(&page, k)))
    .collect();
    fields.push(("decision", "allow".into()));
    fields.push(("access", access.into()));
    for (k, v) in extra {
        fields.retain(|(name, _)| name != k);
        fields.push((k, (*v).into()));
    }
    let body = serde_urlencoded::to_string(&fields).unwrap();
    let r = send(
        app,
        Request::post("/oauth/authorize")
            .header("origin", "http://localhost")
            .header("cookie", cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap(),
    )
    .await;
    assert_eq!(r.status(), 303, "{}", text(r).await);
    r.headers()["location"].to_str().unwrap().to_owned()
}
async fn token(app: &App, pairs: &[(&str, &str)]) -> Response<Body> {
    send(
        app,
        Request::post("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(form(pairs)))
            .unwrap(),
    )
    .await
}
async fn connect(app: &App, user: i64, access: &str) -> (String, String, String) {
    connect_with(app, user, access, &[]).await
}
/// `extra` are further consent form fields, as the person would set them.
async fn connect_with(
    app: &App,
    user: i64,
    access: &str,
    extra: &[(&str, &str)],
) -> (String, String, String) {
    let client_id = register(app).await;
    let cookie = cookie(app, user).await;
    let location = approve(app, &client_id, &cookie, access, extra).await;
    assert_eq!(query_param(&location, "state").as_deref(), Some("xyz"));
    assert_eq!(
        query_param(&location, "iss").as_deref(),
        Some("http://localhost")
    );
    let code = query_param(&location, "code").unwrap();
    let r = token(
        app,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", "http://127.0.0.1:9999/callback"),
            ("client_id", &client_id),
            ("code_verifier", VERIFIER),
            ("resource", "http://localhost/mcp"),
        ],
    )
    .await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["cache-control"], "no-store");
    let t = json_body(r).await;
    assert_eq!(t["token_type"], "Bearer");
    // The code is single use.
    let again = token(
        app,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
            ("code_verifier", VERIFIER),
        ],
    )
    .await;
    assert_eq!(json_body(again).await["error"], "invalid_grant");
    (
        t["access_token"].as_str().unwrap().to_owned(),
        t["refresh_token"].as_str().unwrap().to_owned(),
        client_id,
    )
}
async fn rpc(app: &App, access: &str, method: &str, params: Value) -> Response<Body> {
    send(
        app,
        Request::post("/mcp")
            .header("authorization", format!("Bearer {access}"))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(Body::from(
                json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params}).to_string(),
            ))
            .unwrap(),
    )
    .await
}
async fn tool(app: &App, access: &str, name: &str, args: Value) -> Value {
    let r = rpc(
        app,
        access,
        "tools/call",
        json!({"name": name, "arguments": args}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let v = json_body(r).await;
    assert_eq!(v["id"], 7);
    v["result"].clone()
}

#[tokio::test]
async fn discovery_and_unauthenticated_requests() {
    let (_dir, app) = fixture().await;
    let r = send(
        &app,
        Request::post("/mcp")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await;
    assert_eq!(r.status(), 401);
    let challenge = r.headers()["www-authenticate"].to_str().unwrap().to_owned();
    assert!(challenge.contains(
        "resource_metadata=\"http://localhost/.well-known/oauth-protected-resource/mcp\""
    ));
    assert!(!challenge.contains("invalid_token"));
    let r = rpc(&app, "stka_nope", "ping", json!({})).await;
    assert_eq!(r.status(), 401);
    assert!(r.headers()["www-authenticate"]
        .to_str()
        .unwrap()
        .contains("error=\"invalid_token\""));
    let prm = json_body(
        send(
            &app,
            Request::get("/.well-known/oauth-protected-resource/mcp")
                .body(Body::empty())
                .unwrap(),
        )
        .await,
    )
    .await;
    assert_eq!(prm["resource"], "http://localhost/mcp");
    assert_eq!(prm["authorization_servers"][0], "http://localhost");
    let r = send(
        &app,
        Request::get("/.well-known/oauth-authorization-server")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(r.headers()["access-control-allow-origin"], "*");
    let asm = json_body(r).await;
    assert_eq!(asm["issuer"], "http://localhost");
    assert_eq!(asm["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(
        asm["registration_endpoint"],
        "http://localhost/oauth/register"
    );
    let r = send(
        &app,
        Request::get("/mcp")
            .header("accept", "text/html")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(r.status(), 200);
    assert!(text(r).await.contains("http://localhost/mcp"));
    let r = send(
        &app,
        Request::get("/mcp")
            .header("accept", "text/event-stream")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(r.status(), 405);
    let r = send(
        &app,
        Request::options("/oauth/token")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(r.status(), 204);
    assert!(r.headers()["access-control-allow-headers"]
        .to_str()
        .unwrap()
        .contains("mcp-protocol-version"));
    // Registration refuses redirects a third party could abuse.
    for bad in [
        "http://evil.example/cb",
        "javascript:alert(1)",
        "https://x.example/#f",
    ] {
        let r = send(
            &app,
            Request::post("/oauth/register")
                .header("content-type", "application/json")
                .body(Body::from(json!({"redirect_uris": [bad]}).to_string()))
                .unwrap(),
        )
        .await;
        assert_eq!(json_body(r).await["error"], "invalid_redirect_uri", "{bad}");
    }
}

#[tokio::test]
async fn full_flow_reads_writes_refreshes_and_detects_reuse() {
    let (_dir, app) = fixture().await;
    let (access, refresh, client_id) = connect(&app, 1, "write").await;
    let init = json_body(rpc(&app, &access, "initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}})).await).await;
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "sticktalk");
    let newer = json_body(
        rpc(
            &app,
            &access,
            "initialize",
            json!({"protocolVersion": "2099-01-01"}),
        )
        .await,
    )
    .await;
    assert_eq!(
        newer["result"]["protocolVersion"],
        super::transport::PROTOCOL_VERSIONS[0]
    );
    let r = send(
        &app,
        Request::post("/mcp")
            .header("authorization", format!("Bearer {access}"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(r.status(), 202);
    let tools = json_body(rpc(&app, &access, "tools/list", json!({})).await).await;
    let names: Vec<_> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        names.contains(&"send_message".to_owned()) && names.contains(&"read_messages".to_owned())
    );
    let delete = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "delete_message")
        .unwrap();
    assert_eq!(delete["annotations"]["destructiveHint"], true);

    let servers = tool(&app, &access, "list_servers", json!({})).await;
    assert_eq!(servers["isError"], false);
    assert_eq!(servers["structuredContent"]["servers"][0]["id"], "10");
    let channels = tool(&app, &access, "list_channels", json!({"server_id": "10"})).await;
    assert_eq!(
        channels["structuredContent"]["channels"][0]["name"],
        "general"
    );
    assert_eq!(
        channels["structuredContent"]["channels"][0]["can_send"],
        true
    );

    let sent = tool(
        &app,
        &access,
        "send_message",
        json!({"channel_id": "11", "content": "hello from the agent <@2>"}),
    )
    .await;
    assert_eq!(sent["isError"], false, "{sent}");
    let message_id = sent["structuredContent"]["message"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        sent["structuredContent"]["message"]["mentions"][0]["username"],
        "user2"
    );
    // The connector's backing session must not keep a replay stream alive.
    let session: Vec<u8> = sqlx::query_scalar("SELECT session_hash FROM mcp_grants")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert!(!app.0.core.lock().await.streams.contains_key(&session));
    // Other users see it: unread for user 2.
    let unread: i64 = sqlx::query_scalar(
        "SELECT unread_count FROM read_states WHERE user_id=2 AND channel_id=11",
    )
    .fetch_one(&app.0.pool)
    .await
    .unwrap();
    assert_eq!(unread, 1);
    let read = tool(
        &app,
        &access,
        "read_messages",
        json!({"channel_id": "11", "limit": 500}),
    )
    .await;
    let messages = read["structuredContent"]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["content"], "hello from the agent <@2>");
    assert_eq!(messages[0]["author"]["username"], "user1");
    assert_eq!(
        read["structuredContent"]["channel"]["server"]["name"],
        "guild"
    );
    let search = tool(
        &app,
        &access,
        "search_messages",
        json!({"server_id": "10", "query": "agent"}),
    )
    .await;
    assert_eq!(
        search["structuredContent"]["results"][0]["message"]["id"],
        message_id.as_str(),
        "{search}"
    );
    let reacted = tool(
        &app,
        &access,
        "add_reaction",
        json!({"channel_id": "11", "message_id": message_id, "emoji": "👍"}),
    )
    .await;
    assert_eq!(reacted["structuredContent"]["count"], 1, "{reacted}");
    let edited = tool(
        &app,
        &access,
        "edit_message",
        json!({"channel_id": "11", "message_id": message_id, "content": "edited"}),
    )
    .await;
    assert_eq!(edited["structuredContent"]["message"]["content"], "edited");
    let guarded = tool(
        &app,
        &access,
        "delete_message",
        json!({"channel_id": "11", "message_id": message_id}),
    )
    .await;
    assert_eq!(guarded["isError"], true);
    let unconfirmed = tool(
        &app,
        &access,
        "delete_message",
        json!({"channel_id": "11", "message_id": message_id, "confirm": false}),
    )
    .await;
    assert_eq!(
        unconfirmed["structuredContent"]["error"]["code"],
        "confirmation_required"
    );
    let deleted = tool(
        &app,
        &access,
        "delete_message",
        json!({"channel_id": "11", "message_id": message_id, "confirm": true}),
    )
    .await;
    assert_eq!(deleted["structuredContent"]["deleted"], true);
    // Agent-facing errors are tool results with guidance, not protocol errors.
    let missing = tool(&app, &access, "read_messages", json!({"channel_id": "999"})).await;
    assert_eq!(missing["isError"], true);
    assert!(missing["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("list_channels"));
    let unknown_arg = tool(&app, &access, "list_servers", json!({"bogus": 1})).await;
    assert!(unknown_arg["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Accepted arguments: none"));
    let unknown = json_body(rpc(&app, &access, "tools/call", json!({"name": "nope"})).await).await;
    assert_eq!(unknown["error"]["code"], -32602);
    // New DMs need friendship; the agent is told why instead of a bare refusal.
    let dm = tool(&app, &access, "open_dm", json!({"username": "user3"})).await;
    assert_eq!(
        dm["structuredContent"]["error"]["code"], "forbidden",
        "{dm}"
    );
    assert!(dm["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("friends"));

    // Refresh rotates; replaying any spent refresh token revokes the grant.
    let r = token(
        &app,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", &client_id),
        ],
    )
    .await;
    assert_eq!(r.status(), 200);
    let t = json_body(r).await;
    let access2 = t["access_token"].as_str().unwrap().to_owned();
    assert_ne!(t["refresh_token"].as_str().unwrap(), refresh);
    assert_eq!(
        tool(&app, &access2, "get_me", json!({})).await["structuredContent"]["access"],
        "read_and_send"
    );
    // Rotate once more, then replay the first (twice-rotated) token.
    let r = token(
        &app,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", t["refresh_token"].as_str().unwrap()),
            ("client_id", &client_id),
        ],
    )
    .await;
    assert_eq!(r.status(), 200);
    let access2 = json_body(r).await["access_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let replay = token(
        &app,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", &client_id),
        ],
    )
    .await;
    assert_eq!(json_body(replay).await["error"], "invalid_grant");
    assert_eq!(rpc(&app, &access2, "ping", json!({})).await.status(), 401);
    let grants: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mcp_grants")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(grants, 0);
}

#[tokio::test]
async fn read_only_grant_and_device_revocation() {
    let (_dir, app) = fixture().await;
    let (access, _, _) = connect(&app, 2, "read").await;
    let tools = json_body(rpc(&app, &access, "tools/list", json!({})).await).await;
    let tools = tools["result"]["tools"].as_array().unwrap();
    assert!(tools
        .iter()
        .all(|t| t["annotations"]["readOnlyHint"] == true));
    let denied = tool(
        &app,
        &access,
        "send_message",
        json!({"channel_id": "11", "content": "hi"}),
    )
    .await;
    assert_eq!(
        denied["structuredContent"]["error"]["code"],
        "read_only_connection"
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    // The connection is listed under Devices and Log out disconnects it.
    let (name, platform, id): (String, String, String) = sqlx::query_as("SELECT d.device_name,d.platform,d.id FROM session_devices d JOIN mcp_grants g ON g.session_hash=d.token_hash")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(
        (name.as_str(), platform.as_str()),
        ("Test Agent", "AI connector (MCP)")
    );
    let mut core = app.0.core.lock().await;
    let r = crate::commands::execute(
        &app,
        &mut core,
        &[2; 32],
        sticktalk_proto::Id(2),
        sticktalk_proto::CommandRequest {
            nonce: 1,
            command: sticktalk_proto::Command::Launch(
                sticktalk_proto::LaunchCommand::LogoutSession { session_id: id },
            ),
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(r.outcome, sticktalk_proto::CommandOutcome::Ok(_)),
        "{r:?}"
    );
    drop(core);
    assert_eq!(rpc(&app, &access, "ping", json!({})).await.status(), 401);
}

#[tokio::test]
async fn consent_rejects_forgery_and_mismatches() {
    let (_dir, app) = fixture().await;
    let client_id = register(&app).await;
    let cookie = cookie(&app, 1).await;
    let query = authorize_query(&client_id, "");
    // Unregistered redirect: an error page, never a redirect.
    let bad = query.replace("127.0.0.1%3A9999", "evil.example");
    let r = send(
        &app,
        Request::get(format!("/oauth/authorize?{bad}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(r.status(), 400);
    assert!(r.headers().get("location").is_none());
    // Missing PKCE: redirect back with an error.
    let no_pkce = query.replace("code_challenge_method=S256", "code_challenge_method=plain");
    let r = send(
        &app,
        Request::get(format!("/oauth/authorize?{no_pkce}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(r.status(), 303);
    assert_eq!(
        query_param(r.headers()["location"].to_str().unwrap(), "error").as_deref(),
        Some("invalid_request")
    );
    // Cross-origin POST is refused outright.
    let r = send(
        &app,
        Request::post("/oauth/authorize")
            .header("origin", "https://evil.example")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!("{query}&decision=allow&access=write")))
            .unwrap(),
    )
    .await;
    assert_eq!(r.status(), 403);
    // Same origin but no valid CSRF token: re-rendered, no code issued.
    let r = send(
        &app,
        Request::post("/oauth/authorize")
            .header("origin", "http://localhost")
            .header("cookie", &cookie)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!(
                "{query}&decision=allow&access=write&csrf=forged"
            )))
            .unwrap(),
    )
    .await;
    assert_eq!(r.status(), 200);
    assert!(text(r).await.contains("expired"));
    let codes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mcp_codes")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(codes, 0);
    // A wrong PKCE verifier spends the code.
    let location = approve(&app, &client_id, &cookie, "read", &[]).await;
    let code = query_param(&location, "code").unwrap();
    let r = token(
        &app,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
            ("code_verifier", &"x".repeat(43)),
        ],
    )
    .await;
    assert_eq!(json_body(r).await["error"], "invalid_grant");
    let r = token(
        &app,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
            ("code_verifier", VERIFIER),
        ],
    )
    .await;
    assert_eq!(json_body(r).await["error"], "invalid_grant");
    // Deny returns access_denied.
    let location = approve(&app, &client_id, &cookie, "read", &[("decision", "deny")]).await;
    assert_eq!(
        query_param(&location, "error").as_deref(),
        Some("access_denied")
    );
}

#[tokio::test]
async fn password_sign_in_on_consent_page() {
    let (_dir, app) = fixture().await;
    let hash = crate::http::password_hash("correct horse battery".into()).unwrap();
    sqlx::query("UPDATE users SET password_hash=? WHERE id=3")
        .bind(hash)
        .execute(&app.0.pool)
        .await
        .unwrap();
    let client_id = register(&app).await;
    let query = authorize_query(&client_id, "chat:read");
    let r = send(
        &app,
        Request::get(format!("/oauth/authorize?{query}&st_hop=1"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let page = text(r).await;
    assert!(page.contains("name=\"password\""));
    // Read and send is preselected even though the client asked for chat:read.
    assert!(page.contains("value=\"write\" checked"));
    let csrf = hidden(&page, "csrf");
    let post = |password: &'static str| {
        let body = format!(
            "{query}&csrf={}&decision=allow&access=read&{}",
            form(&[("c", &csrf)]).trim_start_matches("c="),
            form(&[("username", "User3"), ("password", password)])
        );
        Request::post("/oauth/authorize")
            .header("origin", "http://localhost")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap()
    };
    let r = send(&app, post("wrong password!!")).await;
    assert!(text(r).await.contains("password didn&#39;t match"));
    let r = send(&app, post("correct horse battery")).await;
    assert_eq!(r.status(), 303);
    let code = query_param(r.headers()["location"].to_str().unwrap(), "code").unwrap();
    let user: i64 = sqlx::query_scalar("SELECT user_id FROM mcp_codes")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(user, 3);
    let r = token(
        &app,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
            ("code_verifier", VERIFIER),
        ],
    )
    .await;
    assert_eq!(json_body(r).await["scope"], "chat:read");
}

#[tokio::test]
async fn screen_control_is_its_own_approval_and_needs_a_call() {
    let (_dir, app) = fixture().await;
    let names = |tools: &Value| -> Vec<String> {
        tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect()
    };
    // "Read and send" alone does not include it.
    let (chat, _, _) = connect(&app, 2, "write").await;
    let listed = names(&json_body(rpc(&app, &chat, "tools/list", json!({})).await).await);
    assert!(!listed.iter().any(|n| n.contains("screen")), "{listed:?}");
    for (name, args) in [
        ("list_screen_shares", json!({})),
        ("screen_act", json!({"actions": []})),
    ] {
        let refused = tool(&app, &chat, name, args).await;
        assert_eq!(
            refused["structuredContent"]["error"]["code"],
            "screen_control_not_allowed"
        );
    }
    assert_eq!(
        tool(&app, &chat, "get_me", json!({})).await["structuredContent"]["screen_control"],
        false
    );

    // A link cannot tick the box for the person.
    let client_id = register(&app).await;
    let session = cookie(&app, 3).await;
    let query = authorize_query(&client_id, "chat:read screen:control");
    let page = text(
        send(
            &app,
            Request::get(format!("/oauth/authorize?{query}&st_hop=1&screen=1"))
                .header("cookie", &session)
                .body(Body::empty())
                .unwrap(),
        )
        .await,
    )
    .await;
    assert!(page.contains(r#"name="screen" value="1"><span><b>Control shared screens"#));

    // Ticked, it is granted beside either chat access.
    let (agent, _, _) = connect_with(&app, 3, "read", &[("screen", "1")]).await;
    let listed = names(&json_body(rpc(&app, &agent, "tools/list", json!({})).await).await);
    for name in [
        "list_screen_shares",
        "request_screen_control",
        "screen_screenshot",
        "screen_act",
        "release_screen_control",
    ] {
        assert!(listed.contains(&name.to_owned()), "{name}");
    }
    assert!(!listed.contains(&"send_message".to_owned()));
    let scope: String = sqlx::query_scalar("SELECT scope FROM mcp_grants WHERE user_id=3")
        .fetch_one(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(scope, "chat:read screen:control");
    assert_eq!(
        tool(&app, &agent, "get_me", json!({})).await["structuredContent"]["screen_control"],
        true
    );

    // Outside a call there is nothing to see or control, and the agent is told why.
    let shares = tool(&app, &agent, "list_screen_shares", json!({})).await;
    assert_eq!(shares["structuredContent"]["in_call"], false);
    assert!(shares["structuredContent"]["hint"]
        .as_str()
        .unwrap()
        .contains("join the voice channel"));
    let code = |v: Value| v["structuredContent"]["error"]["code"].clone();
    assert_eq!(
        code(
            tool(
                &app,
                &agent,
                "request_screen_control",
                json!({"user_id": "1"})
            )
            .await
        ),
        "not_in_call"
    );
    assert_eq!(
        code(tool(&app, &agent, "screen_screenshot", json!({})).await),
        "not_controlling"
    );
    assert_eq!(
        code(
            tool(
                &app,
                &agent,
                "screen_act",
                json!({"actions": [{"type": "key", "key": "w"}]})
            )
            .await
        ),
        "not_controlling"
    );
    assert_eq!(
        tool(&app, &agent, "release_screen_control", json!({})).await["structuredContent"]
            ["released"],
        false
    );

    // A picture nobody is waiting for is refused, from anyone.
    let frame = |token: Option<&str>| {
        let mut r =
            Request::post("/api/v1/voice/control/frames/5").header("content-type", "image/jpeg");
        if let Some(token) = token {
            r = r.header("authorization", format!("Bearer {token}"));
        }
        r.body(Body::from(vec![0xff, 0xd8, 0xff, 0xd9])).unwrap()
    };
    assert_eq!(send(&app, frame(None)).await.status(), 401);
    let token = session.trim_start_matches("__Host-sticktalk_session=");
    assert_eq!(send(&app, frame(Some(token))).await.status(), 404);
    // An MCP token is not a session: it cannot post pictures either.
    assert_eq!(send(&app, frame(Some(&agent))).await.status(), 401);
}
