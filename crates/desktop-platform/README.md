# StickTalk desktop platform

Independent Windows integration library, plus the `sticktalk-maintenance` helper.
The desktop client integrates these APIs. Linking this crate creates no threads, windows,
tray icons, devices, registry entries, or notifications. The host owns its existing
Tokio runtime, window/event loop, preferences, authentication, and UI reducer.
Runtime dependencies are target-gated where appropriate; protocol DTOs are reused.

## Public API / integration order

1. Before creating a window, await
   `windows::instance::Instance::launch(local_app_data_dir, activation)`.
   `Launch::Forwarded` means exit; retain `Launch::Primary(instance)` until exit.
   The initial activation is supplied to the host reducer directly. Repeatedly
   await `instance.receive()` for subsequent launches; dispatch received values
   to the reducer, then `windows::focus_window(hwnd)` on the UI thread. Invites
   and messages must still pass server authorization; deep links never execute
   commands. Input is bounded to 1,024 bytes, with strict routes and positive
   signed-63-bit IDs. Owner-only named-pipe DACL, remote clients rejected, OS file
   lock, two-second connection/read limits. Foreground permission is transferred
   by the second launcher. There is no idle polling or dedicated IPC runtime.
2. On the UI thread, retain `windows::Apartment::initialize()` (or use the host's
   existing compatible COM/WinRT apartment), then `windows::set_app_id()` before
   creating the taskbar window. AUMID is **`StickTalk.Desktop`** everywhere.
3. `NotificationPolicy::permits(NotificationContext)` implements server/channel
   all/mentions/nothing, mute, focused-current-channel suppression, DMs and DND.
   The host resolves/stores per-server and per-channel preferences. Construct
   `MessageToast` with cached local avatar file URI, sender, server/channel,
   preview and IDs; call `windows::show_toast`. XML is escaped/size-bounded and
   accepted by Windows in headless tests. Protocol activation targets
   `sticktalk://message/<channel>/<message>` even after app exit, via the installer.
   `Activation::parse` also accepts `sticktalk://invite/<code>` and `sticktalk://open`.
4. `windows::set_unread_badge(hwnd, count)` uses ITaskbarList3, clears on zero,
   caps the 16px raster count at 99 (full count in accessible description), and
   destroys its temporary HICON. `flash_mention(hwnd, dnd)` flashes three times,
   never continuously. Badge colors use UI_SPEC danger/text.primary tokens.
5. Retain `TrayIcon::add(hwnd, WM_APP + n, Icon::load_ico(branded_icon_path)?)`.
   Forward callback `lParam` to `on_callback(event, muted, deafened)`; dispatch
   `TrayAction::{Open,Mute,Deafen,Quit}` to the reducer. Mute/deafen actually change
   host audio state; this library never opens a microphone. Route window-close
   through `DesktopPreferences::close_action(tray_available)` and hide only for
   `CloseAction::Hide`; explicit Quit always exits. Default minimize-to-tray is
   on, startup is **off**. After user opt-in, `set_start_with_windows(exe, true)`
   writes quoted HKCU Run with `--background`; the host must support that flag.
   On Explorer's registered `TaskbarCreated` message call
   `restore_after_explorer_restart` and restore the unread overlay. Dropping the
   tray guard removes its icon. No window is created/subclassed by this crate;
   the host must connect WndProc messages during the later client integration.
6. Run `play_sound(Sound, quiet_state_confirmed)` on the host's **bounded** blocking
   executor, only when policy permits. Six original mono PCM WAVs total **49,224
   bytes**, 120–240 ms; `PlaySound` closes the device when each call returns. No
   persistent audio device, thread, mixer, codec, animation or polling loop.

### DND and inline reply limitations

Windows itself applies DND/Focus Assist to native toasts. We never use urgent,
alarm, reminder, or bypass attributes; shell quiet/fullscreen/presentation states
and active Focus Sessions additionally set SuppressPopup. Toasts are silent to
avoid duplicate/uncontrolled sounds. `play_sound` defaults to doing nothing unless
the host explicitly confirms Windows DND is off; shell/Focus Session checks are
also required. **There is no reliable universal public manual-DND query for
unpackaged custom audio**, so do not interpret an app's own unmuted setting as
that confirmation. Conservative custom-sound suppression is intentional until
host policy can supply this signal. Taskbar flashing has the same host DND flag.

Inline Reply is **not implemented**: protocol activation does not deliver text
input reliably. A persistent COM toast activator, installer CLSID registration,
and authenticated host reply routing are needed. No nonfunctional Reply button
is exposed. Click-to-open works without that COM server.

References: [desktop AUMID requirements](https://learn.microsoft.com/en-us/windows/win32/shell/enable-desktop-toast-with-appusermodelid),
[shell notification state](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shqueryusernotificationstate),
[Focus Sessions](https://learn.microsoft.com/en-us/windows/apps/design/shell/tiles-and-notifications/toast-ux-guidance).

## Authenticated updater

`Updater::embedded(https_origin, installed_semver, staging_dir)` reads the
**compile-time** `STICKTALK_UPDATE_PUBLIC_KEY` (base64, 32 bytes). Absent/invalid
key fails closed; there is no generated production key, test-key fallback, or
HTTP-supplied trust anchor. Set the same key when building the client library and
maintenance helper. `Updater::new(..., key)` is explicit injection for tests or
enterprise builds, not server data. Use one Updater (clones share a download gate)
and one worker per running client. Stage inside its per-user LocalAppData directory.

```rust,ignore
let updater = Updater::embedded(origin, env!("CARGO_PKG_VERSION"), staging)?;
// UI retains receiver; one existing-runtime task, one immediate/6 h timer:
let (tx, mut rx) = tokio::sync::mpsc::channel(1);
let worker = updater.clone();
let task = tokio::spawn(async move { worker.watch(tx).await });
// On an update event, spawn/await download on the same runtime:
if let Some(Ok(Some(update))) = rx.recv().await { updater.download(&update).await?; }
// At next launch inspect updater.pending().await? before showing a window.
// Only after user confirms: updater.restart_to_apply(&installed_exe).await?;
// Then exit promptly; helper waits for this process before installation.
```

`watch` checks GET `/api/update/latest` immediately and every six hours. Receiver
drop or task cancellation stops it. Full UI queues apply backpressure, not a new
task. HTTP redirects are refused; public origins require HTTPS (loopback HTTP is
allowed for mocks). Envelope reads and installer size are bounded. Ed25519 is
verified over **the original manifest bytes before parsing**. Validate target,
strict configured-origin content-addressed URL, SHA-256, size <=1 GiB and semver;
never downgrade an installed version. Current/older signed releases and 404 are
normal no-update results. Invalid signatures/errors are surfaced to the host.

Download streams in the background; hash-named `.part` files survive cancellation
and process restart. Resume validates exact Content-Range. A server ignoring
Range causes a safe truncation/restart. Size and SHA-256 must match before atomic
rename to `.exe` and writing `pending.json` (signed envelope, not an executable
path). `pending()` reauthenticates and rehashes disk data on every restart.

`restart_to_apply` copies the installed helper into staging so NSIS can update the
installed helper too. The helper waits on the parent's process handle (no loop),
reauthenticates staged data, locks the installer against writes/deletes, runs NSIS
`/S /D=<existing-install-directory>`, checks exit status, clears the pending marker,
then starts the installed client. The client calls this path after explicit Restart confirmation or to finish a
previously deferred update on an ordinary interactive launch. Watch/download never
execute installers. Test fixtures exercise restart only against loopback/test keys. Failed installs retain the stage for retry. Installer
and copied helper files remain in staging for diagnostics; retention cleanup is
left to the host. Authenticode/SmartScreen and production signing-key provisioning
are separate release responsibilities. The release-engineering integration includes real NSIS apply/restart, failed-startup
rollback, and recovery tests; see `deploy/windows/VALIDATION.md`.

## Safe feature demos

All default commands are safe on the owner's active desktop:

```powershell
$env:CARGO_TARGET_DIR = "$env:USERPROFILE/.cargo-target/sticktalk-desktop-platform"
cargo run -p sticktalk-desktop-platform --example notifications # escaped XML only
cargo run -p sticktalk-desktop-platform --example tray          # reducer/menu only
cargo run -p sticktalk-desktop-platform --example sounds        # byte counts only
cargo run -p sticktalk-desktop-platform --example single_instance # temp IPC, no HWND
cargo run -p sticktalk-desktop-platform --example updater       # offline plan only
cargo test -p sticktalk-desktop-platform --test updater         # test key + loopback
```

## Release integration

Current packaging, test commands, signing-key setup, publishing, and rollback
behavior are documented in [`deploy/RELEASING.md`](../../deploy/RELEASING.md).
`deploy/windows/build-installer.ps1` uses portable NSIS and includes the x64
app-local MSVC runtime, client, maintenance helper, icon and artwork licenses.
The optional `sticktalk-package` CLI is **not installed**; it provides offline
key generation/signing and the loopback test staging command.

The client now wires IPC, tray/autostart, signed startup/six-hour checks, the
restart pill, a UI-health acknowledgment, local diagnostics, and native window
placement. `windows::update_install` owns the 15-second startup grace period,
owned-file backup/restore, failed-release quarantine, and interrupted-setup
recovery. No update restarts automatically without the user's confirmation.
