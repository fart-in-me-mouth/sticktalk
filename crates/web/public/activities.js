// Shared activities in a call (docs/features/activities.md, docs/ACTIVITIES.md):
// the launcher, the activity's tile with its sandboxed frame, the embedded app
// SDK bridge, the shared player (Watch Together's sound goes through the call's
// output device and volume), invites, reports and the server's settings page.
//
// Loaded only when an activity is seen in the call or the launcher is opened.
// The frame lives in one layer outside the app's re-rendered tree, so it never
// reloads while the call view changes; it follows the place the call view keeps
// for it (#live-activity-slot) and becomes a small floating window elsewhere.
// No timers and no animation loops: everything runs on events.
import * as extras from './callextras.js';

const SDK = 1;
const $ = (s, root = document) => root.querySelector(s);
const make = (tag, className = '', text = '') => {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (text) e.textContent = text;
  return e;
};
let emit = () => {};
const KEY = /^[A-Za-z0-9/_\-.:]{1,64}$/;
const LIMITS = { value: 16 * 1024, message: 2048, ops: 32, result: 160, download: 8 * 1024 * 1024 };

// ---- this device's preferences ------------------------------------------------
const PREFS = 'sticktalk-activities';
let prefs = { volume: 80 };
try {
  const raw = JSON.parse(localStorage.getItem(PREFS) ?? '{}');
  const v = Number(raw.volume);
  if (Number.isFinite(v)) prefs.volume = Math.min(100, Math.max(0, Math.round(v)));
} catch {}
function savePrefs() {
  try { localStorage.setItem(PREFS, JSON.stringify(prefs)); } catch {}
}

// ---- the session ------------------------------------------------------------------
let session = null;      // ActivitySession as JSON (IDs as strings)
const doc = new Map();   // key -> { value (parsed), version, by }
let version = 0;
let offset = null;       // server clock minus this clock, best estimate (ms)
let reset = null;        // entries of a document arriving in parts
let wantJoin = null;     // a Join to send once the session is known
const speaking = new Set();
let deafened = !!globalThis.__sticktalkDeafened;
let unfollow = null;

const sessionKey = (s) => `${s.user_id}:${s.connection_id}`;
const me = () => session?.participants.find((p) => p.pid === session.you);
const isHost = () => !!session && session.you != null && session.host === session.you;
const serverNow = () => Date.now() + (offset ?? 0);

// ---- signals (through the app's voice connection) ----------------------------------
const pending = new Map();
const MESSAGES = {
  forbidden: 'You can’t do that in this activity.',
  not_found: 'That activity is no longer running.',
  conflict: 'Someone else just changed that. Try again.',
  limit_exceeded: 'The activity is full.',
  rate_limited: 'Slow down a little.',
  payload_too_large: 'That is too large for the activity.',
  invalid_request: 'The activity sent something it shouldn’t.',
  unsupported_media_type: 'Only audio and video files can be shared.',
};
function failure(data) {
  const code = String(data?.code ?? '').replace(/([a-z])([A-Z])/g, '$1_$2').toLowerCase();
  const text = typeof data?.message === 'string' && data.message && !/^[A-Z][A-Za-z]+$/.test(data.message)
    ? data.message : MESSAGES[code] ?? 'That didn’t work.';
  return Object.assign(new Error(text), { code });
}
function signal(type, data) {
  const nonce = String(Math.floor(Math.random() * 2 ** 52) + 1);
  const payload = JSON.stringify({ type: 'activities', data: { type, data } });
  return new Promise((resolve, reject) => {
    if (pending.size >= 64) {
      const [oldest, p] = pending.entries().next().value;
      pending.delete(oldest);
      p.reject(new Error('No answer from the server.'));
    }
    pending.set(nonce, { resolve, reject });
    emit(`activity-signal:${nonce}:${payload}`);
  });
}

// ---- events from the server ---------------------------------------------------------
export function event(e, fn) {
  emit = fn;
  if (e.request_nonce) {
    const p = pending.get(e.request_nonce);
    pending.delete(e.request_nonce);
    if (p) e.type === 'ack' ? p.resolve() : p.reject(failure(e.data));
    return;
  }
  const d = e.data;
  switch (e.type) {
    case 'session': return onSession(d);
    case 'ended': return onEnded(d);
    case 'state': return onState(d);
    case 'message':
      if (d.session_id === session?.session_id) post({ type: 'message', from: d.from, data: parse(d.data) });
      return;
  }
}

function parse(text) {
  try { return JSON.parse(text); } catch { return null; }
}

function onSession(s) {
  if (session && session.session_id !== s.session_id) close();
  session = s;
  follow();
  if (wantJoin === s.session_id && s.you == null) { wantJoin = null; join(s.session_id); }
  if (s.you != null) {
    openFrame();
    post({ type: 'participants', ...people() });
  } else if (frame) {
    close();
  }
  renderBar();
}

function onEnded(d) {
  if (session?.session_id !== d.session_id) return;
  const name = session.activity.name;
  const result = d.summary?.result ? ` ${d.summary.result}.` : '';
  close();
  session = null;
  emit(`activity:notice,${name} ended.${result}`);
}

function onState(d) {
  if (d.session_id !== session?.session_id) return;
  const sample = d.server_time - Date.now();
  // The largest sample is closest to the true offset (smallest delay).
  offset = offset == null ? sample : Math.max(offset, sample);
  const entries = d.entries.map((e) => ({ key: e.key, value: e.value == null ? undefined : parse(e.value), version: e.version, by: e.by }));
  if (d.reset) reset = [];
  if (reset) {
    reset.push(...entries);
    if (!d.done) return;
    doc.clear();
    for (const e of reset) doc.set(e.key, e);
    version = d.version;
    post({ type: 'state', reset: true, entries: reset, version, server_offset: offset });
    reset = null;
  } else {
    for (const e of entries) e.value === undefined ? doc.delete(e.key) : doc.set(e.key, e);
    version = d.version;
    post({ type: 'state', reset: false, entries, version, server_offset: offset });
  }
  if (entries.some((e) => e.key === 'h/player' || e.key.startsWith('s/media/'))) player.update();
}

// ---- actions from the app -------------------------------------------------------
export function action(a, fn) {
  emit = fn;
  if (a === 'launcher') return launcher();
  if (a.startsWith('join:')) {
    const id = a.slice(5);
    if (session?.session_id === id) join(id);
    else wantJoin = id;
  }
}

async function join(id) {
  try { await signal('join', { session_id: id }); } catch (e) { if (e.code !== 'conflict') notice(e.message); }
}

export function mount(fn) {
  emit = fn;
  for (const section of document.querySelectorAll('[data-activity-settings]')) settingsSection(section);
  if (frame) place();
}

// ---- following the call ------------------------------------------------------------
function follow() {
  if (unfollow) return;
  unfollow = extras.follow((what, value) => {
    if (what === 'speaking') {
      const k = sessionKey(value.session);
      value.speaking ? speaking.add(k) : speaking.delete(k);
      const p = session?.participants.find((p) => sessionKey(p.session) === k);
      if (p) post({ type: 'speaking', pid: p.pid, speaking: !!value.speaking });
    } else if (what === 'deafen') {
      deafened = !!value;
      player.volume();
    } else if (what === 'output') {
      player.output(value);
    } else if (what === 'stop') {
      close();
      session = null;
      wantJoin = null;
      doc.clear();
      unfollow?.();
      unfollow = null;
    }
  });
}

// ---- the tile and its frame -------------------------------------------------------
let frame = null;
function icon(name, className = 'icon size-4') {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('class', className);
  svg.setAttribute('aria-hidden', 'true');
  const use = document.createElementNS(svg.namespaceURI, 'use');
  use.setAttribute('href', `#i-${name}`);
  svg.append(use);
  return svg;
}
function iconButton(glyph, label, onclick, extra = '') {
  const b = make('button', `icon-button activity-bar-button ${extra}`);
  b.type = 'button';
  b.setAttribute('aria-label', label);
  b.title = label;
  b.append(icon(glyph));
  b.onclick = onclick;
  return b;
}

function openFrame() {
  if (frame || !session) return;
  const layer = make('section', 'activity-layer');
  layer.id = 'live-activity-layer';
  layer.setAttribute('aria-label', `${session.activity.name} activity`);
  const bar = make('header', 'activity-bar');
  const body = make('div', 'activity-body');
  const video = make('video', 'activity-player');
  video.hidden = true;
  video.playsInline = true;
  video.preload = 'auto';
  const audioLabel = make('div', 'activity-player-audio');
  audioLabel.hidden = true;
  const iframe = make('iframe', 'activity-frame');
  iframe.title = session.activity.name;
  iframe.setAttribute('sandbox', 'allow-scripts');
  iframe.setAttribute('allow', 'autoplay; fullscreen');
  iframe.setAttribute('referrerpolicy', 'no-referrer');
  iframe.setAttribute('loading', 'eager');
  const note = make('div', 'activity-note');
  note.setAttribute('role', 'status');
  body.append(video, audioLabel, iframe);
  layer.append(bar, body, note);
  frame = { layer, bar, body, iframe, video, audioLabel, note, ready: false, loads: 0, id: session.session_id, height: null, observer: null, slot: null };
  iframe.addEventListener('load', () => {
    // A frame that navigates away is no longer the activity: stop talking to it.
    if (++frame.loads > 1) { frame.ready = false; notice('The activity left its page and was stopped.'); iframe.remove(); }
  });
  iframe.src = session.activity.url;
  document.body.append(layer);
  addEventListener('message', onFrameMessage);
  addEventListener('resize', place);
  renderBar();
  place();
}

function close() {
  if (!frame) return;
  removeEventListener('message', onFrameMessage);
  removeEventListener('resize', place);
  frame.observer?.disconnect();
  player.stop();
  frame.layer.remove();
  frame = null;
  speaking.clear();
}

/** Put the tile over the call view's slot, or float it while the call view is not shown. */
function place() {
  if (!frame) return;
  const slot = $('#live-activity-slot');
  const layout = slot?.dataset.layout ?? 'mini';
  const mini = !slot || layout === 'mini';
  frame.layer.dataset.mode = mini ? 'mini' : layout;
  if (slot !== frame.slot) {
    frame.observer?.disconnect();
    frame.slot = slot;
    if (slot) {
      frame.observer = new ResizeObserver(place);
      frame.observer.observe(slot);
    }
  }
  const style = frame.layer.style;
  if (frame.mode !== frame.layer.dataset.mode) {
    frame.mode = frame.layer.dataset.mode;
    renderBar();
  }
  if (mini) {
    style.left = style.top = style.width = style.height = '';
  } else {
    const r = slot.getBoundingClientRect();
    style.left = `${Math.round(r.left)}px`;
    style.top = `${Math.round(r.top)}px`;
    style.width = `${Math.round(r.width)}px`;
    style.height = `${Math.round(r.height)}px`;
  }
}

function people() {
  return {
    host: session.host,
    shared: !!session.shared_control,
    you: session.you,
    participants: session.participants.map((p) => ({
      pid: p.pid,
      name: p.name,
      avatar: avatars.get(p.avatar_id)?.url ?? null,
      host: p.pid === session.host,
      you: p.pid === session.you,
      speaking: speaking.has(sessionKey(p.session)),
    })),
  };
}

// Pictures for the frame: data URLs (the frame cannot use the app's cookies).
const avatars = new Map();
function avatar(id) {
  if (!id || avatars.has(id)) return;
  if (avatars.size >= 48) avatars.delete(avatars.keys().next().value);
  const entry = { url: null };
  avatars.set(id, entry);
  fetch(`/api/v1/files/${id}/thumbnail`, { credentials: 'same-origin' })
    .then((r) => (r.ok ? r.blob() : null))
    .then((b) => b && b.size <= 96 * 1024 && new Promise((resolve) => {
      const reader = new FileReader();
      reader.onload = () => resolve(reader.result);
      reader.onerror = () => resolve(null);
      reader.readAsDataURL(b);
    }))
    .then((url) => {
      if (!url) return;
      entry.url = url;
      if (session) post({ type: 'participants', ...people() });
      renderBar();
    })
    .catch(() => {});
}

function theme() {
  const css = getComputedStyle(document.documentElement);
  const v = (name) => css.getPropertyValue(name).trim();
  return {
    background: v('--bg-chat'),
    surface: v('--bg-sidebar'),
    muted: v('--bg-elevated'),
    text: v('--text-primary'),
    subtle: v('--text-secondary'),
    accent: v('--accent'),
    accentText: v('--accent-text'),
    border: v('--divider'),
    danger: v('--danger'),
    font: getComputedStyle(document.body).fontFamily,
    dark: document.documentElement.dataset.themeBase !== 'light',
  };
}

function post(message) {
  if (!frame?.ready || !frame.iframe.contentWindow) return;
  try {
    // An opaque-origin frame cannot be addressed by origin; the target is
    // this exact window, checked again on every message it sends.
    frame.iframe.contentWindow.postMessage({ sticktalk: SDK, ...message }, '*');
  } catch {}
}

function init() {
  const entries = [...doc.values()];
  post({
    type: 'init',
    sdk: SDK,
    session: { id: session.session_id, activity: { id: session.activity.id, name: session.activity.name }, started_at: session.started_at },
    ...people(),
    state: { entries, version },
    server_offset: offset ?? 0,
    theme: theme(),
    volume: gain(),
  });
  player.status(true);
}

const gain = () => (deafened ? 0 : prefs.volume / 100);
const reply = (id, ok, error, value) => id != null && post({ type: 'result', id, ok, error: error ?? null, value: value ?? null });
const bytes = (s) => new Blob([s]).size;

async function onFrameMessage(e) {
  if (!frame || e.source !== frame.iframe.contentWindow || e.origin !== 'null') return;
  const m = e.data;
  if (!m || typeof m !== 'object' || m.sticktalk !== SDK || typeof m.type !== 'string') return;
  const id = Number.isSafeInteger(m.id) ? m.id : null;
  const s = session;
  if (!s) return;
  try {
    switch (m.type) {
      case 'ready':
        frame.ready = true;
        init();
        return;
      case 'set': {
        if (!Array.isArray(m.ops) || !m.ops.length || m.ops.length > LIMITS.ops) throw new Error('Too many changes at once.');
        const ops = m.ops.map((op) => {
          if (!op || typeof op.key !== 'string' || !KEY.test(op.key)) throw new Error('That key is not allowed.');
          const value = op.value === undefined || op.value === null ? null : JSON.stringify(op.value);
          if (value != null && bytes(value) > LIMITS.value) throw new Error('That value is too large.');
          const out = { key: op.key, value };
          if (Number.isSafeInteger(op.expect) && op.expect >= 0) out.expect = op.expect;
          return out;
        });
        await signal('write', { session_id: s.session_id, ops });
        return reply(id, true);
      }
      case 'send': {
        const data = JSON.stringify(m.data ?? null);
        if (bytes(data) > LIMITS.message) throw new Error('That message is too large.');
        const to = Array.isArray(m.to) ? m.to.filter(Number.isSafeInteger).slice(0, 16) : [];
        await signal('send', { session_id: s.session_id, to, data });
        return reply(id, true);
      }
      case 'action': {
        const data = JSON.stringify(m.data ?? null);
        if (bytes(data) > LIMITS.message) throw new Error('That move is too large.');
        await signal('action', { session_id: s.session_id, data });
        return reply(id, true);
      }
      case 'save-result': {
        const text = String(m.text ?? '').trim().slice(0, LIMITS.result);
        await signal('save_result', { session_id: s.session_id, text });
        return reply(id, true);
      }
      case 'share-media': {
        const item = await pickMedia();
        if (!item) return reply(id, false, 'cancelled');
        await signal('share_media', { session_id: s.session_id, attachment_id: item.attachment_id });
        return reply(id, true, null, { id: item.attachment_id, name: item.filename, type: item.content_type });
      }
      case 'resize': {
        const h = Number(m.height);
        frame.height = Number.isFinite(h) ? Math.min(900, Math.max(96, Math.round(h))) : null;
        frame.body.style.setProperty('--activity-frame-height', frame.height ? `${frame.height}px` : '');
        return reply(id, true);
      }
      case 'layout':
        if (['focus', 'split', 'mini'].includes(m.layout)) emit(`activity:layout,${m.layout}`);
        return reply(id, true);
      case 'invite':
        invite();
        return reply(id, true);
      case 'download':
        download(String(m.name ?? 'activity.png'), m.url);
        return reply(id, true);
      case 'close':
        leave();
        return;
      default:
        throw new Error('Unknown request.');
    }
  } catch (error) {
    reply(id, false, error.message);
  }
}

// ---- the tile's bar ----------------------------------------------------------------
function renderBar() {
  if (!frame || !session) return;
  const bar = frame.bar;
  const mode = frame.layer.dataset.mode;
  bar.replaceChildren();
  const title = make('div', 'activity-title');
  const glyph = /^[a-z0-9-]+$/.test(session.activity.icon) ? icon(session.activity.icon, 'icon size-4 text-primary') : make('span', '', session.activity.icon);
  title.append(glyph, make('strong', 'truncate', session.activity.name));
  const list = make('ul', 'activity-people');
  list.setAttribute('aria-label', 'In the activity');
  for (const p of session.participants.slice(0, 6)) {
    avatar(p.avatar_id);
    const li = make('li', speaking.has(sessionKey(p.session)) ? 'is-speaking' : '');
    li.title = `${p.name}${p.pid === session.host ? ' (host)' : ''}`;
    const url = avatars.get(p.avatar_id)?.url;
    if (url) { const img = make('img'); img.src = url; img.alt = ''; li.append(img); }
    else li.textContent = p.name.slice(0, 1).toUpperCase();
    list.append(li);
  }
  if (session.participants.length > 6) list.append(make('li', 'more', `+${session.participants.length - 6}`));
  const actions = make('div', 'activity-actions');
  actions.append(iconButton('user-plus', 'Invite', invite));
  if (mode === 'mini') {
    actions.append(iconButton('maximize', 'Return to Activity', () => {
      emit('activity:layout,split');
      emit(`voice:${session.channel_id}`);
    }));
  } else {
    actions.append(
      iconButton(mode === 'focus' ? 'layout-grid' : 'maximize', mode === 'focus' ? 'Show Everyone' : 'Focus Activity', () => emit(`activity:layout,${mode === 'focus' ? 'split' : 'focus'}`)),
      iconButton('chevron-down', 'Minimize Activity', () => emit('activity:layout,mini')),
    );
  }
  actions.append(iconButton('more-horizontal', 'Activity Options', more), iconButton('log-out', 'Leave Activity', leave, 'activity-leave'));
  bar.append(title, list, actions);
}

function leave() {
  if (!session) return;
  signal('leave', { session_id: session.session_id }).catch((e) => notice(e.message));
}

function notice(text) {
  if (frame) {
    frame.note.textContent = text;
    frame.note.hidden = false;
    clearTimeout(frame.noteTimer);
    frame.noteTimer = setTimeout(() => { if (frame) frame.note.hidden = true; }, 5000);
  } else {
    emit(`activity:notice,${text}`);
  }
}

function dialog(title, className = '') {
  return extras.dialog(title, { className: `activity-dialog ${className}` });
}

function more() {
  if (!session) return;
  const d = dialog('Activity Options', 'activity-options');
  const list = make('div', 'call-more-items');
  list.setAttribute('role', 'menu');
  const item = (text, fn) => {
    const b = extras.button(text, 'call-more-item', () => { d.close(); fn(); });
    b.setAttribute('role', 'menuitem');
    list.append(b);
  };
  // Volume of the activity's sound, on this device.
  const field = make('label', 'field activity-volume');
  field.append(make('span', '', 'Activity Volume'));
  const range = make('input');
  range.type = 'range';
  range.min = '0';
  range.max = '100';
  range.value = String(prefs.volume);
  range.setAttribute('aria-label', 'Activity Volume');
  range.oninput = () => { prefs.volume = Number(range.value); savePrefs(); player.volume(); };
  field.append(range);
  if (isHost()) {
    const others = session.participants.filter((p) => p.pid !== session.you);
    for (const p of others.slice(0, 8)) item(`Make ${p.name} the Host`, () => signal('set_host', { session_id: session.session_id, pid: p.pid }).catch((e) => notice(e.message)));
    item(session.shared_control ? 'Only the Host Controls' : 'Let Everyone Control', () => signal('set_shared', { session_id: session.session_id, shared: !session.shared_control }).catch((e) => notice(e.message)));
    item('End Activity for Everyone', () => signal('end', { session_id: session.session_id }).catch((e) => notice(e.message)));
  }
  const others = session.participants.filter((p) => p.session.user_id !== me()?.session.user_id);
  for (const p of others.slice(0, 8)) item(`Report ${p.name}…`, () => emit(`privacy:report,activity,${session.channel_id},${session.session_id},${p.session.user_id}`));
  d.append(field, list, extras.button('Close', 'button secondary', () => d.close()));
  d.showModal();
}

/** Post an invite card into a conversation (the call's chat by default). */
function invite() {
  if (!session) return;
  const targets = [...document.querySelectorAll('#live-activity-targets li')].map((li) => ({ id: li.dataset.channel, label: li.dataset.label }));
  if (!targets.length) targets.push({ id: session.channel_id, label: 'This call’s chat' });
  const d = dialog('Invite to Activity');
  d.append(make('p', 'muted-copy', `Post a card with Join for ${session.activity.name}. It shows who is in it while it runs.`));
  const field = make('label', 'field');
  field.append(make('span', '', 'Post in'));
  const select = make('select');
  select.id = 'activity-invite-target';
  for (const t of targets) select.append(new Option(t.label, t.id));
  field.append(select);
  const send = extras.button('Post Invite', 'button primary', async () => {
    send.disabled = true;
    try {
      await signal('invite', { session_id: session.session_id, channel_id: select.value });
      d.close();
      notice('Invite posted.');
    } catch (e) {
      send.disabled = false;
      d.querySelector('.activity-error')?.remove();
      d.append(make('p', 'activity-error text-destructive', e.message));
    }
  });
  const row = make('div', 'activity-dialog-actions');
  row.append(extras.button('Cancel', 'button secondary', () => d.close()), send);
  d.append(field, row);
  d.showModal();
  select.focus();
}

function download(name, url) {
  if (typeof url !== 'string' || !/^data:image\/png;base64,[A-Za-z0-9+/=]+$/.test(url) || url.length > LIMITS.download * 1.4) {
    return notice('Only PNG pictures can be saved from an activity.');
  }
  const safe = name.replace(/[^\w .-]/g, '').slice(0, 64).replace(/\.png$/i, '') || 'activity';
  const d = dialog('Save Picture');
  d.append(make('p', '', `${session?.activity.name ?? 'The activity'} made ${safe}.png. Save it to this device?`));
  const row = make('div', 'activity-dialog-actions');
  row.append(extras.button('Cancel', 'button secondary', () => d.close()), extras.button('Save', 'button primary', () => {
    const a = make('a');
    a.href = url;
    a.download = `${safe}.png`;
    document.body.append(a);
    a.click();
    a.remove();
    d.close();
  }));
  d.append(row);
  d.showModal();
}

// ---- the launcher --------------------------------------------------------------------
async function launcher() {
  const channel = $('.voice-view')?.dataset.callChannel;
  if (!channel) return emit('activity:notice,Join a call to start an activity.');
  $('.activity-launcher')?.close();
  const d = dialog('Activities', 'activity-launcher');
  const status = make('p', 'muted-copy', 'Loading activities…');
  status.setAttribute('role', 'status');
  d.append(status, extras.button('Close', 'button secondary activity-close', () => d.close()));
  d.showModal();
  let catalog;
  try {
    catalog = await extras.api(`/api/v1/activities?channel_id=${channel}`);
  } catch (e) {
    status.textContent = e.message;
    return;
  }
  const running = session && session.channel_id === channel;
  if (running) {
    status.textContent = session.you != null ? `You are in ${session.activity.name}.` : `${session.activity.name} is running in this call.`;
    if (session.you == null) {
      const b = extras.button(`Join ${session.activity.name}`, 'button primary activity-join', () => { d.close(); join(session.session_id); });
      status.after(b);
    }
  } else if (!catalog.can_launch) {
    status.textContent = 'You don’t have permission to start activities here.';
  } else if (!catalog.activities.length) {
    status.textContent = 'No activities are turned on here.';
  } else {
    status.textContent = 'Everyone in the call can join what you start.';
  }
  const grid = make('ul', 'activity-grid');
  grid.setAttribute('aria-label', 'Available activities');
  for (const a of catalog.activities) {
    const li = make('li', 'activity-choice');
    const art = make('span', 'activity-choice-icon');
    art.append(/^[a-z0-9-]+$/.test(a.icon) ? icon(a.icon, 'icon size-6') : make('span', '', a.icon));
    const text = make('div', 'activity-choice-text');
    const players = a.min_players === a.max_players ? `${a.min_players} players` : `${a.min_players}–${a.max_players} players`;
    text.append(make('strong', '', a.name), make('span', 'activity-choice-description', a.description), make('small', 'activity-choice-players', `${players}${a.builtin ? '' : ' · Added by this StickTalk server'}`));
    const start = extras.button('Start', 'button primary activity-start', async () => {
      start.disabled = true;
      try {
        await signal('launch', { channel_id: channel, activity_id: a.id });
        d.close();
      } catch (e) {
        start.disabled = false;
        status.textContent = e.message;
      }
    });
    start.setAttribute('aria-label', `Start ${a.name}`);
    start.disabled = !catalog.can_launch || !!running;
    li.append(art, text, start);
    grid.append(li);
  }
  status.after(grid);
  if (catalog.results?.length) {
    const recent = make('section', 'activity-results');
    recent.append(make('h3', 'category', 'Recent Results'));
    const ul = make('ul');
    for (const r of catalog.results) ul.append(make('li', '', `${r.name}: ${r.text}`));
    recent.append(ul);
    grid.after(recent);
  }
  grid.querySelector('button:not(:disabled)')?.focus();
}

// ---- choosing a file to share (Watch Together) ---------------------------------------
function pickMedia() {
  return new Promise((resolve) => {
    const channel = session?.channel_id;
    const d = dialog('Share a Video or Audio File', 'activity-media');
    let chosen = null;
    d.addEventListener('close', () => resolve(chosen));
    const status = make('p', 'muted-copy', 'Loading files…');
    status.setAttribute('role', 'status');
    const list = make('ul', 'activity-media-list');
    const tabs = make('div', 'activity-media-tabs');
    const show = async (path, label) => {
      status.textContent = `Loading ${label}…`;
      list.replaceChildren();
      try {
        const { items } = await extras.api(path);
        status.textContent = items.length ? `${label}: choose one to add to the queue.` : `No video or audio files in ${label.toLowerCase()} yet.`;
        for (const item of items) {
          const li = make('li');
          const b = extras.button('', 'activity-media-item', () => { chosen = item; d.close(); });
          b.append(icon(item.content_type.startsWith('video/') ? 'video' : 'audio-lines'), make('span', 'truncate', item.filename), make('small', '', `${Math.max(1, Math.round(item.byte_size / 1048576))} MB`));
          li.append(b);
          list.append(li);
        }
      } catch (e) {
        status.textContent = e.message;
      }
    };
    tabs.append(
      extras.button('This Conversation', 'button secondary', () => show(`/api/v1/activities/media?channel_id=${channel}`, 'This conversation')),
      extras.button('My Uploads', 'button secondary', () => show('/api/v1/activities/media', 'Your uploads')),
    );
    const upload = make('label', 'button secondary activity-upload');
    upload.append(icon('upload'), document.createTextNode('Upload to This Call’s Chat'));
    const input = make('input');
    input.type = 'file';
    input.accept = 'video/*,audio/*';
    input.hidden = true;
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) return;
      status.textContent = `Uploading ${file.name}…`;
      try {
        chosen = await uploadToChat(file, channel);
        d.close();
      } catch (e) {
        status.textContent = e.message;
      }
    };
    upload.append(input);
    d.append(tabs, status, list, upload, extras.button('Cancel', 'button secondary', () => d.close()));
    d.showModal();
    show(`/api/v1/activities/media?channel_id=${channel}`, 'This conversation');
  });
}

/** Upload a file and send it to the call's chat, so everyone can read it there too. */
async function uploadToChat(file, channel) {
  if (!/^(video|audio)\//.test(file.type)) throw new Error('Choose a video or audio file.');
  // The server reads the fields in this order.
  const form = new FormData();
  form.append('purpose', 'message');
  form.append('channel_id', channel);
  form.append('file', file, file.name);
  const r = await fetch('/api/v1/uploads', {
    method: 'POST', credentials: 'same-origin', body: form,
    headers: { 'X-StickTalk-CSRF': extras.csrf() },
  });
  if (!r.ok) throw new Error(r.status === 413 ? 'That file is too large.' : 'The upload didn’t work.');
  // Snowflake IDs do not fit a JavaScript number: read it from the text.
  const id = /"id"\s*:\s*(\d{1,20})/.exec(await r.text())?.[1];
  if (!id) throw new Error('The upload didn’t work.');
  emit(`activity-send-file:${channel}:${id}`);
  return { attachment_id: id, filename: file.name, content_type: file.type };
}

// ---- the shared player ----------------------------------------------------------------
// An activity that writes `h/player` ({ item, playing, position, at, rate })
// gets this player: the file plays here, in the tile, through the call's
// output device at the activity volume (silent while deafened), and stays
// within a fraction of a second of everyone else by nudging the playback
// rate; it seeks only when far off. It runs on media events, not timers.
const player = {
  item: null,
  last: 0,
  update() {
    if (!frame) return;
    const p = doc.get('h/player')?.value;
    const v = frame.video;
    const item = p && typeof p.item === 'string' && /^\d{1,20}$/.test(p.item) ? p.item : null;
    if (!item) return this.stop();
    const media = doc.get(`s/media/${item}`)?.value;
    if (!media) return this.stop();
    if (this.item !== item) {
      this.item = item;
      v.src = `/api/v1/files/${item}`;
      v.ontimeupdate = () => { this.sync(); this.status(); };
      v.onplay = v.onpause = v.onended = v.ondurationchange = v.onwaiting = v.onplaying = () => this.status(true);
      v.onloadedmetadata = () => this.sync(true);
      this.output(extras.callHost()?.output);
    }
    const audio = String(media.type ?? '').startsWith('audio/');
    v.hidden = audio;
    frame.audioLabel.hidden = !audio;
    frame.audioLabel.textContent = audio ? `♪ ${media.name ?? 'Audio'}` : '';
    frame.body.dataset.player = audio ? 'audio' : 'video';
    this.volume();
    this.sync(true);
  },
  target() {
    const p = doc.get('h/player')?.value;
    if (!p) return null;
    const rate = Math.min(2, Math.max(0.25, Number(p.rate) || 1));
    const position = Math.max(0, Number(p.position) || 0);
    const at = Number(p.at) || serverNow();
    return { playing: !!p.playing, rate, time: p.playing ? position + Math.max(0, serverNow() - at) / 1000 * rate : position };
  },
  sync(hard = false) {
    const v = frame?.video;
    const t = this.target();
    if (!v || !t || !this.item || v.readyState < 1) return;
    const drift = t.time - v.currentTime;
    if (!t.playing) {
      if (!v.paused) v.pause();
      if (Math.abs(drift) > 0.15) v.currentTime = t.time;
      v.playbackRate = t.rate;
      return;
    }
    if (Math.abs(drift) > 1.5 || (hard && Math.abs(drift) > 0.5)) {
      if (Number.isFinite(v.duration) && t.time >= v.duration) return;
      v.currentTime = t.time;
      v.playbackRate = t.rate;
    } else if (Math.abs(drift) > 0.08) {
      // Catch up or slow down by at most 8%: inaudible, and no seek.
      v.playbackRate = t.rate * (1 + Math.max(-0.08, Math.min(0.08, drift * 0.5)));
    } else {
      v.playbackRate = t.rate;
    }
    if (v.paused && !v.ended) v.play().catch(() => this.status(true, 'blocked'));
  },
  status(force = false, problem = null) {
    const v = frame?.video;
    if (!v) return;
    const now = performance.now();
    if (!force && now - this.last < 250) return;
    this.last = now;
    post({ type: 'player', status: this.item ? {
      item: this.item,
      time: v.currentTime,
      duration: Number.isFinite(v.duration) ? v.duration : null,
      paused: v.paused,
      ended: v.ended,
      buffering: v.readyState < 3 && !v.paused,
      problem,
    } : null });
  },
  volume() {
    const v = frame?.video;
    if (v) {
      v.volume = Math.min(1, gain());
      v.muted = deafened || prefs.volume === 0;
    }
    post({ type: 'volume', gain: gain() });
  },
  output(id) {
    const v = frame?.video;
    if (v?.setSinkId && id) v.setSinkId(id).catch(() => {});
  },
  stop() {
    const v = frame?.video;
    this.item = null;
    if (v) {
      v.pause();
      v.removeAttribute('src');
      v.load();
      v.hidden = true;
      frame.audioLabel.hidden = true;
      delete frame.body.dataset.player;
    }
    this.status(true);
  },
};

// ---- Server Settings › Activities ----------------------------------------------------
async function settingsSection(section) {
  if (section.dataset.loaded === section.dataset.activitySettings) return;
  section.dataset.loaded = section.dataset.activitySettings;
  const guild = section.dataset.activitySettings;
  let list;
  try {
    list = await extras.api(`/api/v1/guilds/${guild}/activities`);
  } catch (e) {
    section.replaceChildren(make('p', 'muted-copy', e.message));
    return;
  }
  const rows = make('div', 'activity-settings-list');
  for (const { activity: a, enabled } of list.activities) {
    const row = make('label', 'live-checkbox activity-setting');
    const text = make('span', 'activity-setting-text');
    text.append(make('strong', '', a.name), make('small', '', a.builtin ? a.description : `${a.description ? `${a.description} · ` : ''}Added by this StickTalk server (${a.origin})`));
    const input = make('input');
    input.type = 'checkbox';
    input.checked = enabled;
    input.setAttribute('aria-label', `Allow ${a.name}`);
    input.onchange = async () => {
      input.disabled = true;
      try {
        await extras.api(`/api/v1/guilds/${guild}/activities/${encodeURIComponent(a.id)}`, { method: 'PUT', json: { enabled: input.checked } });
      } catch (e) {
        input.checked = !input.checked;
        emit(`activity:notice,${e.message}`);
      } finally {
        input.disabled = false;
      }
    };
    row.append(text, input);
    rows.append(row);
  }
  section.replaceChildren(rows);
}

// ---- for tests -----------------------------------------------------------------------
export function diagnostics() {
  return {
    session: session && { id: session.session_id, activity: session.activity.id, you: session.you, host: session.host, participants: session.participants.length },
    keys: [...doc.keys()],
    version,
    frame: frame && { ready: frame.ready, mode: frame.layer.dataset.mode, src: frame.iframe.getAttribute('src'), sandbox: frame.iframe.getAttribute('sandbox') },
    player: frame && { item: player.item, paused: frame.video.paused, time: frame.video.currentTime, volume: frame.video.volume, muted: frame.video.muted, rate: frame.video.playbackRate },
    pending: pending.size,
  };
}
globalThis.__sticktalkActivities = diagnostics;
