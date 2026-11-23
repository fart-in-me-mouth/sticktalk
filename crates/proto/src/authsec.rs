//! Account security and personal data: passkey sign-in, QR code sign-in, email
//! password reset, the browser account switcher and personal data export.
//! See `docs/features/authsec.md` for routes, limits and permissions.
use crate::*;
use serde::{Deserialize, Serialize};

/// `GET /api/v1/auth/options`: which optional sign-in features this server offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthOptions {
    pub passkeys: bool,
    pub qr_login: bool,
    /// SMTP is configured: users can add an email address and reset by email.
    pub email_reset: bool,
}

// ----- Passkeys -------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasskeySummary {
    /// Opaque decimal identifier of this registration (not the credential ID).
    pub id: String,
    pub name: String,
    pub created_at: UnixMillis,
    pub last_used_at: Option<UnixMillis>,
    /// The authenticator reports that this passkey is synced (backed up).
    pub backed_up: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasskeyRegisterStart {
    pub name: String,
}

/// A WebAuthn ceremony in progress. `public_key` is the JSON text of the
/// `CredentialCreationOptions` / `CredentialRequestOptions` for the browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasskeyChallenge {
    pub challenge_id: String,
    pub public_key: String,
    pub expires_at: UnixMillis,
}

/// `credential` is the JSON text of the browser's `PublicKeyCredential`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasskeyFinish {
    pub challenge_id: String,
    pub credential: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasskeyRename {
    pub name: String,
}

// ----- QR code sign-in ------------------------------------------------------

/// Text frames on the signed-out QR socket (`GET /api/v1/auth/qr/socket`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QrSocketMessage {
    /// A fresh code. `modules` is the QR matrix, row-major, one '1'/'0' per module.
    Code {
        url: String,
        size: u32,
        modules: String,
        expires_at: UnixMillis,
    },
    /// Someone signed in opened the code and sees the confirmation screen.
    Scanned,
    /// They declined. A new code follows.
    Rejected,
    /// They confirmed. Redeem `ticket` within a minute for this browser's own session.
    Approved { ticket: String },
    /// No more codes on this socket; reload to start again.
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QrTokenRequest {
    pub token: String,
}

/// What the confirming device is shown about the device asking to sign in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QrLoginRequest {
    /// For example "Chrome on Windows".
    pub device: String,
    pub ip: String,
    /// The asking device reached the server from the same public address.
    pub same_network: bool,
    pub created_at: UnixMillis,
    pub expires_at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QrRedeemRequest {
    pub ticket: String,
}

// ----- Email and password reset ---------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailStatus {
    pub smtp_enabled: bool,
    pub email: Option<String>,
    pub verified: bool,
    /// An address waiting for its verification link to be opened.
    pub pending: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetEmailRequest {
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailTokenRequest {
    pub token: String,
}

/// `login` is a username or an email address. The answer never says which, or
/// whether an account exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailResetRequest {
    pub login: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailResetComplete {
    pub token: String,
    pub new_password: String,
}

// ----- Account switcher (browser) -------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedInAccount {
    pub user: UserSummary,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedInAccounts {
    pub accounts: Vec<SignedInAccount>,
    /// Most accounts one browser keeps signed in at once.
    pub max: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountChoice {
    pub user_id: UserId,
}

// ----- Personal data export -------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataExportState {
    Queued,
    Running,
    Ready,
    Failed,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataExport {
    pub id: String,
    pub state: DataExportState,
    pub requested_at: UnixMillis,
    pub ready_at: Option<UnixMillis>,
    /// The archive is deleted at this time.
    pub expires_at: Option<UnixMillis>,
    pub byte_size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataExportStatus {
    pub export: Option<DataExport>,
    /// A new request is refused before this time.
    pub next_request_at: Option<UnixMillis>,
}
