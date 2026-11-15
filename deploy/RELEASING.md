# StickTalk Windows releases

## What friends get

One `StickTalk-Setup-<version>.exe`. Setup never asks for administrator access:
a fresh install goes in `%LOCALAPPDATA%\Programs\StickTalk` (registered existing
installations are upgraded in place), adds a Start menu shortcut,
offers an optional desktop shortcut, and registers `sticktalk://` in HKCU.
The first run shows a server address, sign-in/register, and Google when enabled
by that server **and included in the build configuration below**. Google uses a
temporary **loopback callback**, not a custom URI.
Uninstall keeps local settings by default; uncheck the keep-data box to remove
the app-owned `%LOCALAPPDATA%\StickTalk` directory. Silent uninstall supports
`/PURGE=1`; saved Windows credentials are separate (remove StickTalk entries
in Credential Manager if retiring the account/device). Setup never starts the
app or enables startup automatically.

## Releases without a server or signing key (GitHub Releases)

Until there is a long-lived server to pin and a signing key, publish
**manual-update** builds: no compiled-in server (first run asks for the
address of whichever StickTalk server someone hosts, see `deploy/SELF_HOST.md`)
and no updater. People update by installing the newer `StickTalk-Setup-*.exe`
over the old one; settings and sign-ins are kept.

```powershell
# Fresh shell: no STICKTALK_UPDATE_* variables set. Bump the Cargo version first.
./deploy/windows/build-installer.ps1 -Version 0.1.1 -ManualUpdates
./tools/release/test-installer.ps1 -Installer "$env:USERPROFILE/.cargo-target/sticktalk-release-eng-out/StickTalk-Setup-0.1.1.exe" -Client "$env:CARGO_TARGET_DIR/release/sticktalk.exe"
git tag -a v0.1.1 -m 'StickTalk 0.1.1' && git push origin v0.1.1
gh release create v0.1.1 --title 'StickTalk 0.1.1' --notes-file notes.md `
  "$env:USERPROFILE/.cargo-target/sticktalk-release-eng-out/StickTalk-Setup-0.1.1.exe" SHA256SUMS.txt
```

Installed manual-update builds never learn about later releases on their own.
When you do adopt signed updates (sections 2-5), friends install that first
signed build by hand once; after that it updates itself.

## 1. One-time prerequisites

Use normal, non-administrator PowerShell. Rust stable and the existing MSVC/
Windows SDK build tools must already be installed. No paid packaging tools are
needed. `deploy/windows/build-installer.ps1` downloads the official **portable
NSIS 3.13 ZIP**, verifies its pinned SHA-256, and extracts it under the target
folder. It does not run the NSIS setup program, use winget, or request elevation.
You can supply `-MakeNsis C:\path\to\makensis.exe` instead.

Sources: [NSIS portable download](https://sourceforge.net/projects/nsis/files/NSIS%203/3.13/),
[NSIS command-line reference](https://nsis.sourceforge.io/Docs/Chapter3.html).
The app embeds a multi-resolution icon and a per-monitor-v2 DPI manifest.
The build script discovers the installed x64 MSVC redistributable DLLs and
packages them **app-local** beside the EXE, avoiding an elevated VC runtime
installer on a friend's PC. Pass `-RuntimeDir` to select them explicitly.
Refresh these DLLs with releases; app-local copies are serviced by your updates,
not automatically by Windows. See [Microsoft's app-local deployment guide](https://learn.microsoft.com/en-us/cpp/windows/walkthrough-deploying-a-visual-cpp-application-to-an-application-local-folder).

```powershell
$env:CARGO_TARGET_DIR = "$env:USERPROFILE/.cargo-target/sticktalk-release-eng"
$env:CARGO_INCREMENTAL = '0'
```

Use just this one build tree. Stay below 10 GB. Delete the tree after validation;
keep final installers in the **separate** `sticktalk-release-eng-out` directory.

## 2. Generate the real signing key (once)

The private key belongs only on your release PC, **outside the repository,
OneDrive and the server**. Back it up encrypted/offline. Losing it means existing
clients cannot trust new releases. Never send it in chat or commit it.

```powershell
$secrets = "$env:USERPROFILE/.sticktalk-secrets"
New-Item -ItemType Directory -Force $secrets | Out-Null
icacls $secrets /inheritance:r /grant:r "$($env:USERNAME):(OI)(CI)F"
cargo run --release -p sticktalk-desktop-platform --bin sticktalk-package -- keygen "$secrets/update-signing.pk8"
```

Save the printed **public key** (base64, 32 bytes) in your release configuration.
Put that same public key in `STICKTALK_UPDATE_PUBLIC_KEY` on the server and in
the build shell. The private key never goes on the server. The command refuses
to overwrite an existing key or create one inside this checkout.
Key rotation requires a release already trusted by old clients; simply changing
the server key will not rotate installed clients.

## 3. Build an installer

First bump `[workspace.package].version` in `Cargo.toml` and commit it. Do not
reuse a version. Set public values for **your actual HTTPS service**:

```powershell
$env:STICKTALK_UPDATE_PUBLIC_KEY = '<your public key>'
$env:STICKTALK_DEFAULT_SERVER = 'https://chat.your-domain.example'
$env:STICKTALK_UPDATE_ORIGIN = 'https://chat.your-domain.example'
./deploy/windows/build-installer.ps1 -Version 0.1.1
```

For Google sign-in, also set `STICKTALK_GOOGLE_CLIENT_ID` and
`STICKTALK_GOOGLE_CLIENT_SECRET` in this shell to the Desktop OAuth credentials
configured for the server (see `docs/GOOGLE_LOGIN.md`). Never put the secret in
git or scripts. Both must be present, and the client ID must match the server's
capability response. Without them, password login/register still work and Google
is hidden; the engineering test build deliberately contains no real credentials.
The local mock OAuth/PKCE/state tests require no real Google account or browser.

These are compiled in. The update origin stays pinned even when someone changes
chat servers. The public key is compiled into **both** client and maintenance
helper; missing/invalid keys disable updates rather than trusting a server key.
The script rejects a version that differs from Cargo. `-SkipBuild` is only for
repackaging already-validated binaries; it does not embed changed environment
values. `-TestFixture` is solely for local test installers, never publishing.

Output: `%USERPROFILE%\.cargo-target\sticktalk-release-eng-out\StickTalk-Setup-0.1.1.exe`.
The test installer produced during engineering embeds an **ephemeral test public
key**, not your production key: rebuild before distributing to friends.

Ed25519 protects updates; it is **not Authenticode**. No Windows code-signing
certificate is bundled. Unsigned downloads can show SmartScreen reputation
warnings. If you obtain an Authenticode certificate, sign the final installer
**before** producing its update manifest (signing changes the file hash).
Neither installer nor app requests elevation.

## 4. Test before publishing

Close StickTalk. The smoke test refuses to overwrite existing registrations.
It installs into a temporary prefix, checks shortcuts, AUMIDs, registry metadata,
protocol quoting and executable hashes, then uninstalls.

```powershell
./tools/release/test-installer.ps1 `
  -Installer "$env:USERPROFILE/.cargo-target/sticktalk-release-eng-out/StickTalk-Setup-0.1.1.exe" `
  -Client "$env:CARGO_TARGET_DIR/release/sticktalk.exe"
cargo test --release -p sticktalk -p sticktalk-desktop-platform
cargo clippy --release -p sticktalk -p sticktalk-desktop-platform --all-targets -- -D warnings
cargo fmt --all -- --check
```

`deploy/windows/test-update.ps1` additionally exercises signed **0.0.1 → 0.0.2**
fixture installers, HTTP Range resume, apply/healthy restart, a bad-startup rollback,
interrupted-setup recovery, keep/purge data behavior, and ordinary-launch application
of a deferred update. Pass `-RealClient` to use the actual client for startup health
checks (the fixture install metadata versions differ from its compiled 0.1.0). It requires a **test-key build** and the temporary
private key used for that build. It binds only to `127.0.0.1`, never opens a browser,
and launches no fixture windows. Do not use a production key for this test.
See `deploy/windows/VALIDATION.md` for the actual run and remaining limits.

## 5. Sign and publish

Generate a separate random publishing token (32+ characters), set
`STICKTALK_RELEASE_TOKEN` in your release shell and on the server, and keep it
out of scripts/git. This token authorizes uploading; it cannot forge signatures.
Server hosting is in `deploy/SELF_HOST.md` (configuration reference: `deploy/ORACLE.md`).

```powershell
./tools/release/release.ps1 -Version 0.1.1 -Server 'https://chat.your-domain.example'
```

The script builds, signs with `%USERPROFILE%/.sticktalk-secrets/update-signing.pk8`,
and sends the installer plus manifest to `/api/update/publish` over HTTPS.
Alternatively create the signed envelope without publishing:

```powershell
& "$env:CARGO_TARGET_DIR/release/sticktalk-package.exe" sign `
  "$secrets/update-signing.pk8" `
  "$env:USERPROFILE/.cargo-target/sticktalk-release-eng-out/StickTalk-Setup-0.1.1.exe" `
  0.1.1 'https://chat.your-domain.example' `
  "$env:USERPROFILE/.cargo-target/sticktalk-release-eng-out/StickTalk-0.1.1-manifest.json"
```

Keep the exact signed envelope and installer together. The signature covers the
original manifest bytes, including version, target, size, SHA-256 and same-origin
content-addressed URL. Never edit the JSON after signing. Publish only a newer
version, and test using a previously installed client.

## Update behavior / recovery

- One immediate startup check (including the login screen), then one six-hour
  Tokio timer on the existing runtime. No busy-loop or idle background worker.
- Download streams/resumes in `%LOCALAPPDATA%\StickTalk\Updates`; the signature,
  target/version, size and hash are checked before any executable is run, and
  checked again after the old app exits. No redirects or cross-origin artifacts.
- A small **“Update ready — restart”** action asks for confirmation. No forced
  restart or installer UI. If you quit instead, the verified update is applied
  before the window on the next ordinary interactive launch. Background/autostart
  and deep-link launches are never delayed; use the pill or a normal restart.
- The copied helper waits for the old process, backs up owned installation files,
  runs per-user NSIS silently in place, then starts the new client. The client
  acknowledges after actual UI initialization; it must also stay alive for the
  15-second startup grace period. Failure restores files and the old uninstall
  version, then reopens the old app. Settings/data are never migrated/deleted by
  this process; future incompatible data migrations need their own rollback plan.
- Interrupted setup leaves `Updates/rollback/version.txt`. The next manual launch
  copies/runs the helper outside the install directory and restores the backup
  before opening a window. Do not delete this backup until recovery finishes.
- A failed release is quarantined in `failed-update.json` and not offered again.
  Publish a new version. For a deliberate retry after fixing a local problem,
  close StickTalk and remove only that marker. `last-update.log` explains results.
- Rust panic logs include source/backtrace **not panic payloads/account data**.
  An abnormal-exit marker also detects native crashes/forced termination. Next
  startup offers “Open logs folder.” Logs stay local, bounded to current/previous
  panic reports; no external crash service or automatic upload exists.
- Window geometry/maximized state is saved on native events, clamped to an
  available monitor on restore. Startup-in-tray and autostart are opt-in in
  Settings → Advanced. Second launch and protocol links open the existing window,
  including while signed out. Invite links are retained until login.

A failing installer that deletes the installation directory entirely or a disk
failure may require rerunning a previously downloaded installer. Keep your last
known-good installer. Startup health checks cannot diagnose crashes that happen
long after the grace period; local logs still make those visible next time.
