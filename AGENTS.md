# Agent rules for StickTalk (read before doing anything)

Project: StickTalk, a Discord-like chat app in Rust. Contracts:
- `docs/ARCHITECTURE.md` and `docs/PROTOCOL.md`
- `docs/UI_SPEC.md`: authoritative for every visual decision
- `docs/PHASE2_REQUIREMENTS.md`
- `docs/GOOGLE_LOGIN.md`

## The owner is using this PC while you work: never disturb them
- **Never open visible windows on the owner's screen.** UI screenshots must be rendered **headlessly**: offscreen tiny-skia rendering of the iced view tree to PNG, with no OS window. Use the client's headless screenshot mode, `sticktalk --mock --screenshots <dir>`. If it doesn't exist yet, build it before taking any screenshots.
- If a real OS window is unavoidable (testing OS integration, drag-and-drop or focus behavior):
  - Set `STICKTALK_AGENT_WINDOW=secondary`. This opens the window on the **non-primary monitor**, without taking focus or activating it.
  - Close the window as soon as the test is done.
- Never open the system browser, play sounds, show notifications or steal focus. Mock all of these in tests. The Google OAuth flow is tested with a mock authorization server and no real browser.
- **No Windows security prompts.** Tests, benchmarks and dev servers must bind to `127.0.0.1` / `::1` only, never `0.0.0.0`, `::` or the Tailscale/LAN address. Binding to those triggers Windows Firewall "Allow access" popups for every new test binary. Any "advertised IP" or auto-detect logic must be tested with injected fake addresses. Never run anything that needs admin rights/UAC. Prefer per-user installs (`winget --scope user`, portable zips). If admin is truly unavoidable, stop and report it instead.
- Screen-capture benchmarks are fine, because capturing is invisible, but they must not open preview windows.

## Build hygiene
- Build output must stay out of OneDrive. Use your own target dir per task, for example `$env:CARGO_TARGET_DIR = "C:/Users/dwhee/.cargo-target/sticktalk-<task>"`.
- Git: if you were given your own worktree/branch, commit your work there with clear messages when done (never push, never rewrite history). In the shared main checkout, do not commit; the lead does.
- Updating `Cargo.lock` and the workspace `members` list for your crates is always allowed — include them in your commit, don't ask.
- Edit only the crates your task names. `crates/proto` changes must be additive and wire-compatible.
- Before reporting done, run `cargo test`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check` for the crates you touched.

## Product priorities (owner: "10x better than Discord")
1. **Feature quality and smoothness come first.** Calls must sound better than any competitor; scrolling, typing and switching must feel instant. Spend binary size or RAM where it **buys real quality**: best-in-class voice DSP, smooth GPU rendering, great codecs. Never spend it on waste such as duplicate dependencies, idle work or bloat.
2. **Efficiency is the second priority.**
   - ~0% CPU when idle.
   - No animation loops or polling.
   - Every subsystem, audio included, does zero work when it isn't in use.
   - Memory is bounded everywhere.
   - Client idle RAM target is under ~100 MB (working set). Report private bytes too.
3. **Polished UI** that follows `docs/UI_SPEC.md` and `docs/UI_PARITY.md` exactly.
4. **Security:** validate all input on the server and check permissions on every action.

## Reports
Keep the final report short: what changed, what's not done and why, key numbers, and the commands that passed. The lead reads screenshots directly, so don't describe them at length.
