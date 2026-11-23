use super::*;
use crate::audit_tests::fixture;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message as Frame},
};

async fn call(
    app: &App,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: serde_json::Value,
) -> Response {
    let mut r = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    for (name, value) in headers {
        r = r.header(*name, *value);
    }
    crate::http::router(app.clone())
        .oneshot(
            r.body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
}
fn cookies(response: &Response) -> String {
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap())
        .collect::<Vec<_>>()
        .join("; ")
}

#[tokio::test]
async fn revoked_browser_session_probe_expires_http_only_credential() {
    let (_dir, app) = fixture().await;
    let token = state::token();
    let cookie = format!("{SESSION}={token}; {CSRF}={}", csrf(&token));
    let response = call(
        &app,
        "GET",
        "/api/v1/browser/session",
        &[("cookie", &cookie)],
        json!(null),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    for name in [SESSION, CSRF] {
        assert!(response.headers().get_all("set-cookie").iter().any(|h| {
            let h = h.to_str().unwrap();
            h.starts_with(&format!("{name}=;")) && h.contains("Max-Age=0")
        }));
    }
    // A Google signup has a CSRF cookie bound to its signup ticket, but no
    // session yet. A normal unauthenticated boot probe must not erase it.
    let signup_cookie = format!("{SIGNUP}=signup-ticket; {CSRF}=signup-csrf");
    let response = call(
        &app,
        "GET",
        "/api/v1/browser/session",
        &[("cookie", &signup_cookie)],
        json!(null),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(!response.headers().contains_key("set-cookie"));
}
async fn value(r: Response) -> serde_json::Value {
    serde_json::from_slice(&to_bytes(r.into_body(), 65536).await.unwrap()).unwrap()
}

#[tokio::test]
async fn browser_login_csrf_all_routes_logout_and_bearer_unchanged() {
    let (_dir, app) = fixture().await;
    let payload = json!({"username":"browser_user","password":"strong test password"});
    let r = call(
        &app,
        "POST",
        "/api/v1/browser/session/register",
        &[],
        payload.clone(),
    )
    .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = call(
        &app,
        "POST",
        "/api/v1/browser/session/register",
        &[("origin", "http://localhost")],
        payload,
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    for c in r.headers().get_all("set-cookie") {
        let c = c.to_str().unwrap();
        assert!(c.contains("Secure") && c.contains("SameSite=Strict") && c.contains("Path=/"));
        if c.starts_with(SESSION) {
            assert!(c.contains("HttpOnly"));
        }
    }
    let cookies = cookies(&r);
    let session = value(r).await;
    assert!(session.get("token").is_none());
    let csrf = session["csrf_token"].as_str().unwrap();
    let r = call(
        &app,
        "GET",
        "/api/v1/me",
        &[("cookie", &cookies)],
        json!(null),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["cache-control"], "no-store");
    let r = call(
        &app,
        "GET",
        "/api/v1/browser/session",
        &[("cookie", &cookies)],
        json!(null),
    )
    .await;
    assert_eq!(value(r).await["csrf_token"], csrf);
    for path in [
        "/api/v1/uploads",
        "/api/v1/uploads/resumable",
        "/api/auth/google/link",
        "/api/v1/auth/logout",
    ] {
        let r = call(
            &app,
            "POST",
            path,
            &[("cookie", &cookies), ("origin", "http://localhost")],
            json!({}),
        )
        .await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{path}");
    }
    for (origin, csrf_value) in [("https://evil.test", csrf), ("http://localhost", "wrong")] {
        let r = call(
            &app,
            "POST",
            "/api/v1/auth/logout",
            &[
                ("cookie", &cookies),
                ("origin", origin),
                ("x-sticktalk-csrf", csrf_value),
            ],
            json!({}),
        )
        .await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
    }
    let native = state::token();
    sqlx::query("UPDATE sessions SET token_hash=? WHERE user_id=1")
        .bind(state::hash(native.as_bytes()))
        .execute(&app.0.pool)
        .await
        .unwrap();
    let native_header = format!("Bearer {native}");
    let r = call(
        &app,
        "GET",
        "/api/v1/me",
        &[
            ("authorization", &native_header),
            ("cookie", "__Host-sticktalk_session=bad"),
        ],
        json!(null),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let r = call(
        &app,
        "POST",
        "/api/v1/auth/logout",
        &[
            ("cookie", &cookies),
            ("origin", "http://localhost"),
            ("x-sticktalk-csrf", csrf),
        ],
        json!({}),
    )
    .await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert!(r
        .headers()
        .get_all("set-cookie")
        .iter()
        .all(|v| v.to_str().unwrap().contains("Max-Age=0")));
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/me",
            &[("cookie", &cookies)],
            json!(null)
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/auth/logout",
            &[("authorization", &native_header)],
            json!({})
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
}

#[test]
fn duplicate_cookie_and_csp_config_rejected() {
    let mut h = HeaderMap::new();
    h.insert(
        "cookie",
        "__Host-sticktalk_session=a; __Host-sticktalk_session=b"
            .parse()
            .unwrap(),
    );
    assert!(session_cookie(&h).is_err());
    for value in [
        "https://cdn.test/path",
        "https://*.test",
        "https://u:p@cdn.test",
        "https://cdn.test?q=1",
    ] {
        assert!(origin(value).is_err());
    }
    assert!(hashed_asset("/pkg/app-0123456789abcdef.wasm"));
    for path in [
        "/index.html",
        "/manifest-0123456789.json",
        "/sw-0123456789.js",
        "/service-worker.0123456789.js",
        "/foo.html",
        "/app.js",
    ] {
        assert!(!hashed_asset(path));
    }
}
#[tokio::test]
async fn direct_gif_csp_matches_meta_capability_without_wildcards() {
    let (_dir, app) = fixture().await;
    let mut config = app.0.config.clone();
    config.gifs.gif_provider = Some(crate::gifs::ProviderOption::Klipy);
    config.gifs.gif_public_client_key = Some("public-client-key".into());
    for (mode, key, enabled) in [
        (crate::gifs::GifMode::Direct, true, true),
        (crate::gifs::GifMode::Direct, false, false),
        (crate::gifs::GifMode::Proxy, true, false),
    ] {
        config.gifs.gif_mode = mode;
        config.gifs.gif_public_client_key = key.then(|| "public-client-key".into());
        let capability = crate::gifs::GifService::new(&config.gifs)
            .unwrap()
            .capability();
        assert_eq!(
            matches!(capability, GifCapability::Enabled(GifConfig::Direct { .. })),
            enabled
        );
        let browser = Browser::new(&config).unwrap();
        let csp = browser.csp.to_str().unwrap();
        assert_eq!(csp.contains("https://api.klipy.com"), enabled);
        for host in ["static.klipy.com", "static1.klipy.com", "static2.klipy.com"] {
            assert_eq!(csp.contains(&format!("https://{host}")), enabled);
        }
        assert!(!csp.contains('*'));
        assert!(csp.contains("script-src 'self' 'wasm-unsafe-eval';"));
    }
}
#[tokio::test]
async fn static_security_headers_compression_caching_and_no_api_fallback() {
    let (dir, mut app) = fixture().await;
    let root = dir.path().join("web");
    std::fs::create_dir(&root).unwrap();
    for (name, bytes) in [
        ("index.html", b"app shell".as_slice()),
        ("app-0123456789.wasm", b"uncompressed"),
        ("app-0123456789.wasm.br", b"brotli variant"),
        ("app-0123456789.wasm.gz", b"gzip variant"),
        ("sw.js", b"worker"),
        ("manifest.webmanifest", b"{}"),
    ] {
        std::fs::write(root.join(name), bytes).unwrap();
    }
    let inner = Arc::get_mut(&mut app.0).unwrap();
    inner.config.browser.web_root = Some(root);
    inner.config.browser.gif_media_origins = vec!["https://media.klipy.com".into()];
    inner.browser = Browser::new(&inner.config).unwrap();
    for path in ["/", "/index.html", "/sw.js", "/manifest.webmanifest"] {
        let r = call(&app, "GET", path, &[], json!(null)).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()["cache-control"], "no-cache");
        let csp = r.headers()["content-security-policy"].to_str().unwrap();
        assert!(csp.contains("'wasm-unsafe-eval'") && csp.contains("https://media.klipy.com"));
        let csp = r.headers()["content-security-policy"].to_str().unwrap();
        assert!(csp.contains("script-src 'self' 'wasm-unsafe-eval';"));
        assert!(csp.contains("style-src 'self'; style-src-attr 'unsafe-inline';"));
        assert!(!csp.contains("script-src 'self' 'unsafe-inline'"));
        assert!(!r.headers().contains_key("cross-origin-embedder-policy"));
        assert_eq!(r.headers()["referrer-policy"], "no-referrer");
        assert_eq!(r.headers()["x-content-type-options"], "nosniff");
        assert!(r.headers()["permissions-policy"]
            .to_str()
            .unwrap()
            .contains("display-capture=(self)"));
    }
    for (accept, encoding, expected) in [
        ("br, gzip", "br", b"brotli variant".as_slice()),
        ("gzip", "gzip", b"gzip variant"),
    ] {
        let r = call(
            &app,
            "GET",
            "/app-0123456789.wasm",
            &[("accept-encoding", accept)],
            json!(null),
        )
        .await;
        assert_eq!(r.headers()["content-encoding"], encoding);
        assert_eq!(
            r.headers()["cache-control"],
            "public, max-age=31536000, immutable"
        );
        assert_eq!(r.headers()["content-type"], "application/wasm");
        assert_eq!(&to_bytes(r.into_body(), 1024).await.unwrap()[..], expected);
    }
    let r = call(&app, "GET", "/app-0123456789.wasm", &[], json!(null)).await;
    let modified = r.headers()["last-modified"].to_str().unwrap().to_owned();
    let r = call(
        &app,
        "GET",
        "/app-0123456789.wasm",
        &[("if-modified-since", &modified)],
        json!(null),
    )
    .await;
    assert_eq!(r.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(
        r.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    let r = call(
        &app,
        "GET",
        "/app-0123456789.wasm",
        &[("service-worker", "script")],
        json!(null),
    )
    .await;
    assert_eq!(r.headers()["cache-control"], "no-cache");
    for path in [
        "/api/missing",
        "/api/v1/missing",
        "/%2e%2e/sticktalk.sqlite3",
        "/..%5csticktalk.sqlite3",
    ] {
        assert!(!call(&app, "GET", path, &[], json!(null))
            .await
            .status()
            .is_success());
    }
}

#[tokio::test]
async fn cookie_websocket_origin_auth_identify_and_focus() {
    let (_dir, app) = fixture().await;
    let secret = state::token();
    sqlx::query("UPDATE sessions SET token_hash=? WHERE user_id=1")
        .bind(state::hash(secret.as_bytes()))
        .execute(&app.0.pool)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let serve = tokio::spawn(axum::serve(listener, crate::http::router(app.clone())).into_future());
    let url = format!("ws://{address}/api/v1/ws");
    for origin in [None, Some("https://evil.test")] {
        let mut r = url.clone().into_client_request().unwrap();
        r.headers_mut()
            .insert("sec-websocket-protocol", "sticktalk.v1".parse().unwrap());
        r.headers_mut()
            .insert("cookie", format!("{SESSION}={secret}").parse().unwrap());
        if let Some(o) = origin {
            r.headers_mut().insert("origin", o.parse().unwrap());
        }
        assert!(connect_async(r).await.is_err());
    }
    let mut r = url.into_client_request().unwrap();
    r.headers_mut()
        .insert("sec-websocket-protocol", "sticktalk.v1".parse().unwrap());
    r.headers_mut()
        .insert("cookie", format!("{SESSION}={secret}").parse().unwrap());
    r.headers_mut()
        .insert("origin", "http://localhost".parse().unwrap());
    let (mut ws, _) = connect_async(r).await.unwrap();
    assert!(matches!(
        ws.next().await.unwrap().unwrap(),
        Frame::Binary(_)
    ));
    ws.send(Frame::Binary(
        rmp_serde::to_vec_named(&ClientMsg::Identify(Identify {
            text_features: false,
            version: 1,
            token: String::new(),
            resume: None,
            ..Default::default()
        }))
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    loop {
        let frame = ws.next().await.unwrap().unwrap();
        if let Frame::Binary(b) = frame {
            if matches!(
                rmp_serde::from_slice::<ServerMsg>(&b).unwrap(),
                ServerMsg::Ready(_)
            ) {
                break;
            }
        }
    }
    ws.send(Frame::Binary(
        rmp_serde::to_vec_named(&ClientMsg::Focus { focused: true })
            .unwrap()
            .into(),
    ))
    .await
    .unwrap();
    ws.send(Frame::Binary(
        rmp_serde::to_vec_named(&ClientMsg::Heartbeat(Heartbeat {
            nonce: 1,
            last_seq: 0,
        }))
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    loop {
        if let Frame::Binary(b) = ws.next().await.unwrap().unwrap() {
            if matches!(
                rmp_serde::from_slice::<ServerMsg>(&b).unwrap(),
                ServerMsg::HeartbeatAck(_)
            ) {
                break;
            }
        }
    }
    assert!(
        app.0.core.lock().await.streams[&state::hash(secret.as_bytes())]
            .connection
            .as_ref()
            .unwrap()
            .focused
    );
    ws.close(None).await.unwrap();
    serve.abort();
    app.shutdown();
}

use std::{future::IntoFuture, sync::Arc};
#[tokio::test]
async fn history_extent_exact_counts_gaps_acl_and_index_plans() {
    let (_dir, app) = fixture().await;
    let secret = state::token();
    sqlx::query("UPDATE sessions SET token_hash=? WHERE user_id=1")
        .bind(state::hash(secret.as_bytes()))
        .execute(&app.0.pool)
        .await
        .unwrap();
    let auth = format!("Bearer {secret}");
    let path = "/api/v1/channels/11/history-extent";
    let empty =
        value(call(&app, "GET", path, &[("authorization", &auth)], json!(null)).await).await;
    assert_eq!(empty["data"]["message_count"], 0);
    sqlx::query("WITH RECURSIVE n(x) AS (SELECT 100 UNION ALL SELECT x+1 FROM n WHERE x<10099) INSERT INTO messages(id,channel_id,author_id,content,created_at) SELECT x,11,1,'x',0 FROM n").execute(&app.0.pool).await.unwrap();
    sqlx::query("UPDATE messages SET deleted_at=1 WHERE id BETWEEN 500 AND 2500")
        .execute(&app.0.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM messages WHERE id BETWEEN 9000 AND 9099")
        .execute(&app.0.pool)
        .await
        .unwrap();
    let r = value(call(&app, "GET", path, &[("authorization", &auth)], json!(null)).await).await;
    let data: &serde_json::Value = &r["data"];
    assert_eq!(data["message_count"], 7899);
    let anchors = data["anchors"].as_array().unwrap();
    assert!(anchors.len() <= 65);
    assert_eq!(anchors.first().unwrap()["message_id"], 100);
    assert_eq!(anchors.last().unwrap()["message_id"], 10099);
    assert_eq!(anchors.last().unwrap()["rank"], 7898);
    for a in anchors {
        let id = a["message_id"].as_u64().unwrap();
        assert!(!(500..=2500).contains(&id) && !(9000..=9099).contains(&id));
    }
    for query in ["SELECT message_count FROM history_extents WHERE channel_id=11","SELECT ordinal,message_id FROM history_entries WHERE channel_id=11 AND ordinal>=6000 ORDER BY ordinal LIMIT 1","SELECT ordinal,message_id FROM history_entries WHERE channel_id=11 ORDER BY ordinal DESC LIMIT 1"] {
        use sqlx::Row;
        let plan=sqlx::query(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {query}"))).fetch_all(&app.0.pool).await.unwrap();let plan=plan.iter().map(|r|r.get::<String,_>("detail")).collect::<Vec<_>>().join(" ");assert!(plan.contains("SEARCH"),"{plan}");assert!(!plan.contains("SCAN")&&!plan.contains("TEMP"),"{plan}");
    }
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/channels/999/history-extent",
            &[("authorization", &auth)],
            json!(null)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    sqlx::query("INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(12,'voice',10,'voice',1,0)").execute(&app.0.pool).await.unwrap();
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/channels/12/history-extent",
            &[("authorization", &auth)],
            json!(null)
        )
        .await
        .status(),
        // A voice channel keeps its own text chat, so it has a history.
        StatusCode::OK
    );
    sqlx::query("DELETE FROM channels WHERE id=11")
        .execute(&app.0.pool)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM history_entries")
            .fetch_one(&app.0.pool)
            .await
            .unwrap(),
        0
    );
}
