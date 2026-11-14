# Windows release validation — 2026-11-13

Branch: `t3/windows-release-engineering`. All commands ran without elevation.
No public/LAN listeners, real OAuth browser, external crash upload, or git push.
Screenshots use the offscreen tiny-skia renderer, never screen capture.

## Build configuration and limitations

The delivered `0.1.0` installer is a **test distribution**, not a production
release for friends. It embeds an ephemeral test Ed25519 public key and
`http://127.0.0.1:9` server/update defaults. No production hostname, signing key,
or Google desktop OAuth configuration was supplied. Follow `deploy/RELEASING.md`
to rebuild with the real public key and HTTPS origins; keep its private key
outside git. The installer has no Authenticode certificate, so Windows may show
an unsigned-download/SmartScreen warning. It never requests administrator access.

No clean Windows VM was available. App-local VC runtime files are packaged and
checked on this PC; a clean-machine acceptance pass remains an owner release
gate. Google is exercised against mock authorization endpoints, not a real
account. DPI rendering is checked headlessly at 100/125/150/200%; changing the
owner's actual Windows display scaling was deliberately avoided.

## Automated checks

Passed against the touched `sticktalk` and `sticktalk-desktop-platform` crates:

```powershell
cargo test --release -p sticktalk -p sticktalk-desktop-platform
cargo clippy --release -p sticktalk -p sticktalk-desktop-platform --all-targets -- -D warnings
cargo fmt --all -- --check
```

126 tests passed; one pre-existing hardware voice soak test is ignored. This
includes live client/server auth and reconnect coverage with fresh Google
nonces and explicit sensitive-operation reauthentication, native IPC/placement,
local panic-log redaction, manifest rejection, and resumed downloads. Upstream
vendored dependency warnings remain; first-party strict clippy passes.
PowerShell parser checks, Python AST checks, and `git diff --check` also pass.

## Installer and updater acceptance

`tools/release/test-installer.ps1` verifies the installed client hash, app-local
runtime and icon, optional desktop and Start menu shortcuts/AUMIDs, quoted HKCU
protocol command, HKCU uninstall entry, and autostart off. Uninstall removes the
entire installation directory, shortcuts and registrations while retaining data.

`deploy/windows/test-update.ps1 -RealClient` runs an HTTP server bound only to
`127.0.0.1`, with an ephemeral private key outside the repository. It verifies:

- Test-signed `0.0.1 → 0.0.2`, resuming an existing half-download.
- Installation of the actual release client, native UI health acknowledgement,
  and survival of the 15-second startup grace period.
- Failed `0.0.3` startup restores old files/uninstall version, reopens the old
  client, and quarantines the failed release.
- Altered signatures are rejected before an installer request.
- Interrupted installation recovers from a complete backup; an incomplete
  preparation backup never becomes eligible for recovery.
- Deferred `0.2.0` applies on an ordinary no-argument launch of the real client.
- Default keep-data and explicit purge use an isolated, compile-time test data
  directory. The owner's existing profile is never purged.

Fixture installer version labels intentionally differ from the embedded client
Cargo version (`0.1.0`); production packaging rejects that mismatch. The test
server and fixtures are not a production publication.

## Native and visual acceptance

`deploy/windows/test-desktop.ps1` is an opt-in real-window test, authorized for
this task. It installs to the normal per-user prefix, uses an isolated profile,
seeds secondary-monitor placement, and reads native window state without sending
input. It verifies second-launch/link forwarding, startup hidden in tray,
maximized reopen/restart, and removes the normal installation afterward.

Headless captures cover the main UI-spec screens at 1280×800 and 1920×1080,
including fresh login/register/Google, wrong-server/connecting, reconnect,
crash recovery, update pill/confirmation, Advanced preferences and Bluetooth
copy. Auth cards are bounded to 480 logical pixels; the registration footer fits
at 1280×800. The app embeds a nine-resolution icon and PerMonitorV2 manifest.

## Resource hygiene

Only one Cargo target directory and the release profile were used, with
`CARGO_INCREMENTAL=0`. A dependency feature-graph change briefly took the tree
to approximately 10.41 GiB; managed Cargo cleanup removed superseded Windows/DF
artifacts, and subsequent phases were staged below the limit. Final build tree
cleanup is recorded with the delivered evidence. Installers, screenshots and
validation logs remain in the separate `sticktalk-release-eng-out` directory.

## Final acceptance results

- Installer: `C:/Users/dwhee/.cargo-target/sticktalk-release-eng-out/StickTalk-Setup-0.1.0.exe`.
- Size: **53,349,273 bytes** (50.88 MiB).
- SHA-256: `D633A052CD12FDFC9A9534D6C305FFCFF930D5CAC282F50D9901BBCC31110DC8`.
- Final release build, installer smoke, all eight real-client update checks and
  native desktop acceptance passed. The normal install prefix, both shortcuts,
  protocol/AUMID/uninstall registrations were absent afterward. Existing owner
  profile data was retained. No StickTalk processes remained from acceptance.
- Native placement restored to `[2592,32,3872,832]` on the non-primary monitor;
  real app window was hidden with the tray preference, visible/maximized on
  second activation, remained maximized on another activation and after restart.
  The same original PID handled both Open and Invite second launches.
- Idle 60.008 seconds: **0.0781% of one CPU core**, **111,620,096 working-set
  bytes (106.45 MiB)**, **142,172,160 private bytes (135.59 MiB)**. This slightly
  misses the under-100-MB working-set target; further renderer/memory work is not
  part of this packaging change. No polling/animation was added to idle paths.
- Final headless run produced 156 PNGs plus ten refreshed review contact sheets;
  first-run/update/error states and scaled text were visually inspected.
- Evidence logs and JSON live beside the installer in `logs/`; screenshots in
  `screenshots/`. This folder contains no private signing key.
- Final `cargo clean --target-dir C:/Users/dwhee/.cargo-target/sticktalk-release-eng`
  passed, removing 29,544 files / 9.2 GiB. The target directory is gone and the
  separate output installer remains intact.
- Host execution policy refused deletion of the external temporary test-key
  directory. It remains at
  `%TEMP%/StickTalk-release-test-48bba2df35b1492881b6739ec5707704`
  (test key only, never a production key). No attempt was made to bypass that
  restriction. It is not in git or the deliverable folder.
