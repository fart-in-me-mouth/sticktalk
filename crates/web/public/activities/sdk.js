// StickTalk embedded app SDK, protocol version 1 (docs/ACTIVITIES.md).
//
// Include it in an activity page: <script src="/activities/sdk.js"></script>
// (a registered activity copies this file to its own origin). It talks to the
// StickTalk page that frames the activity, only through postMessage, and
// exposes `window.StickTalk`:
//
//   const ctx = await StickTalk.ready();        // session, you, participants
//   StickTalk.on('state', ({ reset, entries }) => ...);
//   await StickTalk.set('notes/1', { text: 'Hi' });
//   StickTalk.send({ x, y });                    // ephemeral, to everyone else
//
// The frame runs sandboxed in an opaque origin: no cookies, no storage, no
// network. Everything shared goes through the session's document.
(() => {
  'use strict';
  const VERSION = 1;
  const host = window.parent;
  const listeners = new Map();
  const state = new Map();
  const waiting = new Map();
  let ctx = null;
  let offset = 0;
  let next = 1;
  let resolveReady;
  const ready = new Promise((r) => { resolveReady = r; });

  function fire(name, detail) {
    for (const fn of listeners.get(name) ?? []) {
      try { fn(detail); } catch (e) { console.error(e); }
    }
  }
  function post(type, body = {}) {
    host.postMessage({ sticktalk: VERSION, type, ...body }, '*');
  }
  function request(type, body = {}) {
    const id = next++;
    return new Promise((resolve, reject) => {
      waiting.set(id, { resolve, reject });
      post(type, { id, ...body });
    });
  }
  function applyTheme(t) {
    if (!t) return;
    const root = document.documentElement.style;
    for (const [name, value] of Object.entries(t)) {
      if (typeof value === 'string' && value.length < 200 && !/[;{}]/.test(value)) {
        root.setProperty(`--st-${name.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`, value);
      }
    }
    document.documentElement.dataset.theme = t.dark ? 'dark' : 'light';
  }
  function view() {
    return {
      session: ctx.session,
      you: ctx.you,
      host: ctx.host,
      shared: ctx.shared,
      participants: ctx.participants.slice(),
      volume: ctx.volume,
    };
  }

  addEventListener('message', (e) => {
    if (e.source !== host) return;
    const m = e.data;
    if (!m || typeof m !== 'object' || m.sticktalk !== VERSION) return;
    switch (m.type) {
      case 'init':
        ctx = m;
        offset = Number(m.server_offset) || 0;
        state.clear();
        for (const entry of m.state.entries) state.set(entry.key, entry);
        applyTheme(m.theme);
        resolveReady(view());
        fire('participants', view());
        fire('state', { reset: true, entries: m.state.entries.slice() });
        break;
      case 'state':
        if (Number.isFinite(m.server_offset)) offset = m.server_offset;
        if (m.reset) state.clear();
        for (const entry of m.entries) {
          if (entry.value === undefined || entry.value === null) state.delete(entry.key);
          else state.set(entry.key, entry);
        }
        fire('state', { reset: !!m.reset, entries: m.entries });
        break;
      case 'participants':
        if (!ctx) return;
        Object.assign(ctx, { participants: m.participants, host: m.host, shared: m.shared, you: m.you });
        fire('participants', view());
        break;
      case 'message':
        fire('message', { from: m.from, data: m.data });
        break;
      case 'speaking': {
        const p = ctx?.participants.find((p) => p.pid === m.pid);
        if (p) p.speaking = m.speaking;
        fire('speaking', { pid: m.pid, speaking: m.speaking });
        break;
      }
      case 'player':
        fire('player', m.status);
        break;
      case 'volume':
        if (ctx) ctx.volume = m.gain;
        fire('volume', m.gain);
        break;
      case 'theme':
        applyTheme(m.theme);
        break;
      case 'result': {
        const w = waiting.get(m.id);
        if (!w) return;
        waiting.delete(m.id);
        if (m.ok) w.resolve(m.value);
        else w.reject(new Error(m.error || 'The request failed.'));
        break;
      }
    }
  });

  const api = Object.freeze({
    version: VERSION,
    /** Resolves with the session once the host has introduced it. */
    ready: () => ready,
    /** Events: state, participants, message, speaking, player, volume. Returns an unsubscribe function. */
    on(name, fn) {
      if (!listeners.has(name)) listeners.set(name, new Set());
      listeners.get(name).add(fn);
      return () => listeners.get(name)?.delete(fn);
    },
    context: () => (ctx ? view() : null),
    me: () => ctx?.participants.find((p) => p.pid === ctx.you) ?? null,
    isHost: () => !!ctx && ctx.host === ctx.you,
    canControl: () => !!ctx && (ctx.host === ctx.you || ctx.shared),
    participant: (pid) => ctx?.participants.find((p) => p.pid === pid) ?? null,
    /** The value of a key of the shared document (undefined if none). */
    get: (key) => state.get(key)?.value,
    /** The key's version (0 if it does not exist), for `set(..., { expect })`. */
    versionOf: (key) => state.get(key)?.version ?? 0,
    /** Every entry, or those whose key starts with `prefix`. */
    entries(prefix = '') {
      const out = [];
      for (const entry of state.values()) if (entry.key.startsWith(prefix)) out.push(entry);
      return out;
    },
    /** Write one key (JSON-able value; null deletes). `expect`: only if its version is this. */
    set: (key, value, { expect } = {}) => request('set', { ops: [{ key, value, expect }] }),
    remove: (key, { expect } = {}) => request('set', { ops: [{ key, value: null, expect }] }),
    /** Several writes, all or none (at most 32). */
    batch: (ops) => request('set', { ops }),
    /** An ephemeral message to these participants (none: everyone else). */
    send: (data, to = []) => request('send', { data, to }),
    /** A move for a game the server runs. */
    action: (data) => request('action', { data }),
    /** The server's clock, in milliseconds. */
    serverNow: () => Date.now() + offset,
    /** The shared player: everyone hears `item` (a shared file) in sync. */
    player: Object.freeze({
      set({ item, playing, position, rate = 1 }) {
        return request('set', { ops: [{ key: 'h/player', value: { item: item == null ? null : String(item), playing: !!playing, position: Math.max(0, Number(position) || 0), rate, at: Date.now() + offset } }] });
      },
      get: () => state.get('h/player')?.value ?? null,
      /** Where the shared timeline is now, in seconds. */
      position() {
        const p = state.get('h/player')?.value;
        if (!p) return 0;
        return p.playing ? p.position + Math.max(0, Date.now() + offset - p.at) / 1000 * (p.rate || 1) : p.position;
      },
    }),
    /** Ask the person to choose (or upload) a file to share; resolves with { id, name, type }. */
    shareMedia: () => request('share-media'),
    /** Keep a short result for when the session ends (the host only). */
    saveResult: (text) => request('save-result', { text }),
    /** Ask for this frame height in pixels below the player (or null). */
    resize: (height) => request('resize', { height }),
    /** Ask for a layout: 'focus', 'split' or 'mini'. */
    layout: (layout) => request('layout', { layout }),
    /** Open the invite picker (a card with Join in a conversation). */
    invite: () => request('invite'),
    /** Offer a PNG (a data: URL) to save; the person confirms. */
    download: (name, url) => request('download', { name, url }),
    /** Leave the activity. */
    close: () => post('close'),
  });
  Object.defineProperty(window, 'StickTalk', { value: api });
  post('ready');
})();
