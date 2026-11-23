# Account security and personal data (`authsec`)

Passkey sign-in, QR code sign-in, optional email password reset, the account
switcher and "Request my data". All free for every user.

There is **no multi-factor authentication** in StickTalk (owner decision,
2026-11-23): no authenticator codes, no recovery codes, and sign-in never asks
for a second step. Passkeys are only an optional passwordless way to sign in.
"Re-authentication" below always means the existing password or Google
re-authentication (`POST /api/v1/auth/reauth`, or the Google redirect with
`?operation=`), which yields a five-minute proof bound to one session and one
operation.

Browser test: `node crates/web/tests/authsec.mjs` (needs `CARGO_TARGET_DIR` with `web-dist`,
`STICKTALK_SERVER_EXE`; screenshots go to `STICKTALK_AUTHSEC_SHOTS`).

Server code: `crates/server/src/authsec/` (one file per feature). Protocol:
`crates/proto/src/authsec.rs`. Migration: `202611230601_authsec.sql`. Web:
`crates/web/public/authsec-login.js` (sign-in screen), `public/authsec.js`
(settings, dialogs), `src/live/ui/authsec.rs` and `src/live/authsec.rs` (hooks).

## Configuration

| Flag / env | Meaning |
|---|---|
| `--webauthn-rp-id` / `STICKTALK_WEBAUTHN_RP_ID` | Passkey relying-party ID. Default: the host of `--public-url`. It must equal that host or be a parent domain of it (`chat.example.com` may use `example.com`); anything else is a startup error. |
| `--smtp-host` / `STICKTALK_SMTP_HOST` | Enables email reset. Unset: email routes answer `feature_disabled`; the administrator reset code stays the only reset and the UI says so. |
| `--smtp-port` | Default 587 (`starttls`) or 465 (`tls`). |
| `--smtp-security` | `starttls` (default), `tls`, or `none`. `none` is refused unless the host is loopback. |
| `--smtp-username`, `--smtp-password` | Both or neither. The password is redacted from logs and `Debug`. |
| `--smtp-from` | Required with a host, e.g. `StickTalk <no-reply@example.com>`. |

Passkeys turn themselves off when the public URL cannot host WebAuthn (plain
HTTP other than `localhost`, or an IP address). `GET /api/v1/auth/options`
(`AuthOptions {passkeys, qr_login, email_reset}`) tells the sign-in screen.

## Passkeys (passwordless sign-in only)

Verification is `webauthn-rs` 0.6 (audited, pure Rust): origin must be the
public URL, RP ID hash, one-use challenge (five minutes, in memory, at most
4,096 pending), user verification **required**, signature, and the signature
counter (a counter that does not advance past the stored one is refused as a
possible clone). Credentials are discoverable (resident), so sign-in needs no
username. Each user has a random 16-byte WebAuthn user handle, never derived
from the user ID. Public keys are stored as webauthn-rs `Passkey` JSON (they
are not secrets).

| Route | Auth | Notes |
|---|---|---|
| `GET /api/v1/users/@me/passkeys` | session | `PasskeySummary[]`: id, name, created, last used, synced. |
| `POST /api/v1/users/@me/passkeys/register/start` `{name}` | session + reauth `passkeys` | Returns `PasskeyChallenge {challenge_id, public_key (JSON text), expires_at}`. The ceremony is bound to this session. |
| `POST /api/v1/users/@me/passkeys/register/finish` `{challenge_id, credential}` | same session | At most 10 passkeys per account; a credential registered anywhere is refused. |
| `PATCH /api/v1/users/@me/passkeys/{id}` `{name}` | session | 1 to 64 characters, no control characters. Someone else's ID is `not_found`. |
| `DELETE /api/v1/users/@me/passkeys/{id}` | session + reauth `passkeys` | |
| `POST /api/v1/auth/passkey/start` | none | Discoverable request options (no credential list). |
| `POST /api/v1/auth/passkey/finish` `{challenge_id, credential}` | none | `AuthResponse` with a brand-new session (normal 30 days, five per account). |
| `POST /api/v1/browser/session/passkey` | same origin | Same, as HttpOnly cookies (`BrowserSession`). |

Limits: 20/min (burst 10) per IP on start/finish, 10/min (burst 5) per account
on sign-in, registration and removal.

**Removing the feature** (owner may decide to): delete `authsec/passkey.rs`, its
`mod`/`.merge(passkey::routes())`/`passkeys` lines in `authsec/mod.rs`, the
passkey section in `public/authsec.js` and the button in `public/authsec-login.js`,
and add a migration dropping `passkeys` and `passkey_users`. No other code
depends on it.

## QR code sign-in

A signed-out browser opens `GET /api/v1/auth/qr/socket` (WebSocket, exact
`Origin` required, 30/min burst 10 per IP in a bucket of its own, at most 256 at once). The server
pushes JSON text frames (`QrSocketMessage`):

| `type` | Meaning |
|---|---|
| `code` | `{url, size, modules, expires_at}`: `url` is `<public>/#qr=<token>`; `modules` is the QR matrix (`'1'` dark), drawn by the page. |
| `scanned` | A signed-in device opened the code and sees the confirmation. |
| `rejected` | It declined; a new `code` follows. |
| `approved` | `{ticket}`: redeem within 60 s for this browser's own new session. |
| `expired` | No more codes on this socket (after 10 codes, 20 minutes). |

Each code lives 2 minutes and is replaced on a timer armed for its own
expiry, then pushed: the page never polls. The page never sends anything.

| Route | Auth | Notes |
|---|---|---|
| `POST /api/v1/auth/qr/inspect` `{token}` | session | `QrLoginRequest {device, ip, same_network, created_at, expires_at}`. The first session to open a code owns it; any other gets `not_found`. |
| `POST /api/v1/auth/qr/approve` `{token}` | the session that inspected it | Pushes the ticket to the waiting socket only; the code is spent. |
| `POST /api/v1/auth/qr/reject` `{token}` | same | The socket gets `rejected` and a new code. |
| `POST /api/v1/browser/session/qr` `{ticket}` | same origin | One use, 60 s; cookie session. Native clients: `POST /api/v1/auth/qr/redeem` returns `AuthResponse`. |

Limits: 10/min burst 5 per account and per IP on inspect/approve/reject; the
login IP bucket on redeem. Nothing is stored in the database.

Phishing defences, as far as the design allows: the token alone grants nothing;
approval requires a signed-in device to open it, see the requesting device
(browser and OS from its User-Agent), its IP address and whether it is on the
same network, and press "Yes, Sign In" under an explicit warning; the session
ticket only travels down the requesting socket; codes are short-lived and
single-use; the in-app scanner refuses codes for any other origin. Closing the
confirmation without answering declines.

**Location:** the server ships no GeoIP database, so "approximate location" is
shown as the IP address plus "same network / not on the same network". City
level location would need an offline GeoIP database (an owner decision).

## Email and password reset

| Route | Auth | Notes |
|---|---|---|
| `GET /api/v1/users/@me/email` | session | `EmailStatus {smtp_enabled, email, verified, pending}`. |
| `PUT /api/v1/users/@me/email` `{email}` | session + reauth `email` | Sends a verification link `<public>/#verify-email=<token>` (24 h). 5/hour burst 3 per account. |
| `DELETE /api/v1/users/@me/email` | session + reauth `email` | |
| `POST /api/v1/auth/email/verify` `{token}` | none | Confirms the address still pending; voids older reset links. |
| `POST /api/v1/auth/email-reset/request` `{login}` | none | Username or address. Always `202` with an empty body, sent before any lookup; mail is sent in the background. Per IP (login bucket) and 3/hour per target string. |
| `POST /api/v1/auth/email-reset/complete` `{token, new_password}` | none | `<public>/#reset=<token>`, 30 minutes, single use, only while that address is still the account's verified one. Sets the password and signs out **every** session (sockets and calls closed). Login IP bucket. |

Addresses are not unique (a reset by address mails every account, at most 5,
that verified it), so adding one never reveals another account. Only SHA-256
of each link secret is stored; one outstanding link per account and kind.
Links carry the secret in the URL fragment, which never reaches the server or
proxies; the page removes it from the address bar at once. At most 64 messages
are in flight; beyond that new ones are dropped (and logged without content).

## Account switcher (browser and Windows app)

Up to five accounts stay signed in on one browser. Each keeps its **own
independent session** (the normal 30-day session, counted toward that account's
five-session limit). The active one is the existing `__Host-sticktalk_session`
cookie; the others wait in one `__Host-sticktalk_accounts` cookie (tokens joined
by `.`). Both are `HttpOnly; Secure; SameSite=Strict`, host-only, so page
script, and therefore an XSS bug, can never read any token. The Windows app is
WebView2 showing this same web app; the same cookies live in its WebView2
profile under the app's data folder.

| Route | Notes |
|---|---|
| `GET /api/v1/browser/accounts` | `SignedInAccounts {accounts: [{user, active}], max}`. Drops dead tokens; revokes duplicate sessions of one account. Works signed out. |
| `POST /api/v1/browser/accounts/add` | Moves the active session into the waiting cookie and signs the browser out to show the sign-in screen (which lists the waiting accounts). `limit_exceeded` at five. |
| `POST /api/v1/browser/accounts/switch` `{user_id}` | Only to an account waiting in this browser. Swaps cookies, issues that account's CSRF cookie, clears any reauth proof. |
| `POST /api/v1/browser/accounts/logout` `{user_id}` | Revokes only that account's session (sockets and calls closed). |

All POSTs require the exact origin, and the CSRF header whenever a session
cookie is present. Switching is limited to 30/min (burst 10) per IP in its own bucket. Trade-off: anyone who can use this browser profile can switch
between its accounts without a password; that is the feature. "Sign Out" per
account, and normal Log Out, remove an account from the browser.

## Personal data export

| Route | Auth | Notes |
|---|---|---|
| `GET /api/v1/users/@me/data-export` | session | `DataExportStatus {export, next_request_at}`. Also opts this stream into `DataExportUpdated` events. |
| `POST /api/v1/users/@me/data-export` | session + reauth `data_export` | One job per account at a time (`conflict`); then one request per 24 h (`rate_limited` with `retry_after_ms`); 64 queued server-wide. A refused request does not spend the proof. |
| `GET /api/v1/users/@me/data-export/{id}/download` | the owner only | `application/zip`, streamed; another user's ID is `not_found`. Counted as a file transfer (per-IP and per-user download limits). |
| `DELETE /api/v1/users/@me/data-export/{id}` | the owner | Deletes the archive early. |

WebSocket event (appended variant): `Event::DataExportUpdated(DataExport)`,
sent only to streams that fetched the status route, so older clients never see it.

One worker for the whole server builds one archive at a time in a blocking
thread, reading the database a page (500 messages) at a time and streaming into
`<data>/exports/<random>.part`, renamed when complete. Archives expire after
7 days on a timer armed for the next expiry; interrupted jobs restart after a
reboot; files without a live row (erased accounts) are deleted when the worker
next wakes. Account erasure deletes all of this module's rows.

Archive (format `sticktalk-data-export`, version 1; times are Unix ms; IDs are strings):

| File | Contents |
|---|---|
| `README.txt`, `index.html` | Human-readable index (all user text HTML-escaped, no script). |
| `account.json` | Account, profile, sign-in methods (password set, Google linked, passkey names), email, active sessions (times only). |
| `settings.json` | The account's rows from notification settings, nicknames, member roles, server folders, GIF favorites, nav pins/favorites/rail, saved messages, read states, reactions, session devices. New per-account tables should be added to `SETTINGS_TABLES` in `export.rs`. Columns named `*_hash`, `secret` or `token` are never exported. |
| `servers.json`, `relationships.json` | Servers with role and join time; friends, requests and blocks. |
| `messages/index.json`, `messages/<channel>.json` | Every non-deleted message the user wrote, per conversation, with attachment metadata. |
| `attachments/index.json`, `attachments/<id>-<safe name>` | Files attached to those messages, copied up to 2 GiB per archive, then listed with their URL only. |

## Decisions the owner may want to revisit

- No MFA anywhere (as instructed); passkeys are passwordless-only and removable.
- No GeoIP: location is the IP address and same-network flag.
- Email addresses are not unique across accounts (prevents account discovery).
- Export cooldown 24 h, retention 7 days, attachment copy cap 2 GiB.
- Five accounts per browser, matching the five-session limit per account.
