# Validation — Windows x86-64, 2026-11-12

- New crate and helper only; `crates/client` unchanged.
- **20 tests**: 5 policy/payload/assets, 1 Windows named-pipe multi-launch,
  3 headless Windows COM/XML/icon-data tests, 11 signed updater mock-server tests.
- Mock HTTP binds **127.0.0.1:0 only**. No public listeners, firewall rules,
  browser, app windows, toast display, tray icons, taskbar effects or sound playback.
- All five default examples ran successfully without desktop effects.
- `cargo test -p sticktalk-desktop-platform`
- `cargo clippy -p sticktalk-desktop-platform --all-targets -- -D warnings`
- `cargo fmt --all -- --check`
- Current source client built with
  `cargo build --release -p sticktalk --bin sticktalk --no-default-features`:
  **11,953,152 bytes** (software-rendered client). Maintenance helper: **739,328 bytes**.
- Portable NSIS **3.13** compiled the real installer: **4,177,128 bytes**.
  Artifact outside source/OneDrive:
  `C:\Users\dwhee\.cargo-target\sticktalk-desktop-platform\StickTalk-0.1.0-Setup.exe`.
- `tools/release/test-installer.ps1` passed: **silent per-user installation into
  a temporary prefix**, exact installed client hash, helper, desktop/start shortcuts,
  persisted AUMID on both shortcuts, quoted deep-link command, uninstall metadata,
  autostart off; silent uninstall removed owned binaries/shortcuts/registrations.
  Existing application cache was not touched. No application was launched.
- NSIS setup via `winget --scope user` had no applicable installer; used official
  portable ZIP instead. **No administrator/UAC action**.

## Explicit remaining work

- Client integration (WndProc/iced window handle, preferences/reducers, runtime
  task ownership, `--background`, startup/deep-link argument dispatch) is deliberately
  deferred to the refactoring agent. No resource benchmark claims before integration.
- Inline toast Reply needs a COM activation server and authenticated reply routing;
  no fake Reply control is shown. Protocol click activation is implemented.
- Native toast/notification-center appearance, real tray/taskbar interaction and
  a complete user-confirmed signed install/restart were not exercised on the
  owner's desktop. Headless XML/AUMID/badge data and mock updater verification pass.
- Custom audio is conservatively suppressed unless the host explicitly confirms
  Windows DND is off. Public shell/Focus Session checks alone cannot reliably
  detect all manual-DND states. The host must also suppress mention flashing when
  DND state is unknown.
- Owner production public key must be supplied as build-time
  `STICKTALK_UPDATE_PUBLIC_KEY` to client and helper. This local installer has no
  production key; updater fails closed. No production signing key was generated.
- Authenticode/SmartScreen trust, staged-artifact retention cleanup, and rollback
  after a failed installer are not provided. Failed updates retain their signed
  stage; checks never execute anything automatically.
