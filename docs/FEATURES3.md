# Features 3

Branch: `t3/custom-emoji-search-slowmode-folders`.

## Custom server emoji

- Server-scoped upload/list/rename/delete, Manage Emoji permission, bounded
  normalization and authenticated asset reads; animated GIF/WebP supported.
- Desktop settings, server picker section, shortcode completion/replacement,
  inline/jumbo rendering, reactions, hover-only animation with no idle timer.
- Intentionally leaves `crates/web` unchanged for the concurrent web agent.

Validation and remaining features are recorded after the final gate.

## Server-side message search

- SQLite FTS5, existing-message backfill, transactional edit/delete indexing,
  permission-filtered keyset paging, and all requested filters.
- Desktop right-hand results with term highlighting, older/newer paging, and
  around-message jumps through the existing bounded history loader.
