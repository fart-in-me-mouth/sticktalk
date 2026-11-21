# StickTalk desktop (Windows)

The server's own web app in a native window. `sticktalk-desktop.exe` (installed
as `sticktalk.exe`) is a thin shell around the Microsoft Edge WebView2 Runtime:
it navigates to the StickTalk server a person chose and to nothing else. The web
app is never bundled, so the app always matches the server's version.

Needs the WebView2 Runtime, which ships with Windows 11 and current Windows 10.

## What the shell does

- **Server address.** `%LOCALAPPDATA%\StickTalk\server.txt`, else the build's
  `STICKTALK_DEFAULT_SERVER`. Without a valid one it shows a built-in page
  (compiled in, served over a WebView custom protocol, no network) that checks
  `GET <origin>/api/v1/health` before saving. The same page appears, with the
  address filled in and a Retry button, whenever the server can't be reached.
- **Change server.** Ctrl+Shift+S, or "Change server…" in the title bar menu
  (right-click the title bar, or Alt+Space).
- **Navigation.** Only the server's origin loads in the window. Other links,
  `target=_blank` and `window.open` go to the default browser. Google sign-in
  started by the server (`/api/v1/browser/google/start`) may visit Google's
  sign-in hosts until it returns to the server. See `src/policy.rs`.
- **Calls.** Microphone and camera are granted to the server's pages only,
  without a prompt; every other page is refused. Screen sharing uses WebView2's
  own "choose what to share" picker, for the server's pages only.
- **Remote control.** Someone watching your shared screen or window can ask
  to control it. The page can only ask: `src/control.rs` puts the question in a
  Windows message box, performs pointer and key input only on what is shared,
  refuses the Windows key, and stops when you press Esc on your own keyboard.
  It is only offered while the server's page is sharing, and shares from an
  ordinary browser can't be controlled. The app cannot check the call itself:
  it trusts the server it was pointed at to say who is asking, which is why the
  question is asked every time and never remembered.
- **Window.** Size, position and maximized state are kept in
  `%LOCALAPPDATA%\StickTalk\window.json` and clamped to a visible monitor. The
  WebView profile is `%LOCALAPPDATA%\StickTalk\WebView`.
- **One instance.** A second launch, or a `sticktalk://` link, brings the
  running window forward. `sticktalk://invite/<code>` opens `/?invite=<code>`
  and `sticktalk://message/<channel>/<message>` opens `/?channel=<channel>`.
- **Idle.** One event loop that sleeps; no timers except a single one-shot
  after a move/resize to save the window placement.

## Checking it without showing a window

Never run the app normally on a PC someone is using. `--smoke` runs the same
code with the window hidden for its whole life, a throwaway data folder under
`%TEMP%`, and no default-browser launches:

```powershell
sticktalk-desktop.exe --smoke http://127.0.0.1:8080/ | Write-Output
# {"ok":true,"title":"StickTalk","url":"http://127.0.0.1:8080/"}   exit code 0
```

It waits for the page load (20 s at most), prints one line of JSON and exits 0
when the page loaded, 1 when it did not, 2 on a timeout or an address the app
would not load. Pipe or redirect the output: release builds have no console.
Optional, for tests:

| Flag | Effect |
|---|---|
| `--smoke-eval <js>` | Run a script in the first page (to drive the built-in page), then report the last load |
| `--smoke-wait <ms>` | How long the page must be quiet before the report (default 1500) |
| `--smoke-shot <png>` | Also save what the page rendered |
| `--smoke-root <dir>` | Use this data folder, so a second `--smoke` launch reaches the first one |

`http://sticktalk-shell.localhost/` is the built-in page. Diagnostics (each
load, and links that would have opened the browser) go to stderr.

## Build and test

```powershell
cargo test -p sticktalk-desktop
cargo clippy -p sticktalk-desktop --all-targets -- -D warnings
cargo build --release -p sticktalk-desktop
```

The installer is built by `deploy/windows/build-installer.ps1`; see
`deploy/RELEASING.md`.
