//! Installed-app OAuth: system browser, loopback only, PKCE S256, bounded I/O.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const CLIENT_ID: Option<&str> = option_env!("STICKTALK_GOOGLE_CLIENT_ID");
const CLIENT_SECRET: Option<&str> = option_env!("STICKTALK_GOOGLE_CLIENT_SECRET");
pub fn configured(server_id: Option<&str>) -> bool {
    matches!((server_id, CLIENT_ID, CLIENT_SECRET), (Some(a), Some(b), Some(c)) if !a.is_empty() && a == b && !c.is_empty())
}
struct Proof {
    verifier: String,
    state: String,
}
impl Proof {
    fn new() -> Result<Self, String> {
        let mut bytes = [0; 64];
        getrandom::fill(&mut bytes).map_err(|_| "Could not start secure sign-in".to_string())?;
        Ok(Self {
            verifier: URL_SAFE_NO_PAD.encode(&bytes[..32]),
            state: URL_SAFE_NO_PAD.encode(&bytes[32..]),
        })
    }
    fn challenge(&self) -> String {
        URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes()))
    }
}
fn state_matches(expected: &str, actual: &str) -> bool {
    // Fixed-size generated states; do not short-circuit comparisons of equal lengths.
    expected.len() == actual.len()
        && expected
            .bytes()
            .zip(actual.bytes())
            .fold(0u8, |d, (a, b)| d | (a ^ b))
            == 0
}
fn callback(request: &str, port: u16, state: &str) -> Result<String, String> {
    if request.len() > 8192 || !request.ends_with("\r\n\r\n") {
        return Err("Invalid Google callback".into());
    }
    let mut lines = request.split("\r\n");
    let parts: Vec<_> = lines.next().unwrap_or_default().split(' ').collect();
    if parts.len() != 3
        || parts[0] != "GET"
        || parts[2] != "HTTP/1.1"
        || !parts[1].starts_with("/?")
    {
        return Err("Invalid Google callback".into());
    }
    let hosts: Vec<_> = lines
        .filter_map(|l| l.split_once(':'))
        .filter(|(k, _)| k.eq_ignore_ascii_case("host"))
        .collect();
    if hosts.len() != 1 || hosts[0].1.trim() != format!("127.0.0.1:{port}") {
        return Err("Invalid callback host".into());
    }
    let url = reqwest::Url::parse(&format!("http://127.0.0.1:{port}{}", parts[1]))
        .map_err(|_| "Invalid Google callback".to_string())?;
    let pairs: Vec<_> = url.query_pairs().collect();
    let one = |key| -> Result<Option<&str>, String> {
        let values: Vec<_> = pairs
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_ref())
            .collect();
        if values.len() > 1 {
            Err("Duplicate callback parameter".into())
        } else {
            Ok(values.first().copied())
        }
    };
    if !state_matches(state, one("state")?.unwrap_or_default()) {
        return Err("Google sign-in state did not match. Please try again.".into());
    }
    if one("error")?.is_some() {
        return Err("Google sign-in was cancelled or denied.".into());
    }
    one("code")?
        .filter(|v| !v.is_empty() && v.len() <= 4096)
        .map(str::to_owned)
        .ok_or_else(|| "Missing Google authorization code".into())
}

pub async fn sign_in(
    client_id: String,
    accepted: tokio::sync::oneshot::Sender<()>,
) -> Result<String, String> {
    if !configured(Some(&client_id)) {
        return Err("Google sign-in is not available for this build and server.".into());
    }
    tokio::time::timeout(
        Duration::from_secs(120),
        flow(
            client_id,
            CLIENT_SECRET.unwrap_or_default(),
            "https://accounts.google.com/o/oauth2/v2/auth".into(),
            "https://oauth2.googleapis.com/token".into(),
            Some(accepted),
            |url| async move {
                tokio::task::spawn_blocking(move || open::that(url))
                    .await
                    .map_err(|_| "Could not open your browser".to_string())?
                    .map_err(|_| {
                        "Could not open your browser. Check your default browser settings.".into()
                    })
            },
        ),
    )
    .await
    .map_err(|_| "Google sign-in timed out after 2 minutes. Please try again.".to_string())?
}
pub async fn flow<F, Fut>(
    client_id: String,
    client_secret: &str,
    authorization_endpoint: String,
    token_endpoint: String,
    accepted: Option<tokio::sync::oneshot::Sender<()>>,
    open_browser: F,
) -> Result<String, String>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let proof = Proof::new()?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| "Could not open the sign-in callback. Please try again.".to_string())?;
    let port = listener
        .local_addr()
        .map_err(|_| "Could not start sign-in".to_string())?
        .port();
    let redirect = format!("http://127.0.0.1:{port}");
    let mut url = reqwest::Url::parse(&authorization_endpoint).expect("constant URL");
    url.query_pairs_mut().extend_pairs([
        ("client_id", client_id.as_str()),
        ("response_type", "code"),
        ("scope", "openid email profile"),
        ("redirect_uri", redirect.as_str()),
        ("code_challenge", proof.challenge().as_str()),
        ("code_challenge_method", "S256"),
        ("state", proof.state.as_str()),
        ("prompt", "select_account"),
    ]);
    let url = url.to_string();
    open_browser(url).await?;
    let code = loop {
        let (mut stream, peer) = listener
            .accept()
            .await
            .map_err(|_| "Sign-in callback failed".to_string())?;
        if !peer.ip().is_loopback() {
            continue;
        }
        let mut bytes = Vec::new();
        let read = tokio::time::timeout(Duration::from_secs(3), async {
            while !bytes.ends_with(b"\r\n\r\n") {
                let mut chunk = [0u8; 512];
                let n = stream.read(&mut chunk).await?;
                if n == 0 || bytes.len() + n > 8192 {
                    return Err(std::io::Error::other("Invalid request"));
                }
                bytes.extend_from_slice(&chunk[..n]);
            }
            Ok::<_, std::io::Error>(())
        })
        .await;
        let result = match read {
            Ok(Ok(())) => std::str::from_utf8(&bytes)
                .map_err(|_| "Invalid callback".into())
                .and_then(|s| callback(s, port, &proof.state)),
            _ => Err("Invalid callback".into()),
        };
        let success = result.is_ok();
        let denied = result
            .as_ref()
            .is_err_and(|e| e == "Google sign-in was cancelled or denied.");
        let body = if success {
            "<!doctype html><meta charset=utf-8><title>StickTalk</title><p>Signed in — you can close this tab.</p>"
        } else {
            "<!doctype html><meta charset=utf-8><title>StickTalk</title><p>Sign-in was not completed. Return to StickTalk to try again.</p>"
        };
        let response = format!("HTTP/1.1 {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nX-Content-Type-Options: nosniff\r\n\r\n{}", if success { "200 OK" } else { "400 Bad Request" }, body.len(), body);
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            stream.write_all(response.as_bytes()),
        )
        .await;
        if denied {
            return result;
        }
        if let Ok(code) = result {
            break code;
        }
        // Ignore unrelated requests, favicon probes and spoofed state. The global deadline still applies.
    };
    drop(listener);
    if let Some(accepted) = accepted {
        let _ = accepted.send(());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(25))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not initialize Google sign-in".to_string())?;
    let response = client
        .post(token_endpoint)
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret),
            ("code", code.as_str()),
            ("code_verifier", proof.verifier.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|_| "Could not reach Google. Please try again.".to_string())?;
    let status = response.status();
    let bytes = crate::backend::limited(response, 32768).await?;
    if !status.is_success() {
        return Err("Google could not complete sign-in. Please try again.".into());
    }
    #[derive(serde::Deserialize)]
    struct Tokens {
        id_token: String,
    }
    let token: Tokens = serde_json::from_slice(&bytes)
        .map_err(|_| "Google did not return a sign-in token".to_string())?;
    if token.id_token.is_empty() {
        return Err("Google did not return a sign-in token".into());
    }
    Ok(token.id_token)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pkce_rfc7636_vector_and_random_state() {
        let p = Proof {
            verifier: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into(),
            state: String::new(),
        };
        assert_eq!(p.challenge(), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let a = Proof::new().unwrap();
        let b = Proof::new().unwrap();
        assert_eq!(a.verifier.len(), 43);
        assert_eq!(a.state.len(), 43);
        assert_ne!(a.verifier, b.verifier);
        assert_ne!(a.state, b.state);
        assert_ne!(a.state, a.verifier);
        assert!(a
            .verifier
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c)));
        assert!(state_matches(&a.state, &a.state));
        assert!(!state_matches(&a.state, &b.state));
        assert!(!state_matches(&a.state, ""));
    }
    #[test]
    fn loopback_parser_rejects_spoofing_duplicates_and_denial() {
        let req = |q: &str| format!("GET /?{q} HTTP/1.1\r\nHost: 127.0.0.1:12345\r\n\r\n");
        assert_eq!(
            callback(&req("state=abc&code=a%2Bb"), 12345, "abc").unwrap(),
            "a+b"
        );
        for q in [
            "code=x",
            "state=bad&code=x",
            "state=abc&code=",
            "state=abc&state=abc&code=x",
            "state=abc&code=x&code=y",
            "state=abc&error=access_denied",
        ] {
            assert!(callback(&req(q), 12345, "abc").is_err());
        }
        for s in [
            req("state=abc&code=x").replace("GET", "POST"),
            req("state=abc&code=x").replace("127.0.0.1:12345", "evil.example"),
            req("state=abc&code=x").replace("/?", "/favicon.ico?"),
            "x".repeat(8193),
        ] {
            assert!(callback(&s, 12345, "abc").is_err());
        }
    }
}

#[cfg(test)]
mod flow_tests {
    use super::*;
    use std::collections::BTreeMap;
    async fn request(stream: &mut tokio::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        loop {
            let mut b = [0; 1024];
            let n = stream.read(&mut b).await.unwrap();
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&b[..n]);
            if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&bytes[..end]);
                let len = header
                    .lines()
                    .find_map(|v| {
                        v.split_once(':')
                            .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + len {
                    break;
                }
            }
            assert!(bytes.len() < 32768);
        }
        String::from_utf8(bytes).unwrap()
    }
    #[tokio::test]
    async fn full_oauth_exchange_uses_mock_authorization_and_token_servers_only() {
        let tokens = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/token", tokens.local_addr().unwrap());
        let (challenge_tx, challenge_rx) = tokio::sync::oneshot::channel::<String>();
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let token_server = tokio::spawn(async move {
            let (mut socket, _) = tokens.accept().await.unwrap();
            let req = request(&mut socket).await;
            assert!(
                accepted_rx.await.is_ok(),
                "callback progress precedes token response"
            );
            assert!(req.starts_with("POST /token HTTP/1.1"));
            let form = req.split_once("\r\n\r\n").unwrap().1;
            let params: BTreeMap<_, _> = reqwest::Url::parse(&format!("http://fixture/?{form}"))
                .unwrap()
                .query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            assert_eq!(params["code"], "mock-code");
            assert_eq!(params["client_id"], "fixture-client");
            assert_eq!(params["client_secret"], "fixture-secret");
            assert_eq!(params["grant_type"], "authorization_code");
            assert_eq!(
                URL_SAFE_NO_PAD.encode(Sha256::digest(params["code_verifier"].as_bytes())),
                challenge_rx.await.unwrap()
            );
            let body = r#"{"id_token":"mock-id-token","access_token":"never-sent-to-sticktalk"}"#;
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        });
        let token=tokio::time::timeout(Duration::from_secs(5),flow("fixture-client".into(),"fixture-secret","http://127.0.0.1/mock-authorization".into(),endpoint,Some(accepted_tx),move |url|async move {
            let auth=reqwest::Url::parse(&url).unwrap();assert_eq!(auth.host_str(),Some("127.0.0.1"));
            let params:BTreeMap<_,_>=auth.query_pairs().map(|(k,v)|(k.into_owned(),v.into_owned())).collect();
            assert_eq!(params["code_challenge_method"],"S256");assert_eq!(params["scope"],"openid email profile");
            let _=challenge_tx.send(params["code_challenge"].clone());
            tokio::spawn(async move {
                let redirect=reqwest::Url::parse(&params["redirect_uri"]).unwrap();assert_eq!(redirect.host_str(),Some("127.0.0.1"));let port=redirect.port().unwrap();
                // A spoofed state must not consume the pending flow.
                for state in ["spoofed",params["state"].as_str()] {
                    let mut socket=tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST,port)).await.unwrap();
                    let req=format!("GET /?code=mock-code&state={state} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
                    // Exercise fragmented header reads too.
                    let middle=req.len()/2;socket.write_all(&req.as_bytes()[..middle]).await.unwrap();socket.write_all(&req.as_bytes()[middle..]).await.unwrap();
                    let mut response=String::new();socket.read_to_string(&mut response).await.unwrap();
                    if state=="spoofed" {assert!(response.starts_with("HTTP/1.1 400"));}else{assert!(response.contains("Signed in — you can close this tab"));}
                }
            });Ok(())
        })).await.unwrap().unwrap();
        assert_eq!(token, "mock-id-token");
        token_server.await.unwrap();
    }
    #[tokio::test]
    async fn timeout_and_cancellation_drop_the_loopback_listener() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let flow = tokio::spawn(flow(
            "fixture".into(),
            "secret",
            "http://127.0.0.1/auth".into(),
            "http://127.0.0.1/token".into(),
            None,
            |url| async move {
                let auth = reqwest::Url::parse(&url).unwrap();
                let redirect = auth
                    .query_pairs()
                    .find(|(k, _)| k == "redirect_uri")
                    .unwrap()
                    .1
                    .into_owned();
                tx.send(redirect).unwrap();
                Ok(())
            },
        ));
        let redirect = reqwest::Url::parse(&rx.await.unwrap()).unwrap();
        let port = redirect.port().unwrap();
        flow.abort();
        assert!(flow.await.unwrap_err().is_cancelled());
        assert!(
            tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
                .await
                .is_err()
        );
        let pending = super::flow(
            "fixture".into(),
            "secret",
            "http://127.0.0.1/auth".into(),
            "http://127.0.0.1/token".into(),
            None,
            |_| async { Ok(()) },
        );
        assert!(tokio::time::timeout(Duration::from_millis(20), pending)
            .await
            .is_err());
    }
}
