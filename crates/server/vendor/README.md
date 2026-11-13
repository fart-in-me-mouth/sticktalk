# Narrow security dependency patches

Published crates copied from the Cargo registry, with upstream licenses retained:

- sqlx-sqlite 0.9.0: Cargo.toml libsqlite3-sys constraint to 0.39.1; no runtime Rust changes.
- cryoglyph 0.1.0: Cargo.toml lru constraint to 0.18.2; no runtime Rust changes.
- tract-nnef 0.21.4: src/tensors.rs checked shape/bit/byte/allocation arithmetic,
  with overflow regression tests. See deploy/AUDIT_EXCEPTIONS.md for the exact
  upstream advisory/fix reference and why the compatible security backport is used.

The root Cargo.lock controls all normal builds. Package-local Cargo.lock files
are intentionally omitted. These directories are excluded from first-party
workspace formatting and test defaults; the targeted Tract guard test can be
run by temporarily making that package a workspace member, using the same dev
profile and task CARGO_TARGET_DIR (do not create a second target tree).

Trailing whitespace was normalized in upstream samples/doc comments and the
documentation dumper to satisfy the repository diff hygiene check; no runtime
logic is changed by that normalization.
