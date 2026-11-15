//! "Is there a StickTalk server at this address?" Asked once when a person
//! presses Connect or Retry, never in the background.

/// `GET /api/v1/health` answers `{"version":"…","protocol_version":…}`.
pub fn is_health(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("version").map(serde_json::Value::is_string))
        .unwrap_or(false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    NotFound,
    Refused,
    TimedOut,
    Untrusted,
    /// The server (or a proxy in front of it) answered with this 5xx status.
    Unwell(u16),
    NotStickTalk,
    Other,
}

/// Plain language for the built-in page; no error codes or library names.
pub fn explain(failure: Failure, host: &str) -> String {
    match failure {
        Failure::NotFound => {
            format!("Couldn't find {host}. Check the address for typos, and that you're online.")
        }
        Failure::Refused => format!(
            "Couldn't connect to {host}. Check that the server is running and that you're online."
        ),
        Failure::TimedOut => {
            format!("{host} took too long to answer. Check your connection and try again.")
        }
        Failure::Untrusted => format!(
            "Couldn't make a secure connection to {host}. This PC doesn't trust its certificate."
        ),
        Failure::Unwell(status) => {
            format!("The server at {host} is having trouble (error {status}). Try again shortly.")
        }
        Failure::NotStickTalk => {
            format!("{host} answered, but it isn't a StickTalk server. Check the address.")
        }
        Failure::Other => format!("Couldn't reach {host}. Try again."),
    }
}

#[cfg(windows)]
pub fn check(server: &crate::address::Server) -> Result<(), Failure> {
    use std::time::Duration;
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    // Windows' own TLS stack and certificate store, like the WebView itself.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .max_redirects(0)
        .max_redirects_will_error(false)
        .user_agent(concat!("StickTalk-Desktop/", env!("CARGO_PKG_VERSION")))
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();
    let mut response = agent.get(server.health()).call().map_err(|error| {
        use std::io::ErrorKind;
        match error {
            ureq::Error::StatusCode(status) if status >= 500 => Failure::Unwell(status),
            ureq::Error::StatusCode(_) => Failure::NotStickTalk,
            ureq::Error::HostNotFound => Failure::NotFound,
            ureq::Error::Timeout(_) => Failure::TimedOut,
            ureq::Error::ConnectionFailed => Failure::Refused,
            ureq::Error::Tls(_) | ureq::Error::NativeTls(_) => Failure::Untrusted,
            ureq::Error::Io(io) => match io.kind() {
                ErrorKind::TimedOut => Failure::TimedOut,
                ErrorKind::ConnectionRefused
                | ErrorKind::ConnectionReset
                | ErrorKind::ConnectionAborted
                | ErrorKind::NotConnected
                | ErrorKind::HostUnreachable
                | ErrorKind::NetworkUnreachable => Failure::Refused,
                // Schannel reports a failed handshake as "invalid data".
                ErrorKind::InvalidData => Failure::Untrusted,
                _ => Failure::Other,
            },
            _ => Failure::Other,
        }
    })?;
    if response.status() != 200 {
        // Redirects are not followed: the address must be the server itself.
        return Err(Failure::NotStickTalk);
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_string()
        .map_err(|_| Failure::NotStickTalk)?;
    if is_health(&body) {
        Ok(())
    } else {
        Err(Failure::NotStickTalk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_a_sticktalk_health_reply() {
        assert!(is_health(r#"{"version":"0.1.2","protocol_version":1}"#));
        assert!(is_health(r#"{"version":"9.9.9"}"#));
        for body in [
            "",
            "OK",
            "<!doctype html><title>Router login</title>",
            "null",
            "[]",
            r#"{"status":"ok"}"#,
            r#"{"version":1}"#,
            r#""version""#,
        ] {
            assert!(!is_health(body), "{body:?}");
        }
    }

    #[test]
    fn every_failure_reads_as_a_sentence_about_the_host() {
        for failure in [
            Failure::NotFound,
            Failure::Refused,
            Failure::TimedOut,
            Failure::Untrusted,
            Failure::Unwell(502),
            Failure::NotStickTalk,
            Failure::Other,
        ] {
            let text = explain(failure, "chat.example.com");
            assert!(text.contains("chat.example.com"), "{text}");
            assert!(text.ends_with('.'), "{text}");
            for jargon in ["ureq", "Error", "Io(", "TLS", "HTTP", "os error"] {
                assert!(!text.contains(jargon), "{text}");
            }
        }
        assert!(explain(Failure::Unwell(503), "h").contains("503"));
    }
}
