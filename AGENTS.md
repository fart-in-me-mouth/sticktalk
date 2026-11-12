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
- Screen-capture benchmarks are fine, because capturing is invisible, but they must not open preview windows.

## Build hygiene
- Build output must stay out of OneDrive. Use your own target dir per task, for example `$env:CARGO_TARGET_DIR = "C:/Users/dwhee/.cargo-target/sticktalk-<task>"`.
- Git: if you were given your own worktree/branch, commit your work there with clear messages when done (never push, never rewrite history). In the shared main checkout, do not commit; the lead does.
- Edit only the crates your task names. `crates/proto` changes must be additive and wire-compatible.
- Before reporting done, run `cargo test`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check` for the crates you touched.

## Product priorities
1. Extremely low resource use on users' PCs:
   - client idle RAM under 60 MB
   - ~0% idle CPU
   - no animation loops or polling
2. Polished UI that follows `docs/UI_SPEC.md` exactly.
3. Security: validate all input on the server, and check permissions on every action.

## Reports
Keep the final report short: what changed, what's not done and why, key numbers, and the commands that passed. The lead reads screenshots directly, so don't describe them at length.
