//! Server-rendered pages for the connector: no scripts, one same-origin stylesheet.
use super::oauth::AuthorizeParams;
use axum::{
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use reqwest::Url;
use sticktalk_proto::UserSummary;

const BASE_CSP: &str =
    "default-src 'none'; style-src 'self'; img-src 'self'; frame-ancestors 'none'; base-uri 'none'";

pub(crate) fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}
fn html(status: StatusCode, csp: &str, title: &str, body: &str) -> Response {
    page(status, csp, title, "", body)
}
fn page(status: StatusCode, csp: &str, title: &str, head: &str, body: &str) -> Response {
    let page = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">{head}<title>{}</title><link rel=\"stylesheet\" href=\"/oauth/style.css\"></head><body><main class=\"card\">{body}</main></body></html>",
        esc(title)
    );
    let mut r = (status, page).into_response();
    let h = r.headers_mut();
    h.insert(
        "content-type",
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    h.insert("cache-control", HeaderValue::from_static("no-store"));
    if let Ok(v) = HeaderValue::from_str(csp) {
        h.insert("content-security-policy", v);
    }
    r
}
pub(crate) fn error(status: StatusCode, message: &str) -> Response {
    html(
        status,
        &format!("{BASE_CSP}; form-action 'none'"),
        "StickTalk sign-in",
        &format!(
            "<h1>Can't connect this app</h1><p>{}</p><p class=\"muted\">Close this tab and try again from your AI app.</p>",
            esc(message)
        ),
    )
}
pub(crate) fn hop(target: &str) -> Response {
    let target = esc(target);
    page(
        StatusCode::OK,
        &format!("{BASE_CSP}; form-action 'none'"),
        "StickTalk sign-in",
        &format!("<meta http-equiv=\"refresh\" content=\"0;url={target}\">"),
        &format!("<p>Opening StickTalk sign-in… <a href=\"{target}\">Continue</a></p>"),
    )
}
/// CSP source for the redirect so Chrome lets the form's 303 leave the page.
fn redirect_source(uri: &str) -> String {
    match Url::parse(uri) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => u.origin().ascii_serialization(),
        Ok(u) => format!("{}:", u.scheme()),
        Err(_) => "'none'".into(),
    }
}
fn redirect_label(uri: &str) -> String {
    match Url::parse(uri) {
        Ok(u) if u.scheme() == "https" => u.host_str().unwrap_or_default().to_owned(),
        Ok(u) if u.scheme() == "http" => "an app on this computer".into(),
        Ok(u) => format!("the {} app", u.scheme()),
        Err(_) => "the app".into(),
    }
}
pub(crate) struct Consent<'a> {
    pub client_name: &'a str,
    pub redirect_uri: &'a str,
    pub user: Option<&'a UserSummary>,
    pub csrf: &'a str,
    pub params: &'a AuthorizeParams,
    pub write: bool,
    pub error: Option<&'a str>,
    pub server: &'a str,
}
fn oauth_fields(p: &AuthorizeParams) -> Vec<(&'static str, &str)> {
    [
        ("response_type", &p.response_type),
        ("client_id", &p.client_id),
        ("redirect_uri", &p.redirect_uri),
        ("code_challenge", &p.code_challenge),
        ("code_challenge_method", &p.code_challenge_method),
        ("state", &p.state),
        ("scope", &p.scope),
        ("resource", &p.resource),
    ]
    .into_iter()
    .filter_map(|(k, v)| v.as_deref().map(|v| (k, v)))
    .collect()
}
pub(crate) fn consent(c: Consent) -> Response {
    let mut hidden = String::new();
    for (k, v) in oauth_fields(c.params) {
        hidden.push_str(&format!(
            "<input type=\"hidden\" name=\"{k}\" value=\"{}\">",
            esc(v)
        ));
    }
    hidden.push_str(&format!(
        "<input type=\"hidden\" name=\"csrf\" value=\"{}\">",
        esc(c.csrf)
    ));
    let mut switch = oauth_fields(c.params);
    switch.push(("st_login", "1"));
    let switch = serde_urlencoded::to_string(&switch).unwrap_or_default();
    let who = match c.user {
        Some(u) => format!(
            "<p class=\"who\">Signed in as <b>{}</b> <span class=\"muted\">@{}</span> · <a href=\"/oauth/authorize?{}\">Use another account</a></p>",
            esc(u.display_name.as_deref().unwrap_or(&u.username)),
            esc(&u.username),
            esc(&switch)
        ),
        None => format!(
            "<p>Sign in to <b>{}</b> to continue.</p><label>Username<input name=\"username\" autocomplete=\"username\" autocapitalize=\"none\" spellcheck=\"false\" maxlength=\"64\" value=\"{}\"></label><label>Password<input name=\"password\" type=\"password\" autocomplete=\"current-password\" maxlength=\"128\"></label><p class=\"muted small\">Signed in with Google? Open <a href=\"/\" target=\"_blank\" rel=\"noopener\">StickTalk</a> in this browser, sign in, then reload this page.</p>",
            esc(c.server),
            esc(c.params.username.as_deref().unwrap_or_default())
        ),
    };
    let error = c
        .error
        .map(|e| format!("<p class=\"error\" role=\"alert\">{}</p>", esc(e)))
        .unwrap_or_default();
    let (read_checked, write_checked) = if c.write {
        ("", " checked")
    } else {
        (" checked", "")
    };
    let body = format!(
        "<h1>Connect <b>{name}</b> to StickTalk</h1>\
<p class=\"muted small\">“{name}” is the name the app gave itself; StickTalk can't verify it. Only continue if you just added StickTalk in that app.</p>\
{error}<form method=\"post\" action=\"/oauth/authorize\">{hidden}{who}\
<fieldset><legend>It will act as you and be able to</legend>\
<label class=\"choice\"><input type=\"radio\" name=\"access\" value=\"read\"{read_checked}><span><b>Read only</b><br><span class=\"muted\">See your servers, channels, DMs and messages, and search them.</span></span></label>\
<label class=\"choice\"><input type=\"radio\" name=\"access\" value=\"write\"{write_checked}><span><b>Read and send</b><br><span class=\"muted\">Also send, edit and delete your messages, react, and mark channels read.</span></span></label>\
</fieldset>\
<p class=\"muted small\">You'll return to {dest}. Disconnect any time: Settings › Devices › Log out.</p>\
<div class=\"actions\"><button class=\"primary\" name=\"decision\" value=\"allow\">Allow</button><button class=\"secondary\" name=\"decision\" value=\"deny\">Cancel</button></div>\
</form>",
        name = esc(c.client_name),
        dest = esc(&redirect_label(c.redirect_uri)),
    );
    page(
        StatusCode::OK,
        &format!(
            "{BASE_CSP}; form-action 'self' {}",
            redirect_source(c.redirect_uri)
        ),
        &format!("Connect {} to StickTalk", c.client_name),
        // The server-wide no-referrer policy makes browsers send `Origin: null` on
        // form POSTs, which the same-origin approval check must reject. Cross-origin
        // requests (the redirect back to the app) still carry no referrer.
        "<meta name=\"referrer\" content=\"same-origin\">",
        &body,
    )
}
/// What a person sees when they open the connector URL in a browser.
pub(crate) fn help(url: &str) -> Response {
    let url = esc(url);
    html(
        StatusCode::OK,
        &format!("{BASE_CSP}; form-action 'none'"),
        "StickTalk MCP connector",
        &format!(
            "<h1>Connect an AI assistant to StickTalk</h1>\
<p>This is your server's MCP connector URL:</p><pre class=\"url\">{url}</pre>\
<ol><li>In your AI app, add a custom connector / remote MCP server and paste the URL. Claude: Settings › Connectors › Add custom connector. Claude Code: <code>claude mcp add --transport http sticktalk {url}</code>.</li>\
<li>Sign in to StickTalk when asked and choose <b>Read only</b> or <b>Read and send</b>.</li>\
<li>The assistant can now list your servers and DMs, read and search messages and, if allowed, send them as you.</li></ol>\
<p class=\"muted small\">Each connection appears under Settings › Devices, where Log out disconnects it.</p>"
        ),
    )
}
pub(crate) async fn stylesheet() -> Response {
    let mut r = CSS.into_response();
    r.headers_mut()
        .insert("content-type", HeaderValue::from_static("text/css"));
    r.headers_mut().insert(
        "cache-control",
        HeaderValue::from_static("public, max-age=3600"),
    );
    r
}
// UI_SPEC tokens: bg.chat, bg.elevated, divider, text.*, accent, danger.
const CSS: &str = ":root{color-scheme:dark}*{box-sizing:border-box}\
body{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;background:#111113;color:#E8EAED;font:15px/1.375 Inter,system-ui,-apple-system,'Segoe UI',sans-serif;padding:16px}\
.card{width:100%;max-width:440px;background:#1C1C20;border:1px solid #2A2A2F;border-radius:12px;padding:24px}\
h1{font-size:20px;font-weight:600;margin:0 0 8px}h1 b{color:#3DDC97;font-weight:600}\
p{margin:0 0 16px}.muted{color:#A8A8B0}.small{font-size:12px;color:#74747C}\
a{color:#3DDC97}code,pre{font:13px ui-monospace,Consolas,monospace;background:#26262B;border-radius:4px}code{padding:0 4px}\
pre.url{padding:12px;overflow-wrap:anywhere;white-space:pre-wrap;user-select:all}ol{padding-left:24px;margin:0 0 16px}li{margin-bottom:8px}\
label{display:block;margin:0 0 12px;color:#A8A8B0;font-size:12px;font-weight:600}\
input:not([type=radio]){display:block;width:100%;margin-top:4px;padding:12px;background:#26262B;color:#E8EAED;border:1px solid #2A2A2F;border-radius:8px;font:inherit}\
input:focus,button:focus-visible{outline:2px solid #3DDC97;outline-offset:1px}\
fieldset{border:0;padding:0;margin:0 0 16px}legend{font-size:12px;font-weight:600;color:#74747C;text-transform:uppercase;letter-spacing:.5px;margin-bottom:8px}\
.choice{display:flex;gap:12px;align-items:flex-start;padding:12px;border:1px solid #2A2A2F;border-radius:8px;margin-bottom:8px;font-size:15px;font-weight:400;color:#E8EAED;cursor:pointer}\
.choice:has(input:checked){background:#2E2E34;border-color:#3DDC97}.choice input{margin-top:4px;accent-color:#3DDC97}.choice .muted{font-size:12px}\
.who{padding:12px;background:#26262B;border-radius:8px}\
.error{padding:12px;border-radius:8px;background:rgba(242,95,92,.12);color:#F25F5C}\
.actions{display:flex;flex-direction:row-reverse;gap:8px}\
button{flex:1;padding:12px 16px;border:0;border-radius:8px;font:600 15px Inter,system-ui,sans-serif;cursor:pointer}\
.primary{background:#3DDC97;color:#08170F}.primary:hover{background:#5BE5AA}\
.secondary{background:#26262B;color:#E8EAED}.secondary:hover{background:#2E2E34}";
