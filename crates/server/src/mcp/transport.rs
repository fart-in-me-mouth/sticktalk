//! Streamable HTTP transport, stateless: every POST carries one JSON-RPC
//! message and gets one JSON reply. No SSE stream and no Mcp-Session-Id, so an
//! idle connection costs the server nothing.
use super::{pages, tools};
use crate::state::{now, App};
use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{HeaderMap, HeaderValue, Request, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Map, Value};
use sqlx::Row;
use sticktalk_proto::Id;

/// Newest first; an unknown requested version gets the newest.
pub(crate) const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];
const MAX_BODY: usize = 1 << 20;
const INSTRUCTIONS: &str = "StickTalk is a Discord-style chat app. These tools act as the signed-in user, with their permissions. \
Servers contain channels; DMs are channels too. IDs are decimal strings. \
Start with list_servers or list_direct_messages, then list_channels and read_messages. list_unread shows what needs attention. \
Messages you send appear under the user's name: unless the user clearly asked you to post, show the draft and confirm first. \
Mention someone with <@USER_ID>.";

pub(crate) struct Grant {
    pub session: Vec<u8>,
    pub user: Id,
    pub write: bool,
}

fn unauthorized(app: &App, presented: bool) -> Response {
    let metadata = format!(
        "{}/.well-known/oauth-protected-resource/mcp",
        super::issuer(app)
    );
    let challenge = if presented {
        format!("Bearer realm=\"StickTalk\", resource_metadata=\"{metadata}\", error=\"invalid_token\", error_description=\"The access token is invalid or expired\"")
    } else {
        format!("Bearer realm=\"StickTalk\", resource_metadata=\"{metadata}\"")
    };
    let mut r = (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": "invalid_token",
            "error_description": "Connect StickTalk from your AI app to sign in.",
        })),
    )
        .into_response();
    if let Ok(v) = HeaderValue::from_str(&challenge) {
        r.headers_mut().insert("www-authenticate", v);
    }
    r.headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    r
}
async fn authenticate(app: &App, h: &HeaderMap) -> std::result::Result<Grant, Box<Response>> {
    let Some(value) = h.get("authorization") else {
        return Err(Box::new(unauthorized(app, false)));
    };
    let token = value
        .to_str()
        .ok()
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|t| t.starts_with("stka_") && t.len() < 128)
        .ok_or_else(|| Box::new(unauthorized(app, true)))?;
    let t = now() as i64;
    let row = sqlx::query("SELECT g.session_hash,g.user_id,g.scope,s.last_used_at FROM mcp_access_tokens a JOIN mcp_grants g ON g.session_hash=a.session_hash JOIN sessions s ON s.token_hash=g.session_hash WHERE a.token_hash=? AND a.expires_at>? AND s.expires_at>? AND NOT EXISTS(SELECT 1 FROM disabled_users d WHERE d.user_id=g.user_id)")
        .bind(super::token_hash(token))
        .bind(t)
        .bind(t)
        .fetch_optional(&app.0.pool)
        .await
        .map_err(|_| Box::new(unavailable()))?
        .ok_or_else(|| Box::new(unauthorized(app, true)))?;
    let session: Vec<u8> = row.get("session_hash");
    if row.get::<i64, _>("last_used_at") < t - 60_000 {
        let _ = sqlx::query("UPDATE sessions SET last_used_at=? WHERE token_hash=?")
            .bind(t)
            .bind(&session)
            .execute(&app.0.pool)
            .await;
    }
    Ok(Grant {
        session,
        user: Id(row.get::<i64, _>("user_id") as u64),
        write: row
            .get::<String, _>("scope")
            .split(' ')
            .any(|s| s == super::SCOPE_WRITE),
    })
}
fn unavailable() -> Response {
    let mut r = (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error": "temporarily_unavailable"})),
    )
        .into_response();
    r.headers_mut()
        .insert("retry-after", HeaderValue::from_static("5"));
    r
}
fn rpc_error(status: StatusCode, id: Value, code: i64, message: &str) -> Response {
    (
        status,
        Json(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})),
    )
        .into_response()
}

pub(crate) async fn post(State(app): State<App>, r: Request<Body>) -> Response {
    let grant = match authenticate(&app, r.headers()).await {
        Ok(g) => g,
        Err(response) => return *response,
    };
    if let Some(v) = r.headers().get("mcp-protocol-version") {
        if !v.to_str().is_ok_and(|v| PROTOCOL_VERSIONS.contains(&v)) {
            return rpc_error(
                StatusCode::BAD_REQUEST,
                Value::Null,
                -32600,
                "Unsupported MCP-Protocol-Version",
            );
        }
    }
    let json_body = r
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(';').next().map(str::trim) == Some("application/json"));
    if !json_body {
        return rpc_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Value::Null,
            -32700,
            "Send application/json",
        );
    }
    let Ok(body) = to_bytes(r.into_body(), MAX_BODY).await else {
        return rpc_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            Value::Null,
            -32600,
            "Request too large",
        );
    };
    let message = match serde_json::from_slice::<Value>(&body) {
        Ok(Value::Object(m)) => m,
        Ok(Value::Array(_)) => {
            return rpc_error(
                StatusCode::BAD_REQUEST,
                Value::Null,
                -32600,
                "JSON-RPC batches are not supported",
            )
        }
        _ => return rpc_error(StatusCode::BAD_REQUEST, Value::Null, -32700, "Parse error"),
    };
    match handle(&app, &grant, message).await {
        Some(reply) => Json(reply).into_response(),
        // Notifications and client responses are acknowledged without a body.
        None => StatusCode::ACCEPTED.into_response(),
    }
}
async fn handle(app: &App, grant: &Grant, m: Map<String, Value>) -> Option<Value> {
    let id = m
        .get("id")
        .cloned()
        .filter(|v| v.is_string() || v.is_number());
    let Some(method) = m.get("method").and_then(Value::as_str) else {
        return id.map(|id| {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32600, "message": "Invalid request"}})
        });
    };
    let id = id?;
    if m.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32600, "message": "jsonrpc must be 2.0"}}),
        );
    }
    let params = m.get("params").and_then(Value::as_object);
    let result = match method {
        "initialize" => {
            let requested = params
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str);
            let version = requested
                .filter(|v| PROTOCOL_VERSIONS.contains(v))
                .unwrap_or(PROTOCOL_VERSIONS[0]);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {
                    "name": "sticktalk",
                    "title": "StickTalk",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => {
            if params
                .and_then(|p| p.get("cursor"))
                .is_some_and(|c| !c.is_null())
            {
                Err((
                    -32602,
                    "Invalid cursor: all tools fit on one page".to_owned(),
                ))
            } else {
                Ok(json!({"tools": tools::list(grant.write)}))
            }
        }
        "tools/call" => {
            let name = params.and_then(|p| p.get("name")).and_then(Value::as_str);
            let args = params.and_then(|p| p.get("arguments"));
            match (name, args) {
                (None, _) => Err((-32602, "params.name is required".to_owned())),
                (Some(_), Some(a)) if !a.is_object() && !a.is_null() => {
                    Err((-32602, "params.arguments must be an object".to_owned()))
                }
                (Some(name), args) => {
                    let args = args.and_then(Value::as_object).cloned().unwrap_or_default();
                    tools::call(app, grant, name, args).await
                }
            }
        }
        _ => Err((-32601, format!("Method not found: {method}"))),
    };
    Some(match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
        }
    })
}

/// No server-initiated stream. Browsers get a page explaining the URL.
pub(crate) async fn get(State(app): State<App>, h: HeaderMap) -> Response {
    let browser = h
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/html"));
    if browser && !h.contains_key("authorization") {
        return pages::help(&super::resource(&app));
    }
    method_not_allowed()
}
pub(crate) async fn delete() -> Response {
    method_not_allowed()
}
fn method_not_allowed() -> Response {
    let mut r = (
        StatusCode::METHOD_NOT_ALLOWED,
        Json(json!({"error": "method_not_allowed", "error_description": "This MCP server is stateless: POST JSON-RPC messages to this URL."})),
    )
        .into_response();
    r.headers_mut()
        .insert("allow", HeaderValue::from_static("POST"));
    r
}
