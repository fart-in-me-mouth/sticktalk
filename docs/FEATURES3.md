# Features 3

Branch: `t3/custom-emoji-search-slowmode-folders`.

## Custom server emoji

- Server-scoped upload/list/rename/delete, Manage Emoji permission, bounded
  normalization and authenticated asset reads; animated GIF/WebP supported.
- Desktop settings, server picker section, shortcode completion/replacement,
  inline/jumbo rendering, reactions, hover-only animation with no idle timer.
- Web UI stays with the concurrent web agent (only its handshake opt-out changed).

## Server-side message search

- SQLite FTS5, existing-message backfill, transactional edit/delete indexing,
  permission-filtered keyset paging, and all requested filters.
- Desktop right-hand results with term highlighting, older/newer paging, and
  around-message jumps through the existing bounded history loader.

## Slowmode

- All requested intervals, persisted channel configuration, serialized send
  enforcement, Manage Messages/Channels bypass, and typed retry metadata.
- Desktop settings and monotonic, active-only composer countdown; edits stay
  available. Deleted messages cannot be used to evade the cooldown.

## Notifications and folders

- Independent, persisted desktop notification master switch overrides all
  server/channel levels without changing unread counts.
- Per-account relational server folders: context-menu add/remove, name/color,
  collapse/expand, rail mention aggregation, cross-device events and persistence.
- New text events have a separate optional handshake capability so older launch
  clients cannot receive unknown variants. The only web edit opts out of these
  events; web UI work stays with qa-web.

## Verification

- Native: 354 passed, 11 pre-existing opt-in hardware/long-running gates ignored.
  Includes a real loopback TCP test across three accounts for upload/asset ACLs,
  live emoji/reactions/rename, FTS edits/deletes, slowmode, and private folders.
- Web: 8 tests passed, plus all 53 shared-core tests rerun with web features;
  native and WASM32 check/strict Clippy passed.
- Client-core covers each feature, including stale responses, Unicode highlights,
  clock skew, notification persistence, folder moves and membership removal.
- Desktop regressions cover unknown/deleted tokens, cached preview load/eviction,
  bounded image decoding, keyboard completion, account-scoped results,
  and zero countdown subscriptions when hidden, read-only, unfocused or exempt.
- Captured and visually inspected all **26** headless PNGs (1280×800 and
  1920×1080); dimensions and SHA-256 verified against
  `crates/client/screenshots/features3/manifest.json`. No OS windows opened.

```powershell
$env:CARGO_TARGET_DIR = 'C:/Users/dwhee/.cargo-target/sticktalk-features3'
$env:CARGO_INCREMENTAL = '0'
cargo test -p sticktalk -p sticktalk-server -p sticktalk-client-core -p sticktalk-proto --no-fail-fast
cargo clippy -p sticktalk -p sticktalk-server -p sticktalk-client-core -p sticktalk-proto --all-targets -- -D warnings
cargo fmt --all -- --check
& "$env:CARGO_TARGET_DIR/debug/sticktalk.exe" --mock --screenshots crates/client/screenshots/features3 --features3-only
# After cleaning the native graph, reuse the same target directory:
cargo test -p sticktalk-web -p sticktalk-client-core
cargo clippy -p sticktalk-web -p sticktalk-client-core --all-targets -- -D warnings
cargo check -p sticktalk-web -p sticktalk-client-core --target wasm32-unknown-unknown
cargo clippy -p sticktalk-web -p sticktalk-client-core --target wasm32-unknown-unknown --all-targets -- -D warnings
```

## Scope and resource notes

- Folder context menus are the requested iced drag-and-drop fallback.
- Web feature UI is not implemented here to avoid the concurrently-owned web
  work. Shared DTOs/core are ready; its handshake explicitly declines new events.
- Idle working set/private bytes and production throughput were not measured.
  Timer absence and image/task bounds are tested, not claimed as CPU benchmarks.
- Duplicate Cargo feature graphs briefly exceeded the disk budget (~10.8 GiB).
  They were interrupted and cleaned; the native validation graph was ~8.52 GiB.
  Web/WASM validation used ~1.01 GiB after cleaning native artifacts. The single
  task target directory was deleted after final validation.
