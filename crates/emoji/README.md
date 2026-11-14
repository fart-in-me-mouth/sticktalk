# sticktalk-emoji

Dependency-free at runtime; `serde_json` is build-only. No network access in
`build.rs`. Vendored inputs generate three byte tables: interned UTF-8 strings,
17-byte picker records and five-offset tone records. No runtime parsing, hash
maps, lazy initialization, background tasks or OS dependencies.

## Data and attribution

- Unicode Emoji **18.0**, `https://unicode.org/Public/emoji/latest/emoji-test.txt`,
  downloaded 2026-11-12 (file date 2026-04-30). Unicode License v3, copyright
  Unicode, Inc.; full license in `data/LICENSE-Unicode`.
- Discord-compatible alias vocabulary from Cal Henderson's MIT-licensed
  `https://github.com/iamcal/emoji-data` (`emoji.json`, downloaded 2026-11-12).
  Full copyright/license in `data/LICENSE-iamcal`.
- Exact vendored input digests: `data/SHA256SUMS`. Builds use these snapshots,
  not a moving latest URL. Refresh inputs, update the version constant, run
  tests and repeat the size measurement when upgrading.

1,923 unmodified picker entries across eight requested categories. Unicode
Smileys & Emotion + People & Body are combined. Names and added versions come
from Unicode. Keywords are English name tokens; aliases also participate in
search. This is not a full CLDR synonym/translation database. For additions
not present in the alias source, a deterministic snake_case name is used.

Skin-tone variants are kept out of picker search results. The five official
same-tone sequences are selected from Unicode's fully-qualified list (never
blindly appended to a ZWJ sequence). Mixed-tone couples, standalone modifier
components and non-RGI/minimally-qualified spellings are not exposed. Metadata
and reverse shortcode lookup refer to unmodified picker entries. Clients
persist the canonical bare shortcode and optionally a separate tone modifier.

## API

- `categories()`, `all()`, `by_category(category)` return allocation-free views.
- `Emoji`: `emoji()`, `name()`, `keywords()`, `shortcodes()`, `shortcode()`,
  `category()`, `version()`, `supports_skin_tone()`.
- `search(query, limit)`: case-insensitive English prefix/keyword AND matching;
  exact aliases rank first, followed by alias prefixes, name prefixes, keyword
  prefixes, with Unicode order ties. Empty query returns no results. Memory
  is proportional to returned limit, not dataset size.
- `shortcode_to_emoji()` accepts bare or colon-wrapped aliases; reverse
  `emoji_to_shortcode()` returns the canonical **bare** alias.
- `apply_skin_tone(glyph, modifier)` returns an official static sequence or None.
- `replace_shortcodes(text)` returns `Cow<str>`, borrowing unchanged text;
  unknown aliases remain intact. This intentionally does not interpret Markdown
  code fences or escapes: callers should exclude those spans if desired.
- `Usage` exposes persistable fields; `record(now, half_life)` increments the
  exponentially decayed counter. `rank_frequently_used()` sorts caller-owned
  entries in place. Caller supplies timestamps in consistent units and handles
  persistence. No serialization dependency or timer. Invalid weights are ignored;
  backwards clocks do not reverse decay. Persist aliases, not table indices.

## Reproducible size measurement

On stable Rust, workspace `release-small` (size optimization, fat LTO, stripped),
`wasm32-unknown-unknown`:

```
$env:CARGO_TARGET_DIR='$env:USERPROFILE/.cargo-target/sticktalk-emoji'
cargo build -p sticktalk-emoji --examples --target wasm32-unknown-unknown --profile release-small
```

Compare `size_probe.wasm` (all public operations reachable through an exported
function) with `size_baseline.wasm` (same exported function without the library):

- Static tables: **124,383 bytes**.
- Full wasm probe: **157,680 bytes**.
- Baseline: **15,236 bytes**.
- Incremental library contribution: **142,444 bytes** (139.1 KiB), below the
  150,000-byte contribution budget. Includes runtime search/replacement/ranking
  code and their marginal std support, not just table payloads. Final client
  contribution depends on compiler, LTO and which operations are reachable.

No wasm-bindgen, wasm-opt or network server is needed. This measures wasm
contribution, not native executable overhead or the `.rlib` archive size.
