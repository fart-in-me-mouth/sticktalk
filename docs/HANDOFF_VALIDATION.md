# Portable handoff validation

Date: 2026-11-13 local time (2026-11-14 UTC), Windows 11 Home 26200 x64.
This is a **fresh-clone simulation**, not a
claim that another physical OS or Pi has been commissioned.

## Setup and isolation

- Cloned with `git clone --no-hardlinks` into
  `C:/sticktalk-handoff-validation/clean`, outside OneDrive and user-profile paths.
- Exactly one physical build tree: the task-specified `sticktalk-handoff`
  directory. The clone's ignored `target` is a junction to that tree, avoiding a
  second copy while testing Cargo's real default target resolution.
- Server build/tests/Clippy ran with `CARGO_TARGET_DIR` **unset**. Cargo metadata
  confirmed `<clean clone>/target`. The normal Windows helper supplied explicit
  task-local native tools; no source/config edits were made in the clone.
- Rust 1.99.1 / Cargo 1.99.1, Node 22.22.3, Python 3.11.9. Native tools:
  CMake 3.31.6, Meson 1.7.2, Ninja 1.13.0, libclang 18.1.1.
- WASM tools: wasm-bindgen 0.2.130, wasm-split-cli 0.2.5, Binaryen 131.
  Both downloaded portable archives were SHA-256 checked by the committed helper.
- `CARGO_INCREMENTAL=0`; workspace profiles unchanged. Package/toolchain source
  caches and installed MSVC/Windows SDK are prerequisites, not an empty OS image.
  The first preparatory native server build populated this same tree; subsequent
  clone builds reuse Cargo caches where valid.
- No visible windows, UAC, real OAuth/provider credentials, public listeners,
  deployment changes or Git pushes. Tests use loopback and headless Chromium.

## Completed checks

| Check | Result |
|---|---|
| `cargo build --locked --profile server -p sticktalk-server --bin sticktalk-server` | PASS in clean clone, default target |
| `cargo test --locked --profile server -p sticktalk-server -p sticktalk-proto -p sticktalk-client-core -p sticktalk-web -p sticktalk-emoji -p sticktalk-desktop-platform` | PASS: 255 passed, 1 ignored |
| Same package group, `cargo clippy --locked --profile server ... --all-targets -- -D warnings` | PASS |
| `cargo test --locked --profile server -p sticktalk-sfu` and matching all-targets Clippy | PASS: 41 passed |
| `cargo test --locked -p sticktalk -p sticktalk-media` and matching default-profile all-targets Clippy | PASS: 213 passed, 14 ignored; combined matrix covers all nine Rust packages: **509 passed, 15 ignored** |
| `deploy/pi/build.ps1 -InstallTools` | PASS: aarch64 GNU/Linux, glibc 2.36 baseline; ARM64 ELF header verified; 20,475,312 bytes |
| Desktop release via `deploy/windows/build-installer.ps1 -TestFixture` | PASS: native client, maintenance and package tools built; installer 53,717,855 bytes |
| `deploy/test-portability.ps1` | PASS: 91 tracked tool/config files; PS/JS syntax, no literal user-profile paths, default Cargo target, private-file ignore rules |
| `cargo fmt --all -- --check` | PASS |
| `npm ci` and headless Playwright Chromium install | PASS; npm reported zero vulnerabilities |
| `node tests/platform.mjs` | PASS: worker, SDP/MSID, speaking bounds, preferences/gestures and codec selection contracts |
| `crates/web/scripts/build.ps1`, unset target on entry, explicit portable tools | PASS: production split WASM; revision `123d25836677f8d07f4b`; core 449,946 bytes Brotli and 31 lazy chunks (asset inventory, not a measured cold-load claim) |
| `cargo clippy --locked -p sticktalk-web --target wasm32-unknown-unknown --profile web-release --all-targets -- -D warnings` | PASS |
| `npm test` | PASS: loading/worker contracts plus live accounts, history, uploads, roles, cookies, audio and decoded H.264 through the actual SFU; 12 headless screenshots; zero app errors/CSP violations |
| `npm run test:server` | PASS: real-server CSP smoke, zero application violations, injected inline probes blocked |
| Extended QA | Polish/platform/loading PASS; **phone account-erasure flow FAILS** (see below). Admin, mock OAuth, offline/PWA updates, phone media and three-round voice negotiation PASS when run independently |
| Windows PowerShell 5.1 `deploy/test-portability.ps1` | PASS, including all 91 tracked tool/config files and documentation environment-variable quoting; downloads use `-UseBasicParsing` for PS 5.1 compatibility |
| Windows PowerShell 5.1 native/WASM helper smoke | PASS: MSVC discovery and pinned wasm-bindgen/wasm-opt versions; matching tool paths |
| Transfer bundle helper | PASS: all-branch bundle verification, ordinary clean clone, matching HEAD and `git fsck`; untracked-file negative test refuses packaging |
| `bash -n deploy/install.sh` (Git for Windows Bash) | PASS; installer now honors the caller's target or defaults to repo/target |

The ARM64 ELF SHA-256 was
`08e66da3a7d4edaf01f602107e7b877a2e15d1a9cb7b0f3e78098acf60382152`.

The desktop build used `USERPROFILE=C:/sticktalk-handoff-validation/empty-profile`
with explicit toolchain cache locations, so the old per-user media-tool bundle
could not be discovered. `CARGO_TARGET_DIR` was unset on installer entry; the
script selected the clone's default target. The installer embeds only an
ephemeral test update key and loopback origin: **do not distribute it**. It was
built, not installed, and no OS registration was changed.

Release-mode client/media test **compilation** was stopped conservatively at
9,248,758,453 target bytes, before tests ran. The release client/installer build
above was already complete. That tree was fully cleaned before retrying tests
in the workspace's ordinary dev/test profile; no debug and release trees were
retained together and no debug/profile overrides were used.

This exposed Cargo's cache-tag protection for manually precreated directories.
The helpers now mark **only newly created** tool/build directories as disposable
cache. Existing directories are not relabeled. Fresh helper setup and
`cargo clean --dry-run` both passed, including a valid cache tag.

Relative `CARGO_TARGET_DIR=target` was normalized before the web helper changed
working directories. Both WASM binaries came from this task's tools, not Trunk's
existing user cache. The revised helper extracts only wasm-opt, not the archive's
approximately 610 MB of unrelated binaries. No NASM/Meson/Ninja/CMake was on the
parent shell PATH; the native builds used the explicitly installed task tools.

## Outstanding extended-QA failure

`npm run test:qa` stopped in `tests/phone.mjs:281`: after confirmed account
deletion, the UI still showed **Unavailable** and the login selector timed out.
The server also logged a housekeeping warning. Root cause is not established;
this is an unresolved release gate, not a passing certification. No application
code or assertions were changed to hide it. Investigating this functional path
was left outside the time-boxed build/path handoff and concurrent server ownership.
The remaining independent QA scripts were run separately and passed.

Portable evidence: [synthetic-account failure snapshot](HANDOFF_PHONE_FAILURE.json).
The original screenshot/logs are under `C:/sticktalk-handoff-validation/web-qa/`.

## Disk and cleanup

Largest observed target: **9,248,758,453 bytes (9.25 GB decimal)**, at the
conservatively stopped release-test compile. Completed default client/media
tests plus Clippy used 8,489,637,516 bytes. Profiles were cleaned between stages.
Final `cargo clean --target-dir <task target>` passed: removed 9,076 files / 2.0
GiB in the last stage. `Test-Path` confirmed **the physical task target is absent**.
The transfer helper independently verifies/clones the final committed bundle;
its adjacent manifest records the exact packaged HEAD and SHA-256.

## Operational boundaries

- A clean OS still needs MSVC/SDK installed by its administrator; the provided
  scripts do not request elevation or pretend this prerequisite is portable.
- Physical Pi/LUKS/systemd/Tailscale, Oracle, DNS/HTTPS, off-device encrypted restore
  and failover rehearsal require the friend's equipment and credentials. They
  are documented, not remotely exercised here.
- Linux ARM64 server is cross-compiled, not executed here. Native Linux/macOS
  desktop media parity is not certified; use the PWA on those systems.
- Windows 10 was not separately executed on a VM/physical PC; this machine's
  actual validation OS was Windows 11. Hardware/long-soak ignored tests remain
  unrun; this work does not certify call quality or all OS-integration behavior.
- The legacy `crates/media/tools/build-env.ps1` remains historical and untouched
  because media code/tooling is owned by the concurrent media task. The new
  `deploy/dev-env.ps1` replaces it for normal builds; media docs warn against using
  that old machine-specific qualification setup.
- Cargo emits an upstream future-incompatibility notice for `proc-macro-error2`
  2.0.1. It is not a first-party Clippy failure on the validated toolchain.

Raw logs and disposable fixture artifacts are under
`C:/sticktalk-handoff-validation/` on the validation PC. The final transfer
bundle's `HANDOFF-CONTENTS.txt` records the exact packaged HEAD and SHA-256.
