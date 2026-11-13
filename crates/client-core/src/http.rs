//! REST routing, validation and response policy independent of fetch/reqwest.
use crate::{
    backend::{AccountAction, Data, Request},
    io::{MaybeSend, MaybeSync, Task},
};
use futures_util::{Stream, StreamExt};
use serde::{de::DeserializeOwned, Serialize};
use std::pin::Pin;
use sticktalk_proto::*;
#[cfg(not(target_arch = "wasm32"))]
pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, String>> + Send>>;
#[cfg(target_arch = "wasm32")]
pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, String>>>>;
pub enum Body {
    Empty,
    Bytes(Vec<u8>),
    Stream { chunks: ByteStream, length: u64 },
}
pub struct HttpRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Body,
    pub max_response: usize,
    pub timeout_ms: u64,
    pub network_error: &'static str,
}
pub struct HttpResponse {
    pub status: u16,
    pub bytes: Vec<u8>,
}
/// Bounded retry policy for idempotent, empty-body reads only. Mutation replies
/// must never be replayed by HTTP adapters: their commit status may be unknown.
pub struct ReadRetry {
    enabled: bool,
    remaining: u8,
    waited_ms: u64,
}
impl ReadRetry {
    pub fn new(empty_get: bool) -> Self {
        Self {
            enabled: empty_get,
            remaining: 3,
            waited_ms: 0,
        }
    }
    pub fn delay(&mut self, response: &HttpResponse) -> Option<u64> {
        if !self.enabled || self.remaining == 0 || response.status != 429 {
            return None;
        }
        let error: ApiError = serde_json::from_slice(&response.bytes).ok()?;
        if error.code != ErrorCode::RateLimited {
            return None;
        }
        let delay = u64::from(error.retry_after_ms.unwrap_or(1000)).max(10);
        if delay > 5000 || self.waited_ms + delay > 10_000 {
            return None;
        }
        self.remaining -= 1;
        self.waited_ms += delay;
        Some(delay)
    }
}
/// Adapters enforce max_response while streaming, disable redirects, and bound time.
/// Browser adapters may add same-origin cookie/CSRF credentials without changing core.
pub trait HttpTransport: Clone + MaybeSend + MaybeSync + 'static {
    fn execute(&self, request: HttpRequest) -> Task<'_, Result<HttpResponse, String>>;
}
#[derive(Clone)]
pub struct Http<A> {
    pub base: String,
    pub token: String,
    pub adapter: A,
}
pub fn validate_server(server: &str) -> Result<String, String> {
    let url =
        url::Url::parse(server).map_err(|_| "Enter a valid http(s) server address".to_string())?;
    if url.scheme() != "https"
        && !(url.scheme() == "http"
            && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")))
    {
        return Err("Use HTTPS; HTTP is allowed only on loopback".into());
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Server address must not contain credentials, query or fragment".into());
    }
    Ok(server.trim_end_matches('/').to_string())
}
pub fn decode_json<T: DeserializeOwned>(response: HttpResponse) -> Result<T, String> {
    if !(200..300).contains(&response.status) {
        return Err(serde_json::from_slice::<ApiError>(&response.bytes).map(|v| {
            if v.code == ErrorCode::LastLoginMethod { return "Set a password before unlinking Google. It is your only sign-in method.".into(); }
            format!("{}{}", v.message, v.retry_after_ms.map_or(String::new(), |t| format!(" Try again in {}s.", t.div_ceil(1000))))
        }).unwrap_or_else(|_| format!("Server returned HTTP {}", response.status)));
    }
    serde_json::from_slice(&response.bytes).map_err(|_| "Server sent an invalid response".into())
}
impl<A: HttpTransport> Http<A> {
    pub fn new(server: &str, token: &str, adapter: A) -> Result<Self, String> {
        Ok(Self {
            base: validate_server(server)?,
            token: token.into(),
            adapter,
        })
    }
    pub async fn execute(
        &self,
        method: &'static str,
        path: &str,
        body: Body,
        headers: Vec<(String, String)>,
        timeout_ms: u64,
    ) -> Result<HttpResponse, String> {
        self.execute_with_error(
            method,
            path,
            body,
            headers,
            timeout_ms,
            "Server request failed",
        )
        .await
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_with_error(
        &self,
        method: &'static str,
        path: &str,
        body: Body,
        mut headers: Vec<(String, String)>,
        timeout_ms: u64,
        network_error: &'static str,
    ) -> Result<HttpResponse, String> {
        if !self.token.is_empty() {
            headers.push(("Authorization".into(), format!("Bearer {}", self.token)));
        }
        self.adapter
            .execute(HttpRequest {
                method,
                url: format!("{}{path}", self.base),
                headers,
                body,
                max_response: 2 * 1024 * 1024,
                timeout_ms,
                network_error,
            })
            .await
    }
    pub async fn json<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        decode_json(
            self.execute(
                "GET",
                &format!("/api/v1{path}"),
                Body::Empty,
                vec![],
                30_000,
            )
            .await?,
        )
    }
    pub async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T, String> {
        self.post_api(&format!("/api/v1{path}"), body).await
    }
    pub async fn post_api<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T, String> {
        self.post_api_with_error(path, body, "Server request failed")
            .await
    }
    pub async fn post_api_with_error<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl Serialize,
        network_error: &'static str,
    ) -> Result<T, String> {
        let bytes = serde_json::to_vec(body).map_err(|_| "Could not encode request".to_string())?;
        decode_json(
            self.execute_with_error(
                "POST",
                path,
                Body::Bytes(bytes),
                vec![("Content-Type".into(), "application/json".into())],
                30_000,
                network_error,
            )
            .await?,
        )
    }
    pub async fn request<R, H>(&self, request: Request<R>) -> Result<Data<H>, String> {
        match request {
            Request::Launch { read, generation } => {
                use crate::launch::{Read, SnapshotData};
                let data = match read {
                    Read::Emoji(id) => SnapshotData::Emoji(
                        id,
                        self.json(&format!("/guilds/{}/emoji", id.0)).await?,
                    ),
                    Read::Profile(id) => {
                        SnapshotData::Profile(self.json(&format!("/users/{}/profile", id.0)).await?)
                    }
                    Read::Pins(id) => SnapshotData::Pins(
                        id,
                        self.json(&format!("/channels/{}/pins", id.0)).await?,
                    ),
                    Read::Overwrites(id) => SnapshotData::Overwrites(
                        id,
                        self.json(&format!("/channels/{}/overwrites", id.0)).await?,
                    ),
                    Read::Permissions(id) => SnapshotData::Permissions(
                        id,
                        self.json(&format!("/channels/{}/permissions", id.0))
                            .await?,
                    ),
                    Read::Extent(id) => SnapshotData::Extent(
                        self.json(&format!("/channels/{}/history-extent", id.0))
                            .await?,
                    ),
                };
                Ok(Data::Launch { data, generation })
            }
            Request::Settings { query, generation } => {
                use crate::settings::{Query, SnapshotData as S};
                let path = query.path();
                macro_rules! get {
                    ($map:expr) => {{
                        let v: Snapshot<_> = self.json(&path).await?;
                        Snapshot {
                            cursor: v.cursor,
                            data: $map(v.data),
                        }
                    }};
                }
                let snapshot = match query {
                    Query::Roles(_) => get!(S::Roles),
                    Query::GuildProfile(_) => get!(S::GuildProfile),
                    Query::Profile(_) => get!(S::Profile),
                    Query::Overwrites(id) => get!(|v| S::Overwrites(id, v)),
                    Query::Notifications => get!(S::Notifications),
                    Query::Sessions => get!(S::Sessions),
                };
                Ok(Data::Settings {
                    snapshot,
                    generation,
                })
            }
            Request::NewerHistory {
                seek,
                channel,
                after,
                generation,
            } => Ok(Data::NewerHistory {
                seek,
                channel,
                generation,
                page: self
                    .json(&format!(
                        "/channels/{}/messages?limit=50&after={}",
                        channel.0, after.0
                    ))
                    .await?,
            }),
            Request::AroundHistory {
                channel,
                anchor,
                seek,
                generation,
            } => Ok(Data::AroundHistory {
                channel,
                anchor,
                seek,
                generation,
                page: self
                    .json(&format!(
                        "/channels/{}/messages?limit=50&{}",
                        channel.0,
                        anchor.query()
                    ))
                    .await?,
            }),
            Request::History {
                seek,
                channel,
                before,
                generation,
            } => Ok(Data::History {
                seek,
                channel,
                page: self
                    .json(&format!(
                        "/channels/{}/messages?limit=50{}",
                        channel.0,
                        before.map_or(String::new(), |v| format!("&before={}", v.0))
                    ))
                    .await?,
                generation,
            }),
            Request::Channel {
                channel,
                generation,
            } => Ok(Data::Channel {
                snapshot: self.json(&format!("/channels/{}", channel.0)).await?,
                generation,
            }),
            Request::Members {
                guild,
                after,
                generation,
            } => Ok(Data::Members {
                guild,
                after,
                page: self
                    .json(&format!(
                        "/guilds/{}/members?limit=50{}",
                        guild.0,
                        after.map_or(String::new(), |v| format!("&after={}", v.0))
                    ))
                    .await?,
                generation,
            }),
            Request::Invites {
                guild,
                after,
                generation,
            } => Ok(Data::Invites {
                guild,
                after,
                page: self
                    .json(&format!(
                        "/guilds/{}/invites?limit=50{}",
                        guild.0,
                        after.map_or(String::new(), |v| format!("&after={}", v.0))
                    ))
                    .await?,
                generation,
            }),
            Request::Bans {
                guild,
                after,
                generation,
            } => Ok(Data::Bans {
                guild,
                after,
                page: self
                    .json(&format!(
                        "/guilds/{}/bans?limit=50{}",
                        guild.0,
                        after.map_or(String::new(), |v| format!("&after={}", v.0))
                    ))
                    .await?,
                generation,
            }),
            Request::Users { ids, generation } => Ok(Data::Users {
                snapshot: self
                    .json(&format!(
                        "/users?ids={}",
                        ids.iter()
                            .map(|v| v.0.to_string())
                            .collect::<Vec<_>>()
                            .join(",")
                    ))
                    .await?,
                generation,
            }),
            Request::Message {
                channel,
                message,
                generation,
            } => Ok(Data::Message {
                snapshot: self
                    .json(&format!("/channels/{}/messages/{}", channel.0, message.0))
                    .await?,
                generation,
            }),
            Request::DiscardUpload(id) => {
                self.execute_with_error(
                    "DELETE",
                    &format!("/api/v1/uploads/{}", id.0),
                    Body::Empty,
                    vec![],
                    30_000,
                    "Could not discard upload",
                )
                .await?;
                Ok(Data::Error(String::new()))
            }
            _ => Err("Invalid HTTP operation".into()),
        }
    }

    pub async fn authenticate(
        &self,
        username: String,
        password: String,
        register: bool,
        display: String,
    ) -> Result<AuthResponse, String> {
        let error = "Cannot reach that server. Check the address and connection.";
        if register {
            self.post_api_with_error(
                "/api/v1/auth/register",
                &RegisterRequest {
                    username,
                    password,
                    display_name: if display.is_empty() {
                        None
                    } else {
                        Some(display)
                    },
                },
                error,
            )
            .await
        } else {
            self.post_api_with_error(
                "/api/v1/auth/login",
                &LoginRequest { username, password },
                error,
            )
            .await
        }
    }
    pub async fn meta(&self) -> Result<MetaResponse, String> {
        decode_json(
            self.execute_with_error(
                "GET",
                "/api/meta",
                Body::Empty,
                vec![],
                30_000,
                "Could not read server capabilities",
            )
            .await?,
        )
    }
    pub async fn google(&self, id_token: String) -> Result<GoogleAuthResponse, String> {
        self.post_api_with_error(
            "/api/auth/google",
            &GoogleAuthRequest { id_token },
            "Could not reach your server. Please try again.",
        )
        .await
    }
    pub async fn google_complete(
        &self,
        ticket: String,
        username: String,
    ) -> Result<AuthResponse, String> {
        self.post_api_with_error(
            "/api/auth/google/complete",
            &GoogleCompleteRequest { ticket, username },
            "Could not finish signup. Please try again.",
        )
        .await
    }
    pub async fn google_challenge(
        &self,
        operation: AccountOperation,
    ) -> Result<GoogleChallengeResponse, String> {
        self.post_api(
            "/api/v1/auth/google/challenge",
            &GoogleChallengeRequest { operation },
        )
        .await
    }
    pub async fn reauthenticate(&self, request: &ReauthRequest) -> Result<ReauthResponse, String> {
        self.post_api("/api/v1/auth/reauth", request).await
    }
    pub async fn delete_account(&self, proof: &str) -> Result<(), String> {
        let response = self
            .execute(
                "POST",
                "/api/v1/auth/account/delete",
                Body::Bytes(b"{}".to_vec()),
                vec![
                    ("Content-Type".into(), "application/json".into()),
                    ("x-sticktalk-reauth".into(), proof.into()),
                ],
                30_000,
            )
            .await?;
        if response.status == 204 {
            Ok(())
        } else {
            decode_json::<serde_json::Value>(response).map(|_| ())
        }
    }
    /// Legacy callers can read methods; mutations should use account_authorized
    /// and persist/reconnect with a returned rotated session.
    pub async fn account(&self, action: AccountAction) -> Result<AuthMethodsResponse, String> {
        Ok(self.account_authorized(action, None).await?.methods)
    }
    pub async fn account_authorized(
        &self,
        action: AccountAction,
        proof: Option<&str>,
    ) -> Result<AccountUpdateResponse, String> {
        if matches!(action, AccountAction::Read) {
            return Ok(AccountUpdateResponse {
                methods: decode_json(
                    self.execute("GET", "/api/auth/methods", Body::Empty, vec![], 30_000)
                        .await?,
                )?,
                session: None,
            });
        }
        let (path, body) = match action {
            AccountAction::Link(id_token) => (
                "/api/auth/google/link",
                serde_json::to_vec(&GoogleAuthRequest { id_token }),
            ),
            AccountAction::Unlink => (
                "/api/auth/google/unlink",
                serde_json::to_vec(&GoogleUnlinkRequest {}),
            ),
            AccountAction::Password { current, new } => (
                "/api/auth/password",
                serde_json::to_vec(&SetPasswordRequest {
                    current_password: current,
                    new_password: new,
                }),
            ),
            AccountAction::Read => unreachable!(),
        };
        let mut headers = vec![("Content-Type".into(), "application/json".into())];
        if let Some(proof) = proof {
            headers.push(("x-sticktalk-reauth".into(), proof.into()));
        }
        decode_json(
            self.execute(
                "POST",
                path,
                Body::Bytes(body.map_err(|_| "Could not encode request")?),
                headers,
                30_000,
            )
            .await?,
        )
    }
    pub async fn logout(&self) {
        let _ = self
            .execute("POST", "/api/v1/auth/logout", Body::Empty, vec![], 30_000)
            .await;
    }
}
/// Shared bounded-body collector for streaming native and fetch adapters.
pub async fn limited(mut chunks: ByteStream, max: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk?;
        if chunk.len() > max.saturating_sub(bytes.len()) {
            return Err("Server response exceeds limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

impl<A: HttpTransport, R: MaybeSend + 'static, H: MaybeSend + 'static>
    crate::protocol::RestService<R, H> for Http<A>
{
    fn request(&self, request: Request<R>) -> Task<'static, Result<Data<H>, String>> {
        let http = self.clone();
        Box::pin(async move { http.request(request).await })
    }
}

#[cfg(test)]
mod qa_read_retry {
    use super::*;
    fn limited(delay: u32) -> HttpResponse {
        HttpResponse {
            status: 429,
            bytes: serde_json::to_vec(&ApiError {
                code: ErrorCode::RateLimited,
                message: "RateLimited".into(),
                retry_after_ms: Some(delay),
                field: None,
            })
            .unwrap(),
        }
    }
    #[test]
    fn reads_honor_backoff_but_retries_and_waiting_are_bounded() {
        let mut read = ReadRetry::new(true);
        for _ in 0..3 {
            assert_eq!(read.delay(&limited(500)), Some(500));
        }
        assert_eq!(read.delay(&limited(500)), None);
        let mut read = ReadRetry::new(true);
        assert_eq!(read.delay(&limited(5000)), Some(5000));
        assert_eq!(read.delay(&limited(5000)), Some(5000));
        assert_eq!(read.delay(&limited(1)), None);
        assert_eq!(ReadRetry::new(true).delay(&limited(60_000)), None);
        assert_eq!(ReadRetry::new(true).delay(&limited(0)), Some(10));
        assert_eq!(
            ReadRetry::new(false).delay(&limited(500)),
            None,
            "never replay a mutation"
        );
        assert_eq!(
            ReadRetry::new(true).delay(&HttpResponse {
                status: 500,
                bytes: limited(1).bytes
            }),
            None
        );
        assert_eq!(
            ReadRetry::new(true).delay(&HttpResponse {
                status: 429,
                bytes: b"invalid".to_vec()
            }),
            None
        );
    }
}
