# Dependency audit disposition — 2026-11-13

These are narrow reachability/source dispositions, **not a blanket clean-audit
claim**. Do not globally ignore advisory IDs. Review again by 2026-12-13 and
whenever features or production dependency edges change.

## Exceptions requiring the stated boundary

| Advisory | Exact package | Allowed scope | Invalidates exception |
|---|---|---|---|
| RUSTSEC-2023-0071 | rsa 0.9.10 | Development-only generated test signers (server/client integration fixtures). Production Google verification uses ring. | Any normal production edge to rsa, signing/decrypting real keys, or exposure to attacker timing queries. |
| RUSTSEC-2016-0003 | portaudio 0.8.0 | Inactive optional dependency in vendored NetEQ's lock graph. NetEQ defaults are disabled; the app does not build/use PortAudio. | Enabling its feature, build script or network download. |

Before using either exception, inspect `cargo tree --locked --edges normal -i rsa`
and `cargo tree --locked --edges normal -i portaudio` for the deployed packages
and target. Neither may appear in an active normal production graph. Development
RSA test keys are randomly generated in memory, not owner credentials.

## Remediated packages

- SQLx 0.9.0 and **libsqlite3-sys 0.39.1**, bundled SQLite 3.54.1. SQLx's published
  driver constraint excludes the patched native crate, so `server/vendor/sqlx-sqlite`
  is the published 0.9.0 source with its native dependency constraint updated (and doc-comment whitespace normalized).
  `s10_bundled_sqlite_is_security_patched` checks the running library version.
- **lru 0.18.5** (minimum requested 0.18.2). The published cryoglyph 0.1.0 pins
  the old lru minor; `server/vendor/cryoglyph` changes only that constraint. Native
  all-target compilation/clippy validates its API compatibility.
- **ringbuf 0.5.3** (minimum requested 0.5.2) in vendored NetEQ. Both manifest
  forms are updated; no application ring-buffer API use needed changing.

## Tract: explicit security backport, not an unpatched-code exception

RUSTSEC-2026-0217 still identifies the nominal **tract-nnef 0.21.4** version.
Tract's ONNX/Pulse packages pin that exact version, and a wholesale upgrade also
changes the ndarray API used by pinned DeepFilterNet. The local patched source
at `crates/server/vendor/tract-nnef` therefore retains its truthful upstream
version and backports checked shape/bit/byte-size multiplication in `read_tensor`,
plus allocation overflow validation and saturating bool-size bounds. This fixes
the identified overflow without changing media APIs or model output.

Reference: https://github.com/sonos/tract/commit/34c7df2c9bd2a36583e09b52f3e6319bf23102e8
and https://rustsec.org/advisories/RUSTSEC-2026-0217.html .
Regression: `s10_nnef_shape_and_byte_overflows_are_errors_not_panics_or_allocations`.
The original implementation panics on these crafted tensor headers in debug and
wraps in release; the patched implementation rejects them before allocation.
Only the bundled trusted DeepFilter model remains supported; this backport is not
a certification of every legacy Tract parser for arbitrary untrusted models.

All patched packages preserve their upstream license files. Remove the local
patches when upstream dependency constraints permit patched releases directly.

## Reproducing the raw audit

Portable tool: `C:/Users/dwhee/.cache/sticktalk-secreview-audit/cargo-audit-x86_64-pc-windows-msvc-v0.23.0/cargo-audit.exe`.
Run `cargo-audit audit --file Cargo.lock --db ADVISORY_DB --json` without ignores.
The raw result and database revision are recorded in `SECURITY_FIX_VALIDATION.md`.
A nonzero raw result remains expected for the two scoped exceptions. Registry
advisories do not attest to local path dependencies: the Tract backport must be
reviewed/tested independently even though it is absent from the raw alert list. Six upstream
unmaintained notices (anymap2, paste, rustybuzz, ttf-parser, basic-toml,
proc-macro-error2) are not silently
suppressed. The latter two are build-time age localization helpers, not crypto
operations; they need upstream migrations rather than vulnerability exceptions.

Backup encryption pins age 0.11.3 with all optional SSH/plugin/CLI features off.
It supports the configured standard X25519 public recipient, interoperable with
age/rage. Age 0.12.1 was evaluated but introduces vulnerable crypto-bigint 0.5
through mandatory HPKE/P-256 dependencies (RUSTSEC-2026-0387); it is deliberately
NOT used. Do not upgrade blindly before that transitive edge is fixed.

The reviewed `tract-nnef/src/tensors.rs` SHA-256 is
`7422d8c8ab7ac1d9bed505a6b30ff5124bd6e7a8ef0577027fd84fc0107d9f2f`.
