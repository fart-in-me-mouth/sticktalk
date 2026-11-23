//! Account security and personal data in settings (docs/features/authsec.md).
//! Rust renders only these empty roots and buttons; `public/authsec.js` draws
//! the sections into them (passkeys, email, QR scanning, data export) and runs
//! the account switcher. Removing the feature removes these hooks.
use super::*;

/// Below the password form on Password & Security.
pub(super) fn security_root() -> AnyView {
    view! {<div class="settings-divider"></div><div class="authsec-root" data-authsec-root="security"></div>}.into_any()
}
/// The Your Data page.
pub(super) fn data_page() -> AnyView {
    view! {<h2>"Request My Data"</h2><div class="authsec-root mt-4" data-authsec-root="data"></div>}
        .into_any()
}
/// In your own profile popout, beside Edit Profile.
pub(super) fn switch_accounts() -> AnyView {
    view! {<button type="button" class="button secondary" data-authsec="switcher">"Switch Accounts"</button>}.into_any()
}
