# Web first load, idle memory and server stack depth (tasks `perf`, `lazy`, `lazy2`)

Ten feature branches roughly doubled the web app's first load and idle
memory. This file records what the cost was, the structure that removes it,
the rules that keep a new feature out of the core, and the server's stack
depth work. Numbers are from headless Chromium on the owner's PC.

**Feature engineers converting a branch:** read "The structure", then follow
"Converting a feature branch" below step by step.
The latest additions to the method (task `lazy2`: per-message and per-user
extras kept undecoded with `Deferred`, composer paths as their own splits,
lazily loaded feature stylesheets, the service worker's size) are in the
next section.

## Activities, apps and identity (task `lazy2`)

Three more eager-style branches (shared Activities, apps/bots/webhooks,
server identity) were merged from the staging branch and converted. Before:
the staging branch as found (`2ed7b7c`). After: this branch. Same method as
below (release web build, headless Chromium, a real local server, one server
binary for both: this branch's `--profile server`).

| | Before (`2ed7b7c`) | After | Target |
| --- | ---: | ---: | ---: |
| Core WASM, Brotli | 1,348,192 B | **487,007 B** | 490,000 B |
| First load (installable shell inventory) | 1,685,192 B | **649,871 B** | 650,000 B |
| First screen's JavaScript, Brotli | 113,533 B | 60,692 B | |
| WASM loaded at idle | 7,538,008 B in 26 modules | 1,912,939 B in 13 modules | |
| Script loaded at idle | 431,153 B | 367,547 B | |
| Idle private memory over a blank browser | 97.4 MiB | **55.5 MiB** | 45 MiB |
| - renderer / browser process | 75.3 / 18.1 MiB | 46.0 / 7.0 MiB | |
| Idle CPU, one logical core | 0.061% | 0.061% | not higher |

Against the integration tree before this merge (`0cbace0`: core 471,007 B,
first load 637,224 B, 51.9 MiB) the three features cost 16 KB of core,
12.6 KB of first load and 3.6 MiB at idle. The first load is 129 B under its
target: the next feature has to pay for itself (see "Finding where the
bytes are"). What is left of them in the core is
what every name, row and message draws: the APP label, the server tag chip,
a role's gradient and icon, the sidebar banner and animated rail icon, the
invite and activity cards' placeholders, and the `/` hook in the composer.

### First-open latency (`tests/perf.mjs`, median of 3)

| Feature | Before, press | After, press | Before, intent | After, intent |
| --- | ---: | ---: | ---: | ---: |
| `/` command picker | 97 ms | 97 ms | 85 ms | 84 ms |
| Activities launcher (in a call) | 64 ms | 65 ms | 54 ms | 66 ms |
| Server tag card | 119 ms | 100 ms | 98 ms | 67 ms |
| Role editor (style and icon controls) | 69 ms | 80 ms | 41 ms | 63 ms |
| User Settings | 105 ms | 77 ms | 99 ms | 56 ms |
| Emoji picker | 1,021 ms | 74 ms | 1,101 ms | 150 ms |
| Inbox | 100 ms | 99 ms | 97 ms | 74 ms |
| Profile card | 109 ms | 90 ms | 97 ms | 71 ms |
| Server menu | 39 ms | 39 ms | 17 ms | 17 ms |

- The `/` picker loads two small modules (`compose_lazy` 7.5 KB, its view
  `load_picker` 2 KB), not the apps feature (20 KB) or its pages (62 KB); the
  built-in commands are a constant in it. Typing `/` in a server also fetches
  that channel's command list.
- The role editor's style and icon controls are in the identity view module
  (24 KB): it is fetched when the Roles page opens and on pointing at a role,
  so the editor does not wait for it (without that: 87 ms press, 69 ms
  intent). The 11-22 ms still left is rendering, not a download: no request
  follows the press. Not chased further.
- The launcher's time is `activities.js` and its stylesheet, both loaded on
  the press; the activity's own code, SDK and shell load only when one is
  launched or joined.

### What was done (beyond "Converting a feature branch")

- **Composer paths as small splits.** The apps feature's `run` module (pages,
  forms) is not what the composer waits for: the picker, sending a command
  (`run_slash`) and a channel's command list (`load_catalog`) are `#[lazy]`
  functions of their own. A feature's latency-critical path can be its own
  split; the doc's state/run split is the default, not the limit.
- **Apps' answers go to `run`** (`features::dispatch`): they answer what its
  pages and the picker sent, so the module is already loaded; its 44 KB
  event module (`apps_state`) loads only for an app event.
- **Identity is one module** (`identity`, no `state`): it has no events or
  results of its own; its answers are core Acks, dispatched to `run`.
- **Activities' host script loads only when needed.** Someone else's session
  is drawn by the call view's own banner; `activities.js` loads when this
  client opens the launcher, joins or takes part (`activities::wake_host`
  then hands it the session it missed).
- **`Deferred<T>` in proto (wasm32 only).** A large type that rides on every
  message, user, server or role puts its decoder in the core even when only
  one feature draws it. In the web build these fields are
  `sticktalk_proto::Deferred<T>` (the JSON, decoded on first use by the
  feature that draws it); the wire and the native types are unchanged:
  `MessageExtra.app` and `.activity`, `UserStyle.tag`, `Guild.look`,
  `Role.look`, and the `Event::Apps` / `CommandResult::Apps` payloads (the
  core never builds those variants, but their clone and drop code was in it).
  A hot-path renderer reads the few fields it draws from `raw()`
  (`ui::identity::worn_tag`, `role_style`, `guild_image`; `ui::apps::author`)
  instead of calling `get()`, which would bring the decoder back. Saved about
  10 KB Brotli of core.
- **Feature stylesheets can load with the feature.** `styles/activities.css`
  is built on its own (`@reference "./app.css"`, so `@apply` and the theme
  work; `scripts/styles.mjs` `LAZY_STYLES`) into `public/activities.css`,
  which `activities.js` links on import. Only for styles nothing on the first
  screen uses.
- **The service worker is part of the first load, uncompressed.** It lists
  every asset; the lists are now grouped by directory (17.3 KB to 9 KB), and
  an activity frame's files are left out (an opaque-origin frame is never
  controlled by the worker).
- **Precache exclusions must match the fingerprinted path.** Feature scripts
  are served from `/public/runtime/<hash>/x.js`: an exclusion written as
  `/public/x.js` does not match it (`activities.js`, 10 KB, was precached).

### Finding where the bytes are

To see exactly what a merge added to the core, build the core of the tree
before it too and diff the two `twiggy` listings by function name (hashes in
symbol names removed): `git archive <commit>` into a scratch directory,
`cargo build -p sticktalk-web --target wasm32-unknown-unknown --profile
web-release` there with the same target directory, then the names-split
commands of "Measuring" below for each. The "function names" subsection is
not shipped; group the rest by crate and module. Here the growth was 74 KB
uncompressed: 42 KB of the features' own code and decoders, 12 KB of view
code (each extra child in a `view!` changes the whole element's type: give
several hooks one slot, as `ui::feature_extras` and `identity::row_after` do),
5 KB of `serde_json::Value`, the rest data and tables. For the first load,
compare `sw.js`'s shell list sizes the same way (`firstLoadBytes` counts
compressed files and the worker as is).

### Server stack depth (debug build, `stack_tests::measure`)

| Path | Combined tree (`lazy`) | This branch |
| --- | ---: | ---: |
| A bot answers a command with embeds and buttons | | 1,158 KiB |
| Create a forum post | 1,031 KiB | 1,094 KiB |
| Share an event into a channel | 1,031 KiB | 1,094 KiB |
| Send a `/tts` message | 903 KiB | 1,031 KiB |
| Send a message with mentions | 839 KiB | 903 KiB |
| Scheduled message delivery (timer) | 775 KiB | 839 KiB |
| Publish an announcement + crosspost delivery | 455 KiB | 519 KiB |
| Build a server from a template | 455 KiB | 519 KiB |
| A server tag edit told to 400 wearers (fan-out) | | 455 KiB |
| An activity launched, its invite card posted | | 391 KiB |
| Opening the server (migrations, timers) | 271 KiB | 327 KiB |
| An event starts (timer) | 199 KiB | 263 KiB |

The send path grew one 64 KiB step (the merged features' hooks on it:
`apps::gate`, `apps::settle`, the bots' event filter). The deepest path is a
bot's answer (`Respond` → `commands::mutate(SendMessage)` through the boxed
apps module): 1,158 KiB of the 2 MiB debug budget. All server tests pass with
the default 2 MiB test stack.


## The combined tree (task `lazy`)

The integration branch merged three more features in the eager style while
task `perf` was done (community; Stage channels and scheduled events; themes,
accessibility, text-to-speech, Streamer Mode, app icons). Task `lazy` merged
the two lines and converted the three features ("Converting a feature
branch" below records how, for the branches still in progress).

Before: the integration tree as found (`8db8f6f`). After: this branch. Same
method as below: release web build, headless Chromium, a real local server,
one server binary for both; memory and CPU from `tests/idle.mjs` (60 s on the
authenticated Friends screen, private bytes of the Chromium tree minus a
blank browser); latency from `tests/perf.mjs` (median of 3, each in a fresh
browser context).

| | Before (`8db8f6f`) | After | Target |
| --- | ---: | ---: | ---: |
| Core WASM, Brotli | 1,185,435 B | **471,007 B** | 490,000 B |
| First load (installable shell inventory) | 1,504,834 B | **637,224 B** | 650,000 B |
| First screen's JavaScript, Brotli | 112,244 B | 59,757 B | |
| WASM loaded at idle | 6,313,420 B in 14 modules | 1,837,393 B in 12 modules | |
| Script loaded at idle | 411,760 B | 356,898 B | |
| Idle private memory over a blank browser | 85.4 MiB | **51.9 MiB** | 45 MiB |
| Idle CPU, one logical core | 0.064% | 0.063% | not higher |

The integration tree also loaded the 546 KB voice module at idle: the empty
Stage list sent after every Ready went through it. It now goes to the stage
feature, and only when it is not empty.

### First-open latency (combined tree)

| Feature | Before, press | After, press | Before, intent | After, intent |
| --- | ---: | ---: | ---: | ---: |
| User Settings | 103 ms | 74 ms | 113 ms | 61 ms |
| Emoji picker | 1,068 ms | **69 ms** | 1,023 ms | **164 ms** |
| Inbox | 93 ms | 90 ms | 82 ms | 72 ms |
| Profile card | 116 ms | 95 ms | 77 ms | 77 ms |
| Server menu | 39 ms | **41 ms** | 17 ms | 18 ms |

The 150 ms goal for the picker is met on a press (all three samples 69-72
ms) and missed by about 15 ms with the pointer resting on the button first
(161-164 ms): the first frame of 128 color emoji costs about 100 ms of main
thread in headless Chromium's software rendering. Drawing fewer up front
would meet it, at the cost of a second render as soon as the sentinel is in
view; not done.

- **Server menu:** the menu's behavior (UI SDK `menu.js`) is defined when
  the browser is first idle after a menu is in the page (`public/ui.js`), so
  a press with no hover first no longer waits for it (task `perf` measured
  84 ms against 38 ms before; now as before).
- **Emoji picker:** it drew a whole category (up to 400 emoji) and, after
  every render, checked every one with a canvas probe (`canDraw` in
  `public/live.js`): about 1.5 ms per color emoji in software, plus a one-off
  200 ms font search for the "missing glyph" reference. Now it draws the
  first 128 (sixteen rows: what the picker shows and two rows more), and more
  as the end comes within two rows of view (an `IntersectionObserver` on a
  sentinel, `expr-more`; no scroll handler). Category buttons and search are
  unchanged and reset the window. The glyph checks run when the browser is
  idle, eight at a time, the reference alone; emoji already checked are
  hidden at once. And `dialog_data` no longer draws the whole app a second
  time when a dialog has nothing to fetch (it did for the picker, ~170 ms).

### Idle memory: where it goes

Measured on the combined tree (`idle.mjs` now reports these):

| Part (over a blank browser) | Before | After |
| --- | ---: | ---: |
| Renderer process | +65.5 MiB | +43.6 MiB |
| - JS heap (used) | 4.1 MiB | 3.8 MiB |
| - WASM linear memory (its size is the high-water mark) | | 1.6 MiB |
| - DOM | 1,195 nodes | 1,195 nodes, 251 listeners |
| - decoded images | | 36,864 px (the 192 px logo) |
| - fonts | not measured | Inter ×3 (JetBrains Mono was loaded too, see below) |
| - the rest: compiled code of the loaded WASM and scripts, styles, Blink | | about 38 MiB |
| Browser process (headless contexts keep the HTTP and worker caches in memory) | +15.8 MiB | +5.8 MiB |
| Network service | +3.8 MiB | +2.4 MiB |
| GPU | +0.2 MiB | +0.1 MiB |

- Linear memory is 1.6 MiB and never grew past it on this screen: decoding
  the Ready (MessagePack rewritten as JSON, then decoded) peaks well below
  it for a normal account, so there is no startup peak to avoid here. (For a
  large account the transient JSON copy is about 1.5x the Ready frame.)
- Taken: no voice module at idle (above); the "Ctrl K" hint on the sidebar
  no longer pulls the 28 KB JetBrains Mono font (it is a shortcut label, now
  in Inter); device preferences reach Rust without a JSON decoder.
- Not cheap: what is left is mostly the compiled code of 1.8 MB of WASM in 12
  modules, the first screen's scripts and the one stylesheet. The 45 MiB
  aspiration needs fewer or smaller idle modules: the state modules loaded
  at sign-in by the features' Ready handling (expression 59 KB, privacy
  41 KB, content 25 KB, threads 24 KB, nav 4 KB, uncompressed). Loading each
  when its first surface appears, instead of at Ready, is the next step; it
  changes when those features subscribe and fetch, so it was left for a task
  of its own.

## Results of task `perf`

Before: the integration tree as handed over (`2abf70c`). After: that branch.
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

### 1. Frames are routed, not decoded (`crates/web/src/inbound.rs`)

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

### 3. Two split entries per feature (`live/features.rs`)

Each feature module (`content`, `threads`, `expression`, `moderation`,
`privacy`, `nav`; `profiles` and `authsec` have only `run`) has two split
functions:

- `<feature>::state(app, Call)`, `#[wasm_split(<feature>_state, fallible)]`:
  its events, the answers to its commands and the fresh snapshot (Ready). The
  first screen loads only these, for the features it shows.
- `<feature>::run(app, Call)`, `#[wasm_split(<feature>, fallible, preload(prefetch))]`:
  its actions, forms, dialogs and page fetches. A page refresh an event asks
  for goes here too (`features::defer(.., Call::Other(..))`), so the state
  module does not carry page code.

Its controller, REST decoders and event/result decoders are reachable only
from these, so the splitter puts them in `split_<feature>[_state].wasm`. The
core talks to a feature only through `Call`s: `Action`, `Submit`, `Opened`,
`Ready`, `Event(Raw)`, `Answer(op, Reply)`, `Other(name, arg)`;
`features::dispatch` sends `Event`, `Ready` and `Answer` to `state`.

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
- The UI SDK's dialog and tooltip are defined on first use (`ui.js`), the menu
  when the browser is idle after a menu is first in the page (task `lazy`:
  a press on the server menu with no hover first waited ~46 ms for it):
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
is no idle-time prefetch of feature modules: it would put every feature in
idle memory. The one exception is the UI SDK's menu behavior (a few KB,
used on every server screen; skipped with Data Saver).

## Checklist: keeping a new feature out of the core

**An event.** Add the variant to `Event` in `proto` as usual. Then:

1. `inbound.rs`, `routes! { route_event ... features { "<tag>" => <Feature>, } }`.
   (A core event goes in `newtype`/`fields`/`unit` instead.)
2. `inbound::owner`: add the variant (the match is exhaustive, so this does not
   compile until it is classified). The test `every_variant_is_listed_with_its_serde_tag`
   compares the tables with serde's own variant names.
3. Decode it in your feature's `state`: list your event enum's variants in
   `inbound::feature` with `decode_tagged!` (variant by variant, without
   serde's buffered second decoder), then
   `Call::Event(raw) => { let Ok(e) = crate::inbound::feature::your_event(&raw.data) else { return Ok(false) }; event(&mut app.borrow_mut(), e); }`.
   Use `raw.tag` if the feature has several `Event` variants (see `nav::state`).

**A command result.** `routes! { route_result ... features { "<tag>" => <Feature>, } }`,
and in `state`: `Call::Answer(op, Reply::Raw(raw)) => crate::inbound::feature::your_result(&raw.data)`.
`operations::response` hands a result to the feature it is routed to (or to a
feature that `owns` the command: moderation, expression, profiles, privacy).

**A new feature module.** Add a `Feature` variant (`inbound.rs`) and its lines
in `features.rs` (`dispatch`, `prefetch`, `from_name`). Give the module two
entry points, `state` (events, answers, Ready; `wasm_split(<name>_state, ...)`)
and `run` (everything a person does):

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
- `content::state` (end of `live/content.rs`) handles `Ready`, events
  (`inbound::feature::content_event`) and answers
  (`inbound::feature::content_result`); `content::run` handles actions, forms
  and dialogs. Each maps a `Call` to the existing functions.
- What stays in the core: `draft_of`, `send_command` and `uploaded` (the send
  path uses them on every message), and the message views that draw polls and
  embeds.

## Converting a feature branch (for the feature engineers)

Branches written before this structure (eager style) put their events and
results in the core's decoders, their actions and dialogs in the core's
router, and their views in the core or the shared dialog module. Community,
Stage/events and themes were converted this way (task `lazy`); follow the same
steps. Merge the integration branch first, then do them in this order: it
compiles at every step.

### 1. Route the wire (`src/inbound.rs`)

- Your `Event` variant: one line in `routes! { route_event ... features { "<tag>" => <Feature>, } }`.
  Your `CommandResult` variant: the same in `route_result`. The tag is serde's
  (snake_case of the variant name).
- New feature: add a variant to `inbound::Feature`, and `inbound::owner` (test
  only, exhaustive: it does not compile until every `Event` variant is classified).
- Decoders for your own enums: one `crate::decode_tagged!` block per enum in
  `inbound::feature` (variant by variant; fields with `#[serde(default)]` keep it).
  Add each to the list in `every_variant_is_listed_with_its_serde_tag`.
- The test **`inbound::tests::every_variant_is_listed_with_its_serde_tag`**
  compares both routing tables and every `decode_tagged!` table with serde's own
  variant names (read from serde's "unknown variant" error). A variant added to
  the protocol and not routed, or routed under a wrong tag, fails it.
  `feature_decoders_read_what_the_server_writes` round-trips samples; add yours.

### 2. Two entry points (`src/live/<feature>.rs`)

```rust
#[leptos::wasm_split::wasm_split(<name>, wasm_split_path = ::leptos::wasm_split, fallible, preload(prefetch))]
pub(super) fn run(app: Shared, call: features::Call) -> features::Outcome { ... }   // pages, forms, actions

#[leptos::wasm_split::wasm_split(<name>_state, wasm_split_path = ::leptos::wasm_split, fallible)]
pub(super) fn state(app: Shared, call: features::Call) -> features::Outcome { ... } // events, answers, Ready
```

- `features.rs`: the `Feature` in `from_name`, `dispatch` (both arms) and `prefetch`.
- `state` gets `Call::Event(raw)`, `Call::Ready`, `Call::Answer(op, reply)` and
  `Call::State(name, arg)`; `run` gets `Call::Action`, `Call::Submit`,
  `Call::Opened` and `Call::Other(name, arg)`.
- Make the functions behind them private (`fn event`, `fn answered`,
  `fn action`, `fn submit`, `fn opened`): the compiler then shows every call
  from the core you still have to turn into a `Call`.
- From the state module, a page refresh goes to `run` with
  `features::defer(app, Feature::X, Call::Other("queue", g))`; from the core,
  something that keeps state current goes to `state` with
  `features::spawn(app, Feature::X, Call::State("guild", g))` (`defer` when
  the app is borrowed: while rendering, in an event handler).
- What stays in the core: small synchronous helpers that render or decide
  (`community::shown`, `stage::is_stage`, `themes::role_style`, `themes::logo`),
  and hooks that only check something cheap before handing over
  (`themes::message_created` checks the TTS setting and the open channel).

### 3. Route actions, forms, dialogs, answers (`src/live/operations.rs`)

- `action`: `"<kind>" => feature(app, Feature::X, Call::Action(kind.into(), arg.into()))`,
  `"submit" if <yours> => feature(app, Feature::X, Call::Submit(arg.into()))`.
- `open_dialog`: add `(x::owns_dialog(name), Feature::X)` to `owners`.
- `respond`: `Some(op) if x::owns(op) => Some(Feature::X)` in the first
  `handler` match if your answer must run before the generic handling (saved
  forms, notices), or `(_, Some(Feature::X))` in the second one if after it.
- `prefetch_for`: the same rules for `"dialog"` and your action kinds, and
  your view module's `__preload_*` (below).
- `ready` / the socket's Ready: what you do after every Ready must not load
  your module unless the first screen needs it (step 5).

### 4. Views (`src/live/ui.rs`)

- Rows, badges and buttons on the first screen stay in the core, reading the
  feature's state (`a.<feature>`, which is a core struct). Keep them small.
- **Pages and dialogs go in the feature's own view module**, not in the
  shared dialog module: add a `LazyView` to `LazyViews`, a `#[lazy] fn
  load_<x>_dialog()` returning your `dialog` function, and a branch in
  `feature_dialog` (or a body function like `themes_page` for a settings page).
  The dialog module (`load_dialog`) is what User Settings and every other
  dialog wait for on first open: each feature page added there slows them all.

### 5. Nothing at idle (sign-in, Ready)

The idle screen must not load your module. What a feature does after every
Ready, and how it was made free:

| After Ready | Before | Now |
| --- | --- | --- |
| `<Feature>Command::Subscribe` | the command encoder module | written by hand: `wire::encode_small` (add yours, and to its test) |
| The subscription's answer (`Ack`) | the feature's state module | skipped in `respond`, like moderation's |
| An empty list answer (events summary) | the stage state module | compared with `wire::EMPTY_EVENT_SUMMARY` in the core |
| An empty voice snapshot (Stages) | the voice module and stage | compared with `wire::EMPTY_STAGE_SNAPSHOT` |
| The theme library | the themes module (derivation, presets) | kept **undecoded** (`themes::subscribed`) when it holds nothing to paint; decoded when the module loads for anything else |
| `?template=`, `?event=`, `?theme=` links | parsed eagerly | the core reads the address; the module loads only when there is one |

If your answer or event is only sometimes needed, keep the `Raw` in your core
state and decode it when your module next runs. Add a `wire` constant (with a
test that `rmp_serde` writes those bytes) for an empty payload you can skip.

### 6. Scripts and styles

- **`#[wasm_bindgen(raw_module = "...")]` imports load with the core**: the
  glue imports them statically. A feature script must be imported on use
  (`import('/public/x.js')`) from a script that is already loaded; themes'
  `speech.js` is now imported by `preferences.js` on the first thing spoken.
- Not imported at startup: add it to the lazy list in `scripts/finalize.mjs`
  (the service worker then caches it when first used) and to `modules` in
  `scripts/version-runtime.mjs` (fingerprinting; `loading.mjs` checks the count).
- Fonts, icons, art: by URL when shown; keep them out of the precache
  (`finalize.mjs`, as `fonts/chat/` and `app-icons/`).

### 7. Budgets

`npm run test:sizes` lists every split module. A new module without a budget
fails ("has no budget"); a removed one fails too ("which the build no longer
has"). Add `"<name>": <measured Brotli + ~10%>` to `modules` in
`scripts/budgets.json` (the name is the split point: `community`,
`community_state`, `load_community_dialog`). Raise `core`, `firstLoad` or a
module's budget only with the measured cost in the commit message. The
490,000 B line in `tests/loading.mjs` is the owner's and is never raised.

### Common mistakes (all made converting the three features)

- **A dialog's views left in `load_dialog`.** It compiles and works, and adds
  every page of the feature to the first open of User Settings.
- **The state module loaded at sign-in by an answer**: a subscription's `Ack`,
  an empty list, a snapshot. Check `wasm loaded at idle` in `idle.mjs`'s
  report: it lists every module the idle screen downloaded.
- **`raw_module` script imports** (above): they never show up as WASM size,
  only as first-screen JavaScript.
- **Calling a now-private function from `ui/`**: views run in the core or in
  the dialog module; a view that needs a fetch must ask for it with a `Call`.
- **`Call::Other` from the state module for something the state module
  does itself** sends it to `run` and loads the page module: use
  `Call::State`.
- **Parsing a float in the core** (`"1.5".parse::<f64>()`) brings Rust's
  float parser and its power-of-five tables: 13 KB uncompressed, which showed
  up as read-only data segments, not as a function. serde_json has its own;
  pass whole numbers across the JS boundary instead (`themes::reload_prefs`).
  Find such surprises by diffing `twiggy top` of two builds (the "Measuring"
  commands below).
- **Serde attributes on enum fields**: a `decode_tagged!` `fields` entry needs
  the same `#[serde(default)]` as the protocol's field, or old servers'
  frames stop decoding.
- **Feature events from the voice envelope** (`VoiceEvent::Stage`): they have
  no sequence number; the socket hands them to the feature with a tag of
  their own (`stage::VOICE_TAG`) before the voice module sees the payload.
- **PowerShell string escapes** in scripted edits turned `` `r `` into a
  carriage return in a doc comment. Use an editor.

### Verify

```powershell
cargo test -p sticktalk-web inbound          # routing tables, decoders
cargo clippy -p sticktalk-web --target wasm32-unknown-unknown --profile web-release --all-targets -- -D warnings
./scripts/build.ps1                          # release web build
npm run test:sizes                           # every module against budgets.json
node tests/loading.mjs                       # core <= 490,000 B
npm run test:idle                            # memory, CPU, what loads at idle
npm run test:perf                            # first-open latency
cargo test -p sticktalk-server stack_tests   # add your deepest command paths to PATHS
```

Measuring what is in the core (names kept; the sizes are the production ones
without the name section):

```powershell
$t = $env:CARGO_TARGET_DIR; $s = "$t/names-split"; mkdir $s
& "$t/tools/bin/wasm-split-cli.exe" "$t/wasm32-unknown-unknown/web-release/sticktalk-web.wasm" $s --out-name core --target web --cost-model binary --request-cost 16384
& $env:WASM_BINDGEN "$s/core.wasm" --out-dir $s --out-name core --target web --keep-lld-exports --no-demangle --keep-debug
& $env:WASM_OPT "$s/core_bg.wasm" -Oz -g --enable-bulk-memory --enable-nontrapping-float-to-int -o "$s/core_named.wasm"
twiggy top -n 100000 -f csv "$s/core_named.wasm" > core.csv   # grep it for your feature's paths
```

### Pattern changes made here (and why)

- **`Call::State(name, arg)`** (new): calls into the state module named by
  the feature. Community fetches a server's state when it opens, themes
  paints a server theme and speaks; with only `Call::Other` (which goes to
  `run`) these loaded the page module. Expression's `"resolve"` moved to it.
- **Feature dialog views in their own modules** (`ui::LazyViews`,
  `feature_dialog`): see step 4.
- **Raw answers kept in the core** (`themes::State::unread`): an answer that
  is usually not needed is not decoded, and its decoder is not loaded.

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

### The combined tree (task `lazy`)

The integration branch's community, schedule and appearance commands are
dispatched by `commands::mutate` with the other features (boxed, the
appearance one built in a frame of its own by `themes::boxed`), and
`core_mutate` refuses them. Their deepest paths are in `stack_tests::PATHS`:
a `/tts` message (the send path through `themes::mutate`), sharing an event
into a channel (`schedule::share` sends a message through `commands::mutate`),
an event starting (the schedule timer, `schedule::tick`) and a server built
from a starter template (`CreateFromTemplate`). Every server test passes with
the default 2 MiB test stack; `RUST_MIN_STACK` stays removed.

| Path (debug build, `stack_tests::measure`) | Task `perf` | Combined tree |
| --- | ---: | ---: |
| Create a forum post | 903 KiB | 1,031 KiB |
| Share an event into a channel | | 1,031 KiB |
| Send a `/tts` message | | 903 KiB |
| Send a message with mentions | 775 KiB | 839 KiB |
| Scheduled message delivery (timer) | 711 KiB | 775 KiB |
| Publish an announcement + crosspost delivery | 391 KiB | 455 KiB |
| Build a server from a template | | 455 KiB |
| Opening the server (migrations, timers) | 263 KiB | 271 KiB |
| An event starts (timer) | | 199 KiB |

The send path grew by one 64 KiB step since task `perf` (most likely the merged
features' hooks on it, such as `community::observe` on every event; not
bisected). All paths stay
under the 2 MiB debug budget (release was not re-measured: its paths were
65-192 KiB, far under its 1 MiB budget).