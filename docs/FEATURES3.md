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
