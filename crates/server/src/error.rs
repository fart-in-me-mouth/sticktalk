use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use sticktalk_proto::{ApiError, ErrorCode};
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug)]
pub struct Error(pub ApiError);
impl Error {
    pub fn new(code: ErrorCode) -> Self {
        Self(ApiError {
            code,
            message: format!("{code:?}"),
            field: None,
            retry_after_ms: None,
        })
    }
    pub fn rate(ms: u32) -> Self {
        let mut e = Self::new(ErrorCode::RateLimited);
        e.0.retry_after_ms = Some(ms);
        e
    }
}
impl From<sqlx::Error> for Error {
    fn from(_: sqlx::Error) -> Self {
        Self::new(ErrorCode::Internal)
    }
}
impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::new(ErrorCode::Unavailable)
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        use ErrorCode::*;
        let status = match self.0.code {
            InvalidRequest | InvalidSequence | InvalidAttachment => 400,
            InvalidCredentials | Unauthenticated | SessionExpired | InvalidSignupTicket => 401,
            Forbidden | Blocked | Banned | OwnerRequired | HierarchyViolation => 403,
            NotFound | InvalidInvite | FeatureDisabled => 404,
            Conflict
            | UsernameTaken
            | AlreadyExists
            | InviteExhausted
            | LimitExceeded
            | ResyncRequired
            | IdentityAlreadyLinked
            | LastLoginMethod => 409,
            InviteExpired => 410,
            PayloadTooLarge => 413,
            UnsupportedMediaType => 415,
            VersionMismatch => 426,
            RateLimited => 429,
            UnsupportedFeature => 501,
            Unavailable => 503,
            Internal => 500,
        };
        let retry = self.0.retry_after_ms;
        let mut r = (
            StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(self.0),
        )
            .into_response();
        r.headers_mut()
            .insert("cache-control", "no-store".parse().expect("static header"));
        if let Some(ms) = retry {
            if let Ok(v) = ms.div_ceil(1000).to_string().parse() {
                r.headers_mut().insert("retry-after", v);
            }
        }
        r
    }
}
pub(crate) fn require(ok: bool, code: ErrorCode) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::new(code))
    }
}
