# Portable handoff validation

Date: 2026-11-13/14, Windows x64. This is a **fresh-clone simulation**, not a
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
| `deploy/pi/build.ps1 -InstallTools` | PASS: aarch64 GNU/Linux, glibc 2.36 baseline; ARM64 ELF header verified; 20,475,312 bytes |
| Desktop release via `deploy/windows/build-installer.ps1 -TestFixture` | PASS: native client, maintenance and package tools built; installer 53,717,855 bytes |
| `deploy/test-portability.ps1` | PASS: 91 tracked tool/config files; PS/JS syntax, no literal user-profile paths, default Cargo target, private-file ignore rules |
| `cargo fmt --all -- --check` | PASS |
| `npm ci` and headless Playwright Chromium install | PASS; npm reported zero vulnerabilities |
| `node tests/platform.mjs` | PASS: worker, SDP/MSID, speaking bounds, preferences/gestures and codec selection contracts |
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

The remaining test/PWA validation results and final cleanup are recorded below
when the matrix completes.

## Operational boundaries

- A clean OS still needs MSVC/SDK installed by its administrator; the provided
  scripts do not request elevation or pretend this prerequisite is portable.
- Physical Pi/LUKS/systemd/Tailscale, Oracle, DNS/HTTPS, off-device encrypted restore
  and failover rehearsal require the friend's equipment and credentials. They
  are documented, not remotely exercised here.
- Linux ARM64 server is cross-compiled, not executed here. Native Linux/macOS
  desktop media parity is not certified; use the PWA on those systems.
- The legacy `crates/media/tools/build-env.ps1` remains historical and untouched
  because media code/tooling is owned by the concurrent media task. The new
  `deploy/dev-env.ps1` replaces it for normal builds; media docs warn against using
  that old machine-specific qualification setup.
- Cargo emits an upstream future-incompatibility notice for `proc-macro-error2`
  2.0.1. It is not a first-party Clippy failure on the validated toolchain.

Raw logs and disposable fixture artifacts are under
`C:/sticktalk-handoff-validation/` on the validation PC. The final transfer
bundle's `HANDOFF-CONTENTS.txt` records the exact packaged HEAD and SHA-256.
