# Building StickTalk activities

An activity is a small web app that people in a StickTalk call run together:
a game, a shared canvas, a player. This guide is for anyone writing one: the
embedded app SDK, the message protocol, the shared state model, the limits,
and the exact security boundary an activity runs in.

The built-in activities (Watch Together, Whiteboard, Four in a Row,
Checkers, Sketch & Guess) use nothing but what is described here. A minimal
example is in `crates/web/public/activities/example/` (a shared counter); the
browser test `crates/web/tests/activities.mjs` serves it from another origin
and runs it as a registered activity.

## 1. What an activity is

- A page (HTML, scripts, styles, images) that StickTalk loads in a frame in
  the call view. It is launched from the call's Activities button, and
  everyone in the call can join it.
- One **session** per call at a time. It ends when its last participant
  leaves it, when the call ends, or when the host or a moderator ends it.
- Each participant (each device) has a small **participant number** (`pid`)
  for the session. The activity sees participants' display names, pictures
  (as `data:` URLs) and who is speaking; never account IDs, tokens, e-mail
  addresses or anything else.

## 2. The security boundary

| | Built-in activity | Registered activity |
|---|---|---|
| Frame | `<iframe sandbox="allow-scripts" allow="autoplay; fullscreen">` at `/activities/<id>/` | the same sandbox, holding the shell `/activities/frame/custom-<n>`, which holds your page (again `sandbox="allow-scripts"`) |
| Origin | opaque (`null`) | opaque (`null`); the sandbox is inherited, so your page does **not** run with your own origin |
| Its policy | `default-src 'none'`; scripts, styles, fonts and images from the StickTalk server only (images also `data:`/`blob:`); `connect-src 'none'`; `media-src 'none'`; `frame-src 'none'`; `worker-src 'none'`; `frame-ancestors` the StickTalk origin | the shell's: `frame-src <your registered origin>` only, `connect-src 'none'`; your own page's policy is yours |
| App's policy | `frame-src 'self'` | unchanged: the app never lists your origin. Only the shell (served fresh from the registry) allows it |

What follows from this:

- **No access to StickTalk.** The frame cannot read the app's cookies,
  storage or DOM, cannot call its API with the person's session (its origin
  is opaque and it sends no cookies; built-in frames cannot make requests at
  all), and cannot navigate the app (`allow-top-navigation` is not granted).
  It cannot open pop-ups, submit forms, use `alert`/`confirm`, download files
  by itself, lock the pointer, or ask for the camera, microphone or screen.
- **No storage of its own.** `localStorage`, `sessionStorage`, IndexedDB and
  cookies throw or are empty in an opaque origin. Keep everything in the
  session's document; keep a result with `saveResult`.
- **No navigation away.** The app's `frame-src` (built-in) and the shell's
  `frame-src` (registered) refuse to load anything else in the frame. If a
  frame still reloads, the host stops talking to it.
- **Every message is checked.** The host accepts messages only from the exact
  frame window it created (`event.source`, origin `null`), only with
  `sticktalk: 1` and a known `type`, with keys, sizes and counts checked
  before anything is sent. The server checks all of it again (it never
  trusts the page either) and checks permissions on every signal.
- **A registered activity can still** run any script in its sandbox, show
  anything it likes inside its frame, see the names and pictures of the
  people in the session and what they write to the session's document,
  and make network requests to wherever *its own* page's policy allows
  (that is the activity operator's responsibility and visible to them only).
  An instance administrator should register only activities they trust with
  that. Sound a registered activity plays itself goes to the device's default
  output (see 6. Sound).

**What your server must send.** Your page is framed by the shell, whose
origin is opaque: do not send `X-Frame-Options`, and do not send a
`frame-ancestors` directive (no source, not even `*`, matches an opaque
origin). If you send a Content Security Policy, name your own origin
explicitly (`script-src https://games.example`): in the sandbox, `'self'`
is the opaque origin and matches nothing.

## 3. The SDK

Include `sdk.js` before your own script. Built-in activities use
`../sdk.js`; a registered activity copies `crates/web/public/activities/sdk.js`
to its own origin.

```html
<script src="sdk.js"></script>
<script src="app.js" defer></script>
```

```js
const st = window.StickTalk;
const ctx = await st.ready();     // { session, you, host, shared, participants, volume }
st.on('state', ({ reset, entries }) => draw());
st.on('participants', (ctx) => drawPeople(ctx.participants));
st.on('message', ({ from, data }) => showCursor(from, data));
st.on('speaking', ({ pid, speaking }) => ring(pid, speaking));
await st.set('board/title', { text: 'Our plan' });
st.send({ cursor: [x, y] });      // ephemeral, to everyone else
```

| Call | Does |
|---|---|
| `ready()` | Resolves once the host introduced the session: `{ session: { id, activity: { id, name }, started_at }, you, host, shared, participants: [{ pid, name, avatar, host, you, speaking }], volume }` |
| `on(event, fn)` | `state`, `participants`, `message`, `speaking`, `player`, `volume`. Returns an unsubscribe function |
| `context()`, `me()`, `participant(pid)`, `isHost()`, `canControl()` | The current view; `canControl()` is the host, or everyone once the host shared control |
| `get(key)`, `entries(prefix)`, `versionOf(key)` | Read the session's document |
| `set(key, value, { expect })`, `remove(key)`, `batch(ops)` | Write it (see 4.). Resolve when the server accepted; reject with a reason |
| `send(data, to = [])` | An ephemeral message (JSON) to these participants, or everyone else |
| `action(data)` | A move for a game the server runs (built-in games only) |
| `serverNow()` | The server's clock in ms (for timelines) |
| `player.set({ item, playing, position, rate })`, `player.get()`, `player.position()` | The shared player (see 6.) |
| `shareMedia()` | The person chooses or uploads a file; resolves `{ id, name, type }` once it is shared into the session |
| `saveResult(text)` | The host keeps a short result (≤ 160 characters) shown when the session ends |
| `resize(height)` | The frame's height below the player, in px (96–900), or `null` |
| `layout('focus' \| 'split' \| 'mini')` | Ask for a layout |
| `invite()` | Open the invite picker (a card with Join in a conversation) |
| `download(name, pngDataUrl)` | Offer a PNG to save; the person confirms in the app |
| `close()` | Leave the activity |

## 4. The shared document

Each session has one key-value document, relayed by the server over the
app's existing connection. Late joiners get all of it first (in parts of
48 KB), then every change. Nothing is kept after the session ends.

- **Keys**: 1–64 bytes of `A-Z a-z 0-9 / _ - . :`.
- **Values**: JSON, at most 16 KB each.
- **Whole document**: at most 256 KB (keys and values) and 2048 keys. A write
  that would go over is refused (`limit_exceeded`); nothing is evicted.
- **A write** is 1–32 operations, applied all or none. `value: null` deletes.
- **Order and conflicts**: the server orders writes; the last one it accepts
  wins (each entry carries `version`, the document's version at that write,
  and `by`, the writer's `pid`, 0 for the server). For a safe update use
  compare-and-set: `set(key, value, { expect: versionOf(key) })` succeeds
  only if nobody changed the key meanwhile (`expect: 0`: only if it does not
  exist), otherwise it rejects with `conflict`.
- **Who may write which keys**:

  | Prefix | Writable by |
  |---|---|
  | `h/` | the host, or everyone while the host shares control (`SetShared`) |
  | `p/<pid>/` | only that participant |
  | `s/` | only the server (shared files `s/media/<id>`, built-in games' `s/game`) |
  | anything else | any participant |

  The host is whoever launched it; it passes to the earliest-joined
  participant when the host leaves, and the host can hand it to anyone.
- **Ephemeral messages** (`send`): JSON, at most 2 KB, to named participants
  or everyone else; not stored, not replayed. Use them for cursors and
  strokes in progress. A slow connection may miss some.
- **Rates** (per device): 600 writes and 1200 messages a minute (bursts 60
  and 40); per session 1800 writes and 3600 messages a minute. Batch your
  cursor updates (the whiteboard sends at most 15 a second).

**Server-authoritative games.** For the built-in games the server is the
host: it runs the rules (`crates/server/src/activities/games.rs`), only it
writes `s/game`, and players send moves with `action(...)`. A move out of
turn, an illegal move or a write to the game is refused, so a tampered
client cannot cheat. A private fact (Sketch & Guess's word) is sent by the
server only to the participant who may know it (`message` with `from: 0`).
Registered activities can use the `h/` keys with a host participant for the
same pattern, with the host's client deciding.

**Visible text.** Text an activity shows to the others must be in a property
named `text` (anywhere in a value or message). In a server's voice channel
the server screens it with that server's AutoMod keyword rules before it is
stored or relayed, exactly like a message (blocked text is refused and
logged; Manage Server is exempt). The built-in activities show text only
from `text` properties.

## 5. The message protocol (version 1)

Between the activity frame and the StickTalk page, by `postMessage`. Every
message is an object with `sticktalk: 1` and a `type`. Requests from the
frame may carry a numeric `id`; the host answers each with
`{ type: 'result', id, ok, error, value }`.

From the frame:

| type | fields |
|---|---|
| `ready` | — (sent by `sdk.js` when it loads; the host answers with `init`) |
| `set` | `id`, `ops: [{ key, value, expect? }]` |
| `send` | `id`, `to: [pid]`, `data` |
| `action` | `id`, `data` |
| `share-media`, `save-result` (`text`), `resize` (`height`), `layout` (`layout`), `invite`, `download` (`name`, `url`) | `id` |
| `close` | — |

From the host:

| type | fields |
|---|---|
| `init` | `sdk`, `session`, `you`, `host`, `shared`, `participants`, `state: { entries, version }`, `server_offset`, `theme`, `volume` |
| `state` | `reset`, `entries: [{ key, value, version, by }]` (value `undefined`: deleted), `version`, `server_offset` |
| `participants` | `participants`, `host`, `shared`, `you` |
| `message` | `from` (0: the server), `data` |
| `speaking` | `pid`, `speaking` |
| `player` | `status: { item, time, duration, paused, ended, buffering, problem } \| null` |
| `volume` | `gain` (0–1; 0 while deafened) |
| `theme` | `theme` |
| `result` | `id`, `ok`, `error`, `value` |

`theme` holds the app's current colors (`background`, `surface`, `muted`,
`text`, `subtle`, `accent`, `accentText`, `border`, `danger`), `font` and
`dark`; `sdk.js` sets them as `--st-*` custom properties.

## 6. Sound: the shared player

Sound must go through the call's output device and volume and respect
deafen. A frame cannot pick an output device, so the StickTalk page plays
shared media itself:

1. `shareMedia()` lets the person choose an audio or video file already on
   this StickTalk server in a conversation they can read (or upload one to
   the call's chat). The server checks they can read it and adds it to the
   session (`s/media/<id>`: name, type, size). While the session runs, every
   participant may stream it; nobody else may.
2. Whoever controls playback writes `h/player = { item, playing, position,
   at, rate }` with `player.set(...)` (`at` is the server time of `position`).
3. The page plays the file in the tile (video above the frame, or a label for
   audio) through the call's output device, at the person's activity volume,
   silent while deafened. It keeps each viewer near the shared timeline by
   nudging the playback rate (±8%) for drift over 80 ms and seeking only for
   more than 1.5 s; no timer, only media events.
4. The frame receives `player` status events for its controls.

An activity that plays sound itself (Web Audio in its frame) reaches the
device's default output instead; it receives `volume` events and should
apply that gain.

## 7. Registering an activity

The instance administrator registers it on the server's machine:

```text
sticktalk-server --data-dir <dir> admin activities add --name "Trivia" \
  --url https://games.example/trivia/ --icon 🎲 --description "Quick questions" \
  --min-players 2 --max-players 8 [--in-dms]
sticktalk-server --data-dir <dir> admin activities list
sticktalk-server --data-dir <dir> admin activities remove custom-1
```

- The URL must be HTTPS (plain HTTP only for `127.0.0.1`/`localhost` test
  servers), without credentials or a fragment. At most 32 registrations.
- Each server turns it on in Server Settings › Activities (Manage Server).
  A registered activity is off until a server turns it on; built-in ones are
  on until turned off. `--in-dms` also offers it in direct and group calls.
- Turning an activity off ends its running sessions in that server.

## 8. A minimal activity

`crates/web/public/activities/example/app.js`, in full:

```js
(async () => {
  const st = window.StickTalk;
  const $ = (id) => document.getElementById(id);
  const ctx = await st.ready();
  const show = () => {
    $('count').textContent = String(st.get('count') ?? 0);
    const c = st.context();
    $('people').textContent = c.participants.map((p) => p.name + (p.pid === c.host ? ' (host)' : '')).join(', ');
  };
  st.on('state', show);
  st.on('participants', show);
  st.on('message', ({ from, data }) => {
    if (data?.wave) $('log').textContent = `${st.participant(from)?.name ?? 'Someone'} waved.`;
  });
  $('add').onclick = async () => {
    for (let i = 0; i < 3; i++) {
      try {
        await st.set('count', (st.get('count') ?? 0) + 1, { expect: st.versionOf('count') });
        return;
      } catch { /* someone else was first; try again with their value */ }
    }
  };
  $('wave').onclick = () => st.send({ wave: true });
  show();
  document.body.dataset.ready = String(ctx.you);
})();
```
