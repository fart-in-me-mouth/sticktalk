//! Installed-app OAuth: system browser, loopback only, PKCE S256, bounded I/O.
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const CLIENT_ID: Option<&str> = option_env!("STICKTALK_GOOGLE_CLIENT_ID");
const CLIENT_SECRET: Option<&str> = option_env!("STICKTALK_GOOGLE_CLIENT_SECRET");
pub fn configured(server_id: Option<&str>) -> bool {
    matches!((server_id, CLIENT_ID, CLIENT_SECRET), (Some(a), Some(b), Some(c)) if !a.is_empty() && a == b && !c.is_empty())
}
use sticktalk_client_core::oauth::{callback, Proof};

pub async fn sign_in(
    client_id: String,
    nonce: String,
    accepted: tokio::sync::oneshot::Sender<()>,
) -> Result<String, String> {
    if !configured(Some(&client_id)) {
        return Err("Google sign-in is not available for this build and server.".into());
    }
    tokio::time::timeout(
        Duration::from_secs(120),
        flow(
            client_id,
            nonce,
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
    nonce: String,
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
    let proof = Proof::new(&crate::platform::Native)?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| "Could not open the sign-in callback. Please try again.".to_string())?;
    let port = listener
        .local_addr()
        .map_err(|_| "Could not start sign-in".to_string())?
        .port();
    let redirect = format!("http://127.0.0.1:{port}");
    let url = proof
        .authorization_url(&authorization_endpoint, &client_id, &redirect, &nonce)
        .expect("constant URL");
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
    sticktalk_client_core::oauth::exchange(
        &crate::platform::NativeHttp(client),
        token_endpoint,
        sticktalk_client_core::oauth::TokenExchange {
            client_id: &client_id,
            client_secret,
            code: &code,
            verifier: &proof.verifier,
            redirect: &redirect,
        },
    )
    .await
}
#[cfg(test)]
mod flow_tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use sha2::{Digest, Sha256};
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
        let token=tokio::time::timeout(Duration::from_secs(5),flow("fixture-client".into(),"n".repeat(43),"fixture-secret","http://127.0.0.1/mock-authorization".into(),endpoint,Some(accepted_tx),move |url|async move {
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
            "n".repeat(43),
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
            "n".repeat(43),
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
