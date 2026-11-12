# Windows releases (owner only)

Prerequisites: Rust/VS C++ build tools, PowerShell, and NSIS 3 (`makensis.exe`
on PATH, or pass `-MakeNsis`). Nothing opens a window during the build.
[NSIS command-line documentation](https://nsis.sourceforge.io/Docs/Chapter3.html).
The installer is per-user, uses LocalAppData and HKCU, never requests elevation,
and does not run the app automatically. Close StickTalk before installing an update.

## One-time key setup — keep it OFF the server and OUTSIDE OneDrive/repo

Default private-key location: `%USERPROFILE%/.sticktalk-secrets/update-signing.pk8`.
This is an Ed25519 PKCS#8 binary private key. The server only receives the public
key. Back up the private key in an encrypted offline location; losing it means
existing clients cannot trust future releases. Never attach it to chat or commit it.

```powershell
$env:CARGO_TARGET_DIR = 'C:/Users/dwhee/.cargo-target/sticktalk-server'
$secrets = "$env:USERPROFILE/.sticktalk-secrets"
New-Item -ItemType Directory -Force $secrets
icacls $secrets /inheritance:r /grant:r "$($env:USERNAME):(OI)(CI)F"
cargo run -p sticktalk-server --bin sticktalk-release -- keygen --key "$secrets/update-signing.pk8"
```

Copy the printed **public** key to `STICKTALK_UPDATE_PUBLIC_KEY` on the server
and in the release shell. The client engineer must embed the same key using
`option_env!("STICKTALK_UPDATE_PUBLIC_KEY")` and verify the raw manifest string
before trusting its fields, and verify the installer size/SHA-256 before running it.
The tooling does not modify client source or assert that client-side verification
has been implemented. Do not ship the updater until that integration is tested.
Key rotation requires a trusted client release, not just changing server config.

Set a separate random publishing token (at least 32 characters) in
`STICKTALK_RELEASE_TOKEN` on the server and release shell. Keep the server env file
mode 0600. This bearer token authorizes publishing; it cannot forge signatures.
Never use the private signing key as this token. Use HTTPS, including Tailscale.

```powershell
# Set public key and publishing token in this shell from your secret manager first.
./tools/release/release.ps1 -Version 0.1.1 -Server https://chat.your-tailnet.ts.net
```

Artifacts stay outside OneDrive under `%USERPROFILE%/.cargo-target/sticktalk-releases`.
The binary's Cargo version is not rewritten: update the workspace package version
through the lead before releasing and use the same `-Version`. Signed manifests
are immutable per version. The release helper rejects malformed fields, but release
ordering/downgrade prevention belongs to the client. Ed25519 is update integrity,
**not** Windows Authenticode; an unsigned installer can show SmartScreen warnings.

Wire format and upload API: `docs/PROTOCOL.md`, operational additions.
No production signing key has been generated or installed by this implementation.
