use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};
pub struct Proof {
    pub verifier: String,
    pub state: String,
}
impl Proof {
    pub fn new(entropy: &impl crate::io::Entropy) -> Result<Self, String> {
        let mut bytes = [0; 64];
        entropy.fill(&mut bytes)?;
        Ok(Self {
            verifier: URL_SAFE_NO_PAD.encode(&bytes[..32]),
            state: URL_SAFE_NO_PAD.encode(&bytes[32..]),
        })
    }
    pub fn authorization_url(
        &self,
        endpoint: &str,
        client_id: &str,
        redirect: &str,
    ) -> Result<String, String> {
        let mut url =
            url::Url::parse(endpoint).map_err(|_| "Invalid authorization URL".to_string())?;
        url.query_pairs_mut().extend_pairs([
            ("client_id", client_id),
            ("response_type", "code"),
            ("scope", "openid email profile"),
            ("redirect_uri", redirect),
            ("code_challenge", self.challenge().as_str()),
            ("code_challenge_method", "S256"),
            ("state", self.state.as_str()),
            ("prompt", "select_account"),
        ]);
        Ok(url.to_string())
    }
    pub fn challenge(&self) -> String {
        URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes()))
    }
}
pub fn state_matches(expected: &str, actual: &str) -> bool {
    // Fixed-size generated states; do not short-circuit comparisons of equal lengths.
    expected.len() == actual.len()
        && expected
            .bytes()
            .zip(actual.bytes())
            .fold(0u8, |d, (a, b)| d | (a ^ b))
            == 0
}
pub fn callback(request: &str, port: u16, state: &str) -> Result<String, String> {
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
    let url = url::Url::parse(&format!("http://127.0.0.1:{port}{}", parts[1]))
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

/// Google token exchange policy shared by host flows; only the callback listener/browser are native.
pub struct TokenExchange<'a> {
    pub client_id: &'a str,
    pub client_secret: &'a str,
    pub code: &'a str,
    pub verifier: &'a str,
    pub redirect: &'a str,
}
pub async fn exchange(
    adapter: &impl crate::http::HttpTransport,
    endpoint: String,
    proof: TokenExchange<'_>,
) -> Result<String, String> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("client_id", proof.client_id),
            ("client_secret", proof.client_secret),
            ("code", proof.code),
            ("code_verifier", proof.verifier),
            ("redirect_uri", proof.redirect),
            ("grant_type", "authorization_code"),
        ])
        .finish()
        .into_bytes();
    let response = adapter
        .execute(crate::http::HttpRequest {
            method: "POST",
            url: endpoint,
            headers: vec![(
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            )],
            body: crate::http::Body::Bytes(body),
            max_response: 32768,
            timeout_ms: 25_000,
            network_error: "Could not reach Google. Please try again.",
        })
        .await?;
    if !(200..300).contains(&response.status) {
        return Err("Google could not complete sign-in. Please try again.".into());
    }
    #[derive(serde::Deserialize)]
    struct Tokens {
        id_token: String,
    }
    let token: Tokens = serde_json::from_slice(&response.bytes)
        .map_err(|_| "Google did not return a sign-in token".to_string())?;
    if token.id_token.is_empty() {
        return Err("Google did not return a sign-in token".into());
    }
    Ok(token.id_token)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestEntropy;
    impl crate::io::Entropy for TestEntropy {
        fn fill(&self, bytes: &mut [u8]) -> Result<(), String> {
            static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let seed = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as u8;
            for (i, byte) in bytes.iter_mut().enumerate() {
                *byte = seed.wrapping_add(i as u8);
            }
            Ok(())
        }
    }
    #[test]
    fn pkce_rfc7636_vector_and_random_state() {
        let p = Proof {
            verifier: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into(),
            state: String::new(),
        };
        assert_eq!(p.challenge(), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let a = Proof::new(&TestEntropy).unwrap();
        let b = Proof::new(&TestEntropy).unwrap();
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
