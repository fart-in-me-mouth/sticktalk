# StickTalk: build, run and look after it

This guide is for the friend taking over the **whole repository**, not just an
installer. Commands start at the repository root unless a step says otherwise.
No source edits are needed to build. Real deployment requires your own addresses
and credentials in private environment files, not changes committed to Git.

## 1. Understand the five pieces

1. StickTalk is a Rust chat app with a native Windows client and a browser PWA.
2. The **Oracle ARM64 VM** runs the main Rust server: API, chat, SQLite and SFU.
3. Caddy serves HTTPS on Oracle; the SFU forwards voice/video over separate UDP.
4. Your **Raspberry Pi** is private file storage over Tailscale, not the public API.
5. Pi also holds nightly encrypted database backups and a **manual cold standby**.

Read [architecture](docs/ARCHITECTURE.md), [protocol](docs/PROTOCOL.md), and the
[feature/QA caveats](docs/QA_WEB_REPORT.md). Old benchmark reports are historical;
see [the evidence note](docs/HISTORICAL_EVIDENCE.md). Building successfully is not
proof of acoustic quality, real-device browser compatibility or disaster recovery.

## 2. Receive the repository without GitHub

The sender commits their work, then runs in ordinary PowerShell:

```powershell
./deploy/make-handoff-bundle.ps1 -Output "$HOME/sticktalk-transfer"
```

This bundles all local branches and tags, verifies the bundle, clones it into a
temporary folder, checks HEAD/object integrity/clean status, then removes that
verification clone. No build-required untracked files currently exist, so there
is **no extra ZIP**. Needed nonsecret assets must be tracked before packaging;
the script refuses uncommitted/untracked changes rather than guessing what to
copy. It never includes ignored target directories, local databases or secrets.
Committed history *is* included: review it for credentials before sharing.

Copy `sticktalk.bundle` and `HANDOFF-CONTENTS.txt` to your PC (USB is fine).
Compare the SHA-256 with the sender over a separate trusted channel:

```powershell
Get-FileHash ./sticktalk.bundle -Algorithm SHA256
git clone ./sticktalk.bundle sticktalk
cd sticktalk
git log -1 --oneline
git branch -a
```

The clone checks out the sender's handoff HEAD, even if master is older. Other
branches initially appear as `origin/<name>`; use `git switch --track origin/NAME`
if needed. Keep the original bundle safely. `origin` is a local bundle, **not** a
live Git host; configure a real private remote later if desired. On Linux/macOS
use `sha256sum sticktalk.bundle` / `shasum -a 256 sticktalk.bundle`, then the same
Git commands. Exchange operational secrets separately and encrypted.

## 3. Prepare Windows 10/11 x64

Use a normal PowerShell, not an administrator shell. Put the checkout on a local
disk outside OneDrive, for example `$HOME/src/sticktalk`. Keep at least 10 GB free
for a build tree plus room for sources, installers and downloads. A full build
can take tens of minutes. Do not sync live databases, uploads or build trees.

### Prerequisites (once)

- **Git**: `winget install --id Git.Git --exact --scope user`. If that package
  does not offer user scope on your PC, use the official [portable Git](https://git-scm.com/downloads/win)
  ZIP and add its `cmd` directory to this shell's PATH; do not silently retry elevated.
- **Rust stable/MSVC**, per user:

  ```powershell
  Invoke-WebRequest https://win.rustup.rs/x86_64 -OutFile "$env:TEMP/rustup-init.exe"
  & "$env:TEMP/rustup-init.exe" -y --default-host x86_64-pc-windows-msvc --profile minimal
  $env:PATH = "$HOME/.cargo/bin;$env:PATH"
  rustup component add rustfmt clippy
  ```

- **Visual Studio 2022 Build Tools**, *Desktop development with C++*, including
  MSVC x64/x86 tools and a Windows 10/11 SDK. Community/Professional/Enterprise
  2022 with those components also work; the helper discovers any edition.
  Microsoft installs these machine-wide and may require an administrator.
  There is no supported no-admin replacement for this MSVC prerequisite. If
  unavailable, have the PC administrator install it; **stop rather than trying
  UAC workarounds**. The administrator's explicit install command is:

  ```powershell
  Invoke-WebRequest https://aka.ms/vs/17/release/vs_BuildTools.exe -OutFile "$env:TEMP/vs_BuildTools.exe"
  & "$env:TEMP/vs_BuildTools.exe" --wait --passive --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended
  ```

  References: [Rust prerequisites](https://rust-lang.org/tools/install/),
  [Microsoft installer arguments](https://learn.microsoft.com/en-us/visualstudio/install/use-command-line-parameters-to-install-visual-studio).
- **Node LTS**: download the Windows x64 **ZIP**, not MSI, from
  [Node's official downloads](https://nodejs.org/en/download), extract under
  `$HOME/Tools/node`, and add the directory containing `node.exe` to this shell:
  `$env:PATH = "$HOME/Tools/node;$env:PATH"`. Check `node --version` and `npm.cmd --version`.
  The tests use modern Node APIs; use Node 22 or newer LTS, not an old system Node.
- **Python 3.11+ with pip**: `winget install --id Python.Python.3.11 --exact --scope user`.
  Reopen PowerShell if its PATH changed. Check `python -m pip --version`.
- **NSIS**: no manual install: the installer script downloads a SHA-256-pinned
  official portable NSIS ZIP under the selected target. Alternatively pass
  `-MakeNsis C:/your/tools/makensis.exe`. It never installs the app or elevates.

### Build shell (each new shell)

```powershell
# Optional: without this override Cargo uses the ignored <repo>/target.
$env:CARGO_TARGET_DIR = "$HOME/.cargo-target/sticktalk"
$env:CARGO_INCREMENTAL = '0'
. ./deploy/dev-env.ps1 -InstallTools
```

`-InstallTools` installs pinned CMake/Meson/Ninja/libclang Python wheels inside
that one target directory, without administrator rights. On subsequent shells,
dot-source without `-InstallTools`. The script sets only process-local PATH,
Python/libclang and `CMAKE_GENERATOR='Visual Studio 17 2022'`. The generator is
**not** set globally in Cargo, so Linux/macOS and ARM cross-builds are unaffected.
Do not source the historical `crates/media/tools/build-env.ps1`; it describes an
older developer setup. No NASM is required for the normal default desktop build.

## 4. Build each part

Use `--locked` so Cargo.lock is honored. The Windows desktop is the supported
native release target; Linux/macOS desktop media/installer parity is not certified.

```powershell
# Oracle/server binary for local Windows tests (unwinding enabled for SFU):
cargo build --locked --profile server -p sticktalk-server --bin sticktalk-server
# Native Windows desktop, maintenance helper and signing/package tool:
cargo build --locked --release -p sticktalk --bin sticktalk -p sticktalk-desktop-platform --bin sticktalk-maintenance --bin sticktalk-package
```

Outputs: `$env:CARGO_TARGET_DIR/server/sticktalk-server.exe` and
`$env:CARGO_TARGET_DIR/release/{sticktalk,sticktalk-maintenance,sticktalk-package}.exe`.
Without an override substitute `./target`. Do not keep debug and optimized trees
together on a small disk. Save the server executable in `target/tools/bin` before
cleaning its profile if necessary; inspect target size regularly, delete unused
profiles, and never exceed your available space. `cargo clean --profile server`
removes that profile, not your checked-in sources or live data.

### Web PWA

This uses a Leptos split-WASM packager, **not a JavaScript web-app build**. Trunk
is optional, not needed by the normal `build.ps1` path. On Windows x64 the helper
installs the WASM Rust target, pinned/checksummed portable wasm-bindgen/Binaryen,
and the source-built split CLI, and configures this shell:

```powershell
. ./crates/web/scripts/install-tools.ps1
```

Manual alternative (also useful on other hosts):

```powershell
rustup target add wasm32-unknown-unknown
cargo install wasm_split_cli_support --version 0.2.5 --locked --features build-binary --root "$env:CARGO_TARGET_DIR/tools"
cargo install wasm-bindgen-cli --version 0.2.130 --locked --root "$env:CARGO_TARGET_DIR/tools"
$env:PATH = "$env:CARGO_TARGET_DIR/tools/bin;$env:PATH"
```

Install the official [Binaryen version 131 archive](https://github.com/WebAssembly/binaryen/releases/tag/version_131)
for your host, extract under `target/tools`, and set `WASM_OPT` to its
`bin/wasm-opt.exe` (`wasm-opt` on Unix). Set `WASM_BINDGEN` to the installed
`tools/bin/wasm-bindgen.exe` if another version is on PATH. Its version **must match
Cargo.lock** (currently 0.2.130); update both together when dependencies change.
`WASM_SPLIT` can override the default `target/tools/bin/wasm-split-cli.exe`.

```powershell
cd crates/web
npm.cmd ci
./scripts/build.ps1
cd ../..
```

Production output is `target/web-dist`, including hashed lazy WASM chunks and
compressed assets. Copy the **whole directory**, not only index.html. Never ship
`STICKTALK_WEB_MOCK=1` or a `--features mock` build. See
[web build/QA](crates/web/README.md) and [hosting](deploy/BROWSER.md).

### Pi ARM64 cross-build from Windows (no Docker/admin)

```powershell
./deploy/pi/build.ps1 -InstallTools
```

This installs per-user Zig 0.13.0/cargo-zigbuild 0.24.1 and the Rust ARM target,
then builds for `aarch64-unknown-linux-gnu.2.36` (64-bit Debian Bookworm's glibc
baseline). It validates the ELF ARM64 header. It builds only the server, **not**
Windows audio/GUI. Do not copy a newer Ubuntu-glibc binary to Bookworm. Output:
`target/aarch64-unknown-linux-gnu/server/sticktalk-server`. Restore the Windows
build shell by dot-sourcing `deploy/dev-env.ps1` before another native build.
For initial Pi storage setup follow its README; `deploy/pi/deploy.ps1` is the
legacy Pi-primary updater and restarts `sticktalk`, not `sticktalk-storage`.

## 5. Run locally, then test

No credentials or real Google account are needed for password sign-in/local QA.
Start in a dedicated terminal; Ctrl+C stops the server:

```powershell
& "$env:CARGO_TARGET_DIR/server/sticktalk-server.exe" --bind 127.0.0.1:8080 --public-url http://127.0.0.1:8080 --voice-bind-ip 127.0.0.1 --voice-advertised-ip 127.0.0.1 --data-dir "$env:CARGO_TARGET_DIR/local-data" --web-root "$env:CARGO_TARGET_DIR/web-dist"
```

In another normal terminal run `target/release/sticktalk.exe` (using your selected
target), enter `http://127.0.0.1:8080`, register and send a message. Open that same
URL yourself for the PWA; no separate web server/CORS proxy is needed. Loopback
HTTP is a browser secure-context exception; real users need HTTPS. Do not reuse
the local test data directory for production.

Rust tests use mock OAuth and loopback servers, not a real browser or real secrets:

```powershell
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
./deploy/test-portability.ps1
```

On a space-limited machine run package groups separately, cleaning the old profile
before switching from release to dev/test. `--profile server` may be used for
optimized tests that need unwinding; do not override workspace debug settings.
Long hardware/audio/10-minute streaming tests marked ignored are **manual opt-in**,
not proof from the default test run. See crate QA docs before running them.

For headless web integration, with server and PWA already built:

```powershell
$env:STICKTALK_SERVER_EXE = "$env:CARGO_TARGET_DIR/server/sticktalk-server.exe"
$env:STICKTALK_WEB_DIST = "$env:CARGO_TARGET_DIR/web-dist"
$env:TRUNK_BUILD_DIST = $env:STICKTALK_WEB_DIST
$env:PLAYWRIGHT_BROWSERS_PATH = "$env:CARGO_TARGET_DIR/browsers"
# For TLS OAuth QA use the openssl executable shipped with your Git installation:
$env:STICKTALK_QA_OPENSSL = 'C:/Program Files/Git/usr/bin/openssl.exe'
cd crates/web
npx.cmd playwright install chromium
npm.cmd test
npm.cmd run test:server
npm.cmd run test:qa
cd ../..
```

Set that OpenSSL path to **your** Git location, or put `openssl` on PATH. These
tests use synthetic accounts, headless Chromium, fake devices and loopback only.
Never bind development/tests to `0.0.0.0`, `::`, LAN or Tailscale on Windows:
that can trigger Firewall prompts. Do not run installer/update smoke tests on a
PC with an existing valuable StickTalk install; read their safety checks first.

## 6. Linux and macOS build variants

**Ubuntu/Debian server** (on Oracle ARM64 build natively; no cross compiler):

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config clang libclang-dev cmake ninja-build python3 python3-venv git curl ca-certificates sqlite3
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup.sh
sh /tmp/rustup.sh -y --profile minimal
. "$HOME/.cargo/env"
export CARGO_INCREMENTAL=0
# Optional: export CARGO_TARGET_DIR="$HOME/.cache/sticktalk-target"
cargo build --locked --profile server -p sticktalk-server --bin sticktalk-server
cargo test --locked -p sticktalk-server -p sticktalk-proto -p sticktalk-client-core -p sticktalk-web
```

The server does not need desktop audio/display packages. For the web install Node
LTS from its official host archive, add the WASM target and the same two Cargo
CLI tools, and install Binaryen for Linux. With `CARGO_TARGET_DIR` set to your
absolute target path, run from `crates/web`:

```sh
npm ci
cargo build --locked -p sticktalk-web --target wasm32-unknown-unknown --profile web-release
export TRUNK_STAGING_DIR="$CARGO_TARGET_DIR/web-dist"
mkdir -p "$TRUNK_STAGING_DIR"
node scripts/assemble.mjs
```

Build from `crates/web` so its WASM relocation configuration is loaded. Preserve
the same WASM tool versions as Windows. For Pi cross-build on Linux/macOS use
a Python virtualenv with `pip install ziglang==0.13.0 cargo-zigbuild==0.24.1`,
`rustup target add aarch64-unknown-linux-gnu`, then
`cargo zigbuild --locked --profile server -p sticktalk-server --bin sticktalk-server --target aarch64-unknown-linux-gnu.2.36`.
Native ARM64 Bookworm build on a sufficiently large machine is another option;
the 2 GB Pi is not the recommended build host.

**macOS:** install Xcode Command Line Tools (`xcode-select --install`, interactive
Apple prerequisite), Rust via rustup, and Node LTS. A package manager can supply
CMake/pkg-config/LLVM/Binaryen. Server/web commands above are feasible, but macOS
has **not** been validated in this Windows handoff. Windows installers, WASAPI,
GPU capture/remote input and native desktop media are Windows-specific; use the
PWA on Linux/macOS rather than assuming full desktop feature parity.

## 7. Deploy Oracle, then your Pi

Use the full procedures, not a guessed combination of old Pi-primary scripts:

1. **[Oracle guide](deploy/README.md):** create the intended ARM64 Ubuntu VM,
   verify its *current* free-tier/cost estimate yourself, transfer source and run
   `bash deploy/install.sh` on the VM. Root/systemd/firewall changes are for your
   remote Linux server, not your Windows development PC. Review scripts first.
2. Point your **DuckDNS** IPv4 record at Oracle. Keep its API token private.
   Configure `/etc/sticktalk.env`; Caddy terminates HTTPS on ports 80/443 and
   proxies the Rust server on **127.0.0.1:8080**. Open the documented media UDP
   range 50000–50100 in both Oracle and Ubuntu firewalls. Advertise Oracle's
   public IP for SFU media, not its Tailscale IP. No TURN is bundled.
3. Copy the complete PWA distribution to `/opt/sticktalk/web` and set
   `STICKTALK_WEB_ROOT`; see [browser deployment](deploy/BROWSER.md). Retain the
   previous `pkg/<digest>` and runtime asset directories while old tabs exist.
4. **[Pi guide](deploy/pi/README.md):** use 64-bit Bookworm and powered storage.
   Before storing data, identify the disk carefully and configure **LUKS2** on
   the verified empty data partition. Formatting destroys existing contents;
   make an independent backup first. The mount must be `/srv/sticktalk` on the
   encrypted mapper, not the SD card. Decide manual unlock versus an SD-card
   keyfile; a keyfile does not protect a whole stolen Pi. Keep the recovery
   passphrase elsewhere. Do not run the old Pi-only `install.sh`.
5. **[Hybrid connection](deploy/HYBRID.md):** join Oracle and Pi to your Tailscale
   account, restrict tailnet ACLs to Oracle→Pi TCP 9400 plus operator SSH, and
   generate one shared storage HMAC secret. Pi listens only on its tailnet IP.
   Friends connect to Oracle and do not normally need Tailscale. No Pi router
   port forwarding/Funnel/public DNS. Confirm `admin storage-status` on Oracle.
6. Set the public **age backup recipient** on Oracle, enable/test the nightly
   Oracle→Pi backup timer, and inspect its logs. Pi's encrypted off-device backup
   needs its own reviewed destination/timer too: a second independent copy is
   essential because the Pi drive is the sole bulk storage copy. Follow the
   encryption/backup section in the Pi guide; never send plaintext snapshots.
7. Test from another network: HTTPS, login, chat, upload/download and a call.
   Reboot both machines and verify mounts, services and backup timers recover.

### Restore and failover rehearsal (before trusting the setup)

Decrypt a copied `.tar.age` / `.sqlite3.age` on your trusted recovery PC with the
offline age identity, into a private scratch directory. Run SQLite
`PRAGMA integrity_check;` (expect `ok`) and verify old messages and attachments.
Never restore over a live SQLite database or copy only its main file while WAL
is active. Reapply any account/content deletions since the snapshot before
serving it; encrypted backups retain deleted history until their 14-day expiry.

For a scheduled drill, notify users, **fence Oracle** (prove it cannot write),
decrypt/review a completed snapshot, place `recovery.sqlite3` in
`/srv/sticktalk/node/backups`, and on Pi run:

```sh
sudo STICKTALK_RECOVERY_DB=recovery.sqlite3 /usr/local/sbin/sticktalk-failover --oracle-fenced
```

Use the explicit encrypted-recovery procedure in the Pi guide, not an
automatically selected older plaintext snapshot. Test old history, attachment hashes
and a new message on standby. Keep Pi private: temporary native access requires
tailnet permission; browser media also requires HTTPS. Fence Pi before restoring
its *newest* state to a replacement Oracle VM and updating DuckDNS. Never run two
writable primaries or overwrite newer standby messages with an old Oracle DB.
Record the drill date, snapshot age and actual restore time. Recovery can lose
at least the time since the last successful nightly backup; it is not automatic.

## 8. Create your own credentials and keep recovery copies

Do not reuse the developer's keys. Do not commit secrets, add them to bundles,
paste them into chat/logs, or put them in OneDrive. Store private files outside
the checkout, restrict ACLs/modes and keep an encrypted admin backup.

| Item | Where it belongs / who needs recovery access |
|---|---|
| Oracle SSH key, cloud/DuckDNS/Tailscale accounts | Your password manager; a sealed recovery copy with the owner. Record MFA recovery methods. |
| Storage HMAC secret | Same value in Pi `/etc/sticktalk/storage.env` and Oracle `/etc/sticktalk.env`; encrypted operator/owner recovery copy. |
| KLIPY key | Create your own approved provider account. Configure the private server GIF settings per the Oracle guide; do not assume proxy/cache permission. Client-direct API keys are public to clients, not confidential server secrets. |
| Google OAuth | Your own Desktop app client for native builds; separate Web application client/secret on the server with the exact HTTPS callback. [Google guide](docs/GOOGLE_LOGIN.md), [browser settings](deploy/BROWSER.md). Desktop IDs/secrets are embedded build configuration (not confidential installed-client credentials); web secrets must never enter WASM. Optional: password login works without Google. |
| Update Ed25519 signing key | Generate per [RELEASING.md](deploy/RELEASING.md), only on your release PC outside the repo. Owner and friend should each hold an encrypted offline recovery copy. Only its **public key** goes into server/client builds. Losing it breaks trust for existing users. |
| Release publishing token | Random 32+ characters, only the release shell and private server config; separate from the signing key. |
| age backup identity | Generate on trusted operator PC; encrypted offline copies with both owner and friend, not Oracle/Pi. Only the public `age1...` recipient is installed on servers. Test both recovery copies. |
| Pi LUKS passphrase/header backup | Separate offline encrypted recovery copy; do not store beside the drive. Decide which trusted people can unlock it. |
| Server config and VAPID key | `/etc/sticktalk.env`, Pi environment files, Caddy config and server data `vapid.pk8`; encrypted admin backups, owner-only permissions. Never publish these with source. |

Agree explicitly who is the primary operator and who can release/recover data.
Changing backup recipients alone does not re-encrypt old archives. Losing an age
identity or LUKS unlock method can make the only backup permanently unreadable.

## 9. Release an update to users

Follow [the release runbook](deploy/RELEASING.md). First back up the server and
test database migrations. Bump the workspace version and commit; never reuse a
published version. In your configured Windows build shell set the **public**
update key and real HTTPS server/update origin, then run:

```powershell
$env:STICKTALK_UPDATE_PUBLIC_KEY = '<your base64 public key>'
$env:STICKTALK_DEFAULT_SERVER = 'https://YOUR-NAME.duckdns.org'
$env:STICKTALK_UPDATE_ORIGIN = $env:STICKTALK_DEFAULT_SERVER
./deploy/windows/build-installer.ps1 -Output "$HOME/sticktalk-releases"
```

The result is a per-user `StickTalk-Setup-VERSION.exe`, with app-local MSVC DLLs;
no runtime installer/admin access is needed by recipients. Test it in an isolated
user/test PC and keep the previous good installer. Unsigned Windows downloads
may still show SmartScreen: update signing is **not Authenticode**.
Set the private publishing token in the shell (not in scripts/history) and use
`tools/release/release.ps1 -Version VERSION -Server https://YOUR-NAME.duckdns.org`
with the signing key outside the repo. It signs the exact installer/manifest and
publishes over HTTPS. Never edit a signed manifest. Existing clients verify the
pinned key/hash and offer restart; failing startup rolls back. Server/PWA deploys
are separate from desktop updates. Back up before replacing the server; schema
downgrades require the matching backup. Keep OS/Caddy and app-local DLLs patched.

## 10. Troubleshooting

- **link.exe / CMake / VS generator missing:** install the VS **2022 C++** workload
  and SDK, not just the editor. Dot-source `deploy/dev-env.ps1`; Build Tools is
  supported. Do not set a Windows generator in global Cargo config on Linux.
- **meson / ninja / libclang missing:** rerun the dot-sourced helper with
  `-InstallTools`; deleting the target also deletes these tools. Do not rely on
  another developer's Python or absolute tool paths.
- **PowerShell blocks npm.ps1:** use `npm.cmd` / `npx.cmd`. For repo scripts use a
  trusted normal PowerShell process with `-ExecutionPolicy Bypass -File SCRIPT`;
  do not weaken machine-wide execution policy.
- **Disk full / long paths:** use one short non-synced target, incremental off,
  remove unused profiles only after stopping builds. Never recursively delete a
  computed directory until you have checked its full absolute path. Do not
  lower quality settings or change Cargo profiles just to hide a disk problem.
- **WASM tools fail:** match wasm-bindgen to Cargo.lock; set `WASM_OPT`,
  `WASM_BINDGEN`, `WASM_SPLIT` explicitly. Build from `crates/web` for relocation
  flags. Keep all hashed chunks together. Node/npm install is not the WASM build.
- **Pi ELF won't start:** verify `uname -m` is aarch64, OS is 64-bit Bookworm or
  newer, and use the `.2.36` cross-build instead of a newer Ubuntu executable.
- **HTTPS fails:** check DuckDNS A record, Oracle and Ubuntu TCP rules, Caddy
  logs and system clock. Never disable certificate validation. Do not expose 8080.
- **Calls connect but have no media:** check advertised public IP and both UDP
  firewalls; Caddy does not forward media. Restrictive networks may need TURN.
- **Storage offline:** check encrypted drive is mounted, Tailscale ACLs/address,
  matching storage secret, clocks and `admin storage-status`. Never discard the
  Oracle spool/cache unless replication is confirmed.
- **Backup timer green but restore fails:** check actual snapshot age, destination
  contents, age recipient/identity, LUKS unlock and SQLite integrity. A timer alone
  is not proof. Never delete the only good backup to make room.
- **Google hidden:** configure both native build values and matching server ID;
  the PWA uses separate web credentials/callback. Use mock tests locally.
- **Old PWA after deploy:** retain old hashed assets until tabs reload; verify
  service worker version and prompt users to Reload after saving drafts.

The handoff validation record is [docs/HANDOFF_VALIDATION.md](docs/HANDOFF_VALIDATION.md).
