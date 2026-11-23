# Web first load, idle memory and server stack depth (task `perf`)

Ten feature branches roughly doubled the web app's first load and idle
memory. This file records what the cost was, the structure that removes it,
the rules that keep a new feature out of the core, and the server's stack
depth work. Numbers are from headless Chromium on the owner's PC.

## Results

Before: the integration tree as handed over (`2abf70c`). After: this branch.
Both measured the same way on the owner's PC: release web build, headless
Chromium, a real local server. Memory and CPU come from `tests/idle.mjs`
(authenticated idle Friends screen, 60 s; private bytes of the whole Chromium
process tree minus a blank browser).

| | Recorded Nov 12 | Before | After | Target |
| --- | ---: | ---: | ---: | ---: |
| Core WASM, Brotli | 421,493 B | 993,709 B | **445,136 B** | 490,000 B |
| Core WASM, uncompressed | 1,797,367 B | 4,545,465 B | 1,472,931 B | |
| First load (installable shell inventory) | 580,049 B | 1,298,846 B | **597,314 B** | 650,000 B |
| First screen's JavaScript, Brotli | | 105,871 B | 52,498 B | |
| WASM loaded at idle | | 4,627,049 B in 10 modules | 1,757,276 B in 13 modules | |
| Script loaded at idle | | 383,680 B | 250,764 B | |
| Idle private memory over a blank browser | 36.0 MiB | 71.4 MiB | **51.4 MiB** | 45 MiB |
| Idle CPU, one logical core | 0.086% | 0.133% | 0.061% | not higher |

(The integration engineer measured 816,119 B / 1,113,142 B / 69.8 MiB / 0.092%
on an earlier combined tree; `2abf70c` added the QA branch's fixes.)

**Memory target not met.** Idle memory over a blank browser is about 20 MiB
lower, not 26. At idle the renderer holds 60 MiB (blank: 18), the browser
process 24 MiB (blank: 17; headless test contexts keep the HTTP and service
worker caches in memory) and the JS heap 3.3 MiB. Below about 2 MB of WASM the
renderer no longer shrinks in proportion: removing the last 0.35 MB at idle
(the command encoder, the feature pages' fetches) saved under 1 MiB. What is
left is mostly the core's own compiled code and the page (DOM, styles, fonts).
Getting the last 5 MiB would need the core itself well under 1 MB: the first
screen's views (235 KB of Leptos view code) and the model's collection code
(361 KB of generic instantiations) are the bulk, and both would need a
restructuring of the views (shared non-generic render helpers) that is larger
than this task.

### First-open latency

`tests/perf.mjs`: each open is the first in a fresh browser context (cold HTTP
and module caches), local server, median of 3. "Press": pressed as soon as the
pointer arrives. "Intent": the pointer rests 150 ms first (prefetch time).

| Feature | Before, press | After, press | Before, intent | After, intent |
| --- | ---: | ---: | ---: | ---: |
| User Settings | 111 ms | 79 ms | 90 ms | 67 ms |
| Emoji picker | 1,114 ms | 820 ms | 1,019 ms | 1,208 ms |
| Inbox | 87 ms | 92 ms | 81 ms | 69 ms |
| Profile card | 98 ms | 97 ms | 73 ms | 72 ms |
| Server menu | 38 ms | 84 ms | 19 ms | 19 ms |

- The server menu now downloads its behavior on first use: a press with no
  hover before it waits for it (+46 ms locally); with the usual hover first
  it is as before.
- The emoji picker takes about a second in both builds: it renders some 400
  emoji. Its faster "press" time after is not a real gain: the first frame
  measured there is drawn before the expression module arrives. With intent
  prefetch it is about 0.2 s slower than before (measured: without the
  prefetch 1,020 ms, the same as before); the prefetched picker draws its full
  first frame at once. Making the picker render fewer emoji up front
  (virtualize the grid) would fix both; not done here.

## Where the core's bytes went

`twiggy` on the split core (names kept with `wasm-opt -g`; the name section is
left out, so the totals are the production sizes):

| Core, by kind (uncompressed bytes) | Before | After |
| --- | ---: | ---: |
| Decoders: MessagePack (rmp_serde) | 1,724,764 | 0 |
| App logic (web) | 564,198 | 339,478 |
| Decoders: serde buffered path | 508,704 | 8,890 |
| Generic instantiations (collections, sort, drop, clone) | 438,302 | 361,042 |
| Data (strings, tables) | 372,567 | 127,150 |
| Decoders: JSON (serde_json) | 302,785 | 172,161 |
| View code (Leptos/tachys) | 254,558 | 234,761 |
| Other | 96,332 | 77,509 |
| Model (client-core) | 89,627 | 57,400 |
| std | 57,450 | 47,813 |
| Encoders | 53,835 | 5,201 |
| URL parsing (url, idna) | 32,152 | 0 |
| std/fmt | 19,295 | 18,234 |
| JS bindings | 17,076 | 17,384 |
| Decoders: other | 8,490 | 5,908 |
| **Total** | **4,540,135** | **1,472,931** |

| Core, by feature (uncompressed bytes) | Before | After |
| --- | ---: | ---: |
| core (messages, channels, servers, socket, shell) | 2,727,425 | 1,245,446 |
| privacy | 325,148 | 5,675 |
| threads | 307,487 | 16,727 |
| content | 275,106 | 39,405 |
| moderation | 251,360 | 23,684 |
| expression | 220,774 | 29,463 |
| nav | 148,483 | 15,061 |
| launch | 107,176 | 60,741 |
| profiles | 105,870 | 14,172 |
| features3 | 36,147 | 10,952 |
| gifs | 26,207 | 9,786 |
| authsec | 6,795 | 240 |
| calls | 1,449 | 1,552 |
| emoji tables | 681 | 0 |
| callextras | 27 | 27 |

Feature attribution is by the type or module a function belongs to; shared
types (messages, users, channels) count as core. Every lazily loaded module
after (uncompressed; Brotli sizes and budgets are in `crates/web/scripts/budgets.json`).
`chunk_*` are code shared by several modules; `chunk_33` is the emoji tables,
loaded with the pickers and `:` completion:

| Lazy module (after) | Bytes | Decoders | Logic | Views | Data | Generic/std/other |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| chunk_32 | 8,491 | 6,571 | 1,005 | 0 | 272 | 643 |
| chunk_33 | 122,315 | 0 | 112 | 0 | 118,107 | 4,096 |
| chunk_34 | 12,328 | 0 | 4,364 | 2,405 | 647 | 4,912 |
| chunk_35 | 9,666 | 28 | 257 | 815 | 2,298 | 6,268 |
| chunk_36 | 38,930 | 22,492 | 13,544 | 0 | 1,183 | 1,711 |
| chunk_37 | 28,271 | 17,053 | 8,237 | 0 | 942 | 2,039 |
| chunk_38 | 24,662 | 19,531 | 1,912 | 0 | 652 | 2,567 |
| chunk_41 | 13,383 | 11,408 | 379 | 0 | 438 | 1,158 |
| chunk_42 | 24,352 | 9,080 | 7,332 | 0 | 567 | 7,373 |
| chunk_43 | 16,691 | 0 | 5,385 | 0 | 309 | 10,997 |
| chunk_44 | 16,601 | 15,037 | 241 | 0 | 247 | 1,076 |
| split_content | 37,119 | 2,580 | 14,784 | 30 | 1,860 | 17,865 |
| split_content_state | 25,191 | 5,140 | 3,353 | 0 | 774 | 15,924 |
| split_dialog_data | 44,123 | 6,425 | 31,932 | 0 | 989 | 4,777 |
| split_expression | 21,184 | 2,191 | 10,434 | 16 | 1,468 | 7,075 |
| split_expression_state | 60,183 | 11,792 | 21,941 | 0 | 1,377 | 25,073 |
| split_forward_voice | 158,397 | 71,001 | 1,238 | 0 | 2,876 | 83,282 |
| split_load_call | 18,329 | 0 | 7,156 | 4,572 | 1,844 | 4,757 |
| split_load_dialog | 932,301 | 31,661 | 300,136 | 203,786 | 68,976 | 327,742 |
| split_load_forum | 28,104 | 0 | 11,180 | 3,980 | 2,322 | 10,622 |
| split_load_message_encoder | 65,711 | 0 | 2,335 | 0 | 1,637 | 61,739 |
| split_load_search | 8,359 | 0 | 2,437 | 3,054 | 1,463 | 1,405 |
| split_moderation | 32,796 | 11,741 | 8,801 | 16 | 1,754 | 10,484 |
| split_moderation_state | 57,028 | 27,520 | 883 | 0 | 1,294 | 27,331 |
| split_nav | 56,394 | 6,101 | 44,200 | 16 | 2,988 | 3,089 |
| split_privacy | 45,301 | 16,524 | 16,907 | 16 | 1,581 | 10,273 |
| split_privacy_state | 40,564 | 21,656 | 3,660 | 0 | 1,474 | 13,774 |
| split_profiles | 31,729 | 11,025 | 13,735 | 16 | 969 | 5,984 |
| split_response | 9,779 | 0 | 6,684 | 12 | 2,038 | 1,045 |
| split_submit | 20,349 | 0 | 4,020 | 16 | 601 | 15,712 |
| split_threads | 39,659 | 5,201 | 18,809 | 16 | 1,982 | 13,651 |
| split_threads_state | 24,332 | 1,117 | 1,306 | 0 | 523 | 21,386 |
| split_upload | 22,953 | 1,814 | 14,473 | 0 | 1,433 | 5,233 |
| split_voice_action | 90,306 | 51,922 | 871 | 0 | 1,950 | 35,563 |


The diagnosis held, and was larger than estimated: 60% of the core was
decoding. Three causes:

1. **Every event and command result decoded in the core.** The socket decoded
   each frame into `ServerMsg`, whose derived decoder contains every feature's
   events and results (all nested types), so all of it was reachable from the
   core.
2. **Adjacently tagged enums are decoded twice.** For `#[serde(tag, content)]`
   enums serde also generates a "content before tag" path that buffers the
   value and decodes every variant again from the buffer (`ContentDeserializer`,
   0.5 MB). The server always writes the tag first; the path never runs.
3. **rmp_serde's derived decoders are 3 to 6 times larger than serde_json's**
   for the same type (a two-field struct: 8.6 KB against 1.4 KB), and every
   type the app also reads over REST had both.

The feature controllers (their actions, forms, REST calls and JSON decoders)
were reachable from the core's action router, and the emoji tables were in the
core's data.

## The structure

### 1. Frames are routed, not decoded (`live/inbound.rs`)

A small MessagePack reader walks a frame without decoding it: it finds the
`type` tags and the byte ranges of `data` values. The core decodes only the
payloads of its own events (`routes!` table), each with its own type. A
feature's event or command result is passed on as the undecoded bytes of its
`data` value (`Routed::Feature(Feature, Raw)`). The wire format is unchanged.

### 2. One decoder per type (`inbound::de`)

`de::<T>(bytes)` rewrites the MessagePack value as JSON text and reads it with
`serde_json::from_slice`, the same decoder instantiation the REST client uses.
A type has one decoder in the whole app, shared by the socket and REST, and
serde_json's are much smaller. Integer map keys become decimal strings and
byte strings become arrays of numbers, which is how serde reads them back;
nesting is limited to 32 levels as before. Cost: one extra pass over each
frame (microseconds for an event; about a millisecond for a large Ready).

### 3. One split entry per feature (`live/features.rs`)

Each feature module (`content`, `threads`, `expression`, `moderation`,
`privacy`, `nav`, `profiles`, `authsec`) has exactly one split function,
`<feature>::run(app, Call)`, declared with
`#[wasm_split(<feature>, fallible, preload(prefetch))]`. Its controller, REST
decoders and event/result decoders are reachable only from there, so the
splitter puts them in `split_<feature>.wasm`. The core talks to a feature only
through `Call`s: `Action`, `Submit`, `Opened`, `Ready`, `Event(Raw)`,
`Answer(op, Reply)`, `Other(name, arg)`.

- `features::spawn` runs a call synchronously when the module is loaded (a
  loaded module answers on the first poll), otherwise as soon as it arrives.
- `features::run(...).await` is used by the socket, which waits for a feature's
  event (and `operations::response` for a feature's answer) before reading the
  next frame.
- A download failure is reported ("That didn't load...") instead of aborting
  the WASM instance, which a failed `#[lazy]` load does.

### 4. Events stay ordered and correct across reconnects

- **No buffer is needed.** The socket awaits the feature's module before it
  reads the next frame, so every event and answer is applied in the server's
  order, even during the first download. Frames that arrive meanwhile wait in
  the WebSocket's own queue.
- **The cursor only moves when an event is applied** (`Model::advance` for a
  feature's event, which the model does not keep). If the module cannot be
  downloaded the socket reconnects; the resumed stream sends the event again,
  and a replayed event (seq not newer than the cursor) is skipped, as before.
- **Resync** (`ResyncRequired`, or a fresh Ready) resets everything exactly as
  before: features refetch on `Call::Ready`.
- Feature events are not journaled (the journal is only replayed for the
  core's message and metadata events, which ignore them).

### 5. Views and scripts

- Views that are not on the first screen are `ui::LazyView`s (the forum view,
  search results), like the dialog and call views already were.
- The UI SDK's dialog, menu and tooltip are defined on first use (`ui.js`):
  pointing at, focusing or pressing one downloads it; a press that arrives
  first is held and replayed. Avatars stay eager. Their content is rendered
  `hidden`, and icon buttons carry their own `aria-label`, so nothing shows or
  loses its name before then.
- Every other feature script was already gesture-loaded; the service worker
  now precaches only what the first screen imports statically (see "Scripts
  and styles" below).
- The web build uses the browser's URL parser (`client-core/src/urls.rs`)
  instead of the `url` crate and its IDNA tables.
- wasm-ld runs with `-O1`: identical strings are still merged, but strings are
  no longer tail-merged, which glued strings of different modules into ranges
  the splitter had to keep in the core.

### 6. Prefetch on intent

Pointing at or focusing any control sends `prefetch-for:<action>` once per
kind of action (`public/live.js`). The app maps the action to the module it
will need with the same rules as the action router (`operations::prefetch_for`)
and downloads it with the feature's `prefetch()`. Never on a metered,
data-saver or slower-than-4G connection (`features::fast_connection`). There
is no idle-time prefetch: it would put every feature in idle memory.

## Checklist: keeping a new feature out of the core

**An event.** Add the variant to `Event` in `proto` as usual. Then:

1. `inbound.rs`, `routes! { route_event ... features { "<tag>" => <Feature>, } }`.
   (A core event goes in `newtype`/`fields`/`unit` instead.)
2. `inbound::owner`: add the variant (the match is exhaustive, so this does not
   compile until it is classified).
3. In your feature's `run`: `Call::Event(raw) => { let Ok(e) = inbound::de::<YourEvent>(&raw.data) else { return Ok(false) }; event(&mut app.borrow_mut(), e); }`.
   Use `raw.tag` if the feature has several `Event` variants (see `nav::run`).

**A command result.** `routes! { route_result ... features { "<tag>" => <Feature>, } }`,
and in `run`: `Call::Answer(op, Reply::Raw(raw)) => inbound::de::<YourResult>(&raw.data)`.
`operations::response` hands a result to the feature it is routed to (or to a
feature that `owns` the command: moderation, expression, profiles, privacy).

**A new feature module.** Add a `Feature` variant and its two lines in
`features.rs` (`dispatch`, `prefetch`) and `from_name`. Give the module one
entry point:

```rust
#[leptos::wasm_split::wasm_split(<name>, wasm_split_path = ::leptos::wasm_split, fallible, preload(prefetch))]
pub(super) fn run(app: Shared, call: features::Call) -> features::Outcome { ... }
```

and route its actions and forms in `operations::action` with
`feature(app, Feature::<Name>, Call::Action(kind.into(), arg.into()))` /
`Call::Submit(form)`, and its dialogs in `open_dialog` with an `owns_dialog`
predicate. Add the same rules to `operations::prefetch_for`.

**A view.** If it is not on the first screen, render it through a
`ui::LazyView` (`LazyViews`, `lazy_view`, a `#[lazy] fn load_<view>()`), or put
it in a dialog. A view on the first screen that only shows feature state (a
badge, a row) may stay in the core: keep it small and keep its data decoded by
the feature.

**What stays in the core.** Only small, synchronous helpers the core calls
while rendering or sending (`threads::thread`, `content::draft_of`,
`privacy::muted`, `moderation::gated`, ...). Anything that fetches, decodes or
builds a form belongs behind `run`.

**A script.** Import it from the code that uses it (`import()` on the gesture),
never from `live.js` or `ui.js` at the top level. If it is not imported at
startup, add it to the service worker's on-demand list in
`scripts/finalize.mjs`, and to `modules` in `scripts/version-runtime.mjs`.

**A stylesheet.** Styles are one Tailwind build of `styles/app.css`: use
utility classes in the views. Large feature-only art (cosmetics, stickers,
camera backgrounds) is fetched by URL when shown, never inlined.

**Budgets.** `npm run test:sizes` (also run by `npm run audit`) checks the core,
the first load, every split module, the shared chunks and the first screen's
JavaScript against `scripts/budgets.json`. A new split module needs a budget
entry; raise a budget only with the measured cost in the commit message.

## Worked example: content (polls, scheduled messages, GIFs)

Before, the core matched `Event::Content(e) => content::event(&mut a, e.clone())`
and `CommandResult::Content(r) => content::answered(...)`, and routed actions
straight to `content::action`. All of content's code and decoders were in the
core. After:

- `inbound.rs`: `features { "content" => Content, }` in both `routes!` tables.
- `operations::action`: `kind if content::ACTIONS.contains(&kind) => feature(app, Feature::Content, Call::Action(kind.into(), arg.into()))`,
  and `Call::Submit(form)` for `content::FORMS`.
- `open_dialog`: `content::owns_dialog(name)` decides whether a dialog opening
  loads content (`poll`, `forward:*`, `gif`).
- `content::run` (end of `live/content.rs`) maps each `Call` to the existing
  functions, decoding events with `inbound::de::<ContentEvent>` and results
  with `inbound::de::<ContentResult>`.
- What stays in the core: `draft_of`, `send_command` and `uploaded` (the send
  path uses them on every message), and the message views that draw polls and
  embeds.

## Scripts and styles

- The service worker precaches only what the first screen loads: the core,
  its glue, `live.js` and its static imports, the UI SDK modules those import
  (the avatar; dialog, menu and tooltip load on first use), the stylesheets,
  the three Inter weights and the 192 px icon. Lazily imported scripts
  (sign-in extras, account security, call sounds, alerts), install-time icons
  and the monospace font are cached when first used (`scripts/finalize.mjs`).
- The WASM glue's TypeScript declarations are no longer shipped.
- Audited: calls, clips, camera backgrounds, soundboard, QR scanning (jsQR),
  passkeys, the theme editor, profile cosmetics and forum views load only
  when used. `alerts.js` and `calls.js` (3 KB each) are imported by the WASM
  glue and load with it.
- The splitter now merges its tiny shared chunks (`--cost-model binary`, a
  request weighed at 16 KiB): 190 modules became 45, 13 at idle. Each module
  is a request and a WebAssembly instance; with ~150 tiny ones at idle the
  page also filled the browser's 250-entry resource timing buffer.

## Server stack depth

A command runs as a spawned task: its future is on the heap, and each poll
nests one call per level of `async fn`. In a debug build each level's frame
holds every temporary of the whole function (no stack slot reuse), so a large
function costs its full frame every time it is on the path. `commands::mutate`
(a 1,446-line match over every core command) was on the forum-post path twice:
the post's first message goes back through it.

Measured with `stack_tests::measure`: the smallest thread stack each path
runs in, found by bisection in child processes. Windows reserves stacks in
64 KiB steps, so a figure means "fits in that, rounded up to a 64 KiB step";
the release numbers are therefore ranges. Release is the `server` profile.

| Path | Debug before | Debug after | Release before | Release after |
| --- | ---: | ---: | ---: | ---: |
| Create a forum post | 1,414 KiB | **903 KiB** | 129–192 KiB | 65–128 KiB |
| Publish an announcement + crosspost delivery | 839 KiB | 391 KiB | 65–128 KiB | 65–128 KiB |
| Send a message with mentions | 775 KiB | 775 KiB | 65–128 KiB | 65–128 KiB |
| Scheduled message delivery (timer) | 711 KiB | 711 KiB | 65–128 KiB | 65–128 KiB |
| Opening the server (migrations, timers) | 263 KiB | 263 KiB | 65–128 KiB | 65–128 KiB |

Changes:

- `commands::mutate` dispatches the feature commands and boxes everything
  else into `core_mutate`, so the large frame is entered once per command,
  not once per nesting level. The forum's calls into the send path and into
  `forum::create_post` are boxed too.
- The server's startup and accept loop are polled on a thread whose stack the
  server chooses (`sticktalk-main`, 4 MiB), not on the main thread (1 MiB on
  Windows). Worker and blocking threads: 4 MiB (was 8): twice the debug
  budget, address space only.
- `RUST_MIN_STACK = 8 MiB` is gone from `.cargo/config.toml`: every server
  test passes with 2 MiB test threads.
- Regression test `stack_tests::deepest_paths_fit_the_stack_budget` runs each
  path in a child process on a thread of the budget (1 MiB optimized, 2 MiB
  debug); an overflow fails that test instead of crashing a server.
  `cargo test -p sticktalk-server stack_tests::measure -- --ignored --nocapture`
  prints the numbers above. New deep paths belong in its list.

Threads: tokio workers and blocking threads (4 MiB), `sticktalk-main`
(4 MiB), sqlx's SQLite connection threads (they run SQLite, not command
futures), and the export builder's `spawn_blocking` + `block_on` (a blocking
thread, 4 MiB).
