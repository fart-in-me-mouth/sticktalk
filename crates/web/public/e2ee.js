// End-to-end encrypted calls in the page (docs/E2EE.md): this device's identity
// key, the call's encryption worker, frame transforms on the call's senders and
// receivers, and what people see: the lock in the call, the encryption panel
// with the call code and the devices, verification, and warnings. media.js
// loads this when a call starts; nothing here exists outside a call.
import * as core from './e2ee-core.js';

// This feature's stylesheet (styles/e2ee.css) comes with it, not with the app.
if (!document.querySelector('link[data-e2ee-css]')) {
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = '/public/e2ee.css';
  link.dataset.e2eeCss = '';
  document.head.append(link);
}

const MODE = globalThis.RTCRtpScriptTransform ? 'script'
  : globalThis.RTCRtpSender?.prototype?.createEncodedStreams ? 'legacy' : null;
/** This browser can encrypt calls (the same test the app makes before Join). */
export const supported = () => !!MODE && !!globalThis.crypto?.subtle && !!globalThis.Worker;
const $ = (s) => document.querySelector(s);
const key = (s) => `${s.user_id}:${s.connection_id}`;
const sessionOf = (k) => { const [user_id, connection_id] = k.split(':'); return { user_id, connection_id }; };
let host = null;
let call = null;
let nextId = 1;

/** What media.js lets this module do in the call. */
export function bind(h) { host = h; }

// ---- preferences and trust (this device only) ----------------------------------
const PREF = 'sticktalk-e2ee';
function pref() {
  try { return { require: JSON.parse(localStorage.getItem(PREF) ?? '{}').require === true }; } catch { return { require: false }; }
}
function setPref(next) { try { localStorage.setItem(PREF, JSON.stringify(next)); } catch {} }

let dbPromise = null;
function db() {
  dbPromise ??= new Promise((resolve, reject) => {
    const open = indexedDB.open('sticktalk-e2ee', 1);
    open.onupgradeneeded = () => {
      open.result.createObjectStore('identity');
      open.result.createObjectStore('trust');
    };
    open.onsuccess = () => resolve(open.result);
    open.onerror = () => reject(open.error);
  });
  return dbPromise;
}
async function idb(store, mode, fn) {
  const d = await db();
  return new Promise((resolve, reject) => {
    const tx = d.transaction(store, mode);
    const request = fn(tx.objectStore(store));
    tx.oncomplete = () => resolve(request?.result);
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error);
  });
}
const memory = new Map();
/** The device's identity key for this account and session; a new session gets a new key. */
async function identityFor(user, deviceId) {
  try {
    const stored = await idb('identity', 'readonly', (s) => s.get(user));
    if (stored?.deviceId === deviceId && stored.privateKey instanceof CryptoKey && stored.publicRaw?.length === 65)
      return { identity: { privateKey: stored.privateKey, publicRaw: stored.publicRaw }, persistent: true };
    const identity = await core.newIdentity();
    await idb('identity', 'readwrite', (s) => s.put({ deviceId, ...identity, created: Date.now() }, user));
    return { identity, persistent: true };
  } catch {
    // No IndexedDB here (some private windows): a key for this page only.
    const id = `${user}:${deviceId}`;
    if (!memory.has(id)) memory.set(id, await core.newIdentity());
    return { identity: memory.get(id), persistent: false };
  }
}
const trustCache = new Map();
async function trust(me, peer) {
  const id = `${me}:${peer}`;
  if (!trustCache.has(id)) {
    let t = null;
    try { t = await idb('trust', 'readonly', (s) => s.get(id)); } catch {}
    trustCache.set(id, { verified: new Set(t?.verified ?? []), seen: new Set(t?.seen ?? []) });
    while (trustCache.size > 256) trustCache.delete(trustCache.keys().next().value);
  }
  return trustCache.get(id);
}
async function saveTrust(me, peer, t) {
  const bound = (set) => [...set].slice(-8);
  try { await idb('trust', 'readwrite', (s) => s.put({ verified: bound(t.verified), seen: bound(t.seen) }, `${me}:${peer}`)); } catch {}
}

// ---- the call --------------------------------------------------------------------
/** Extra RTCPeerConnection options: older Chromium needs encoded streams enabled up front. */
export function pcConfig() {
  return call?.worker && MODE === 'legacy' ? { encodedInsertableStreams: true } : {};
}
export function active() { return !!call?.worker; }

export async function event(ev) {
  const d = ev?.data;
  if (!d) return;
  if (d.type === 'welcome') return welcome(d.data);
  if (!call?.worker) {
    if (call && d.type === 'roster') { call.roster = d.data; mount(); }
    return;
  }
  if (d.type === 'roster') {
    call.roster = d.data;
    call.allow = d.data.allow_unencrypted === true;
    call.worker.postMessage({ type: 'roster', roster: d.data });
  } else if (d.type === 'relay') call.worker.postMessage({ type: 'relay', from: key(d.data.from), data: String(d.data.data) });
}

async function welcome(w) {
  const me = String(host.self());
  const base = {
    channel: String(w.channel_id), guild: String(w.guild_id ?? '0'), session: w.session, sessionKey: key(w.session),
    deviceId: String(w.device_id), allow: w.allow_unencrypted === true, me,
  };
  if (!supported()) {
    // Only possible where the call allows it: it is not encrypted, and everyone is told.
    call = { ...base, unsupported: true };
    mount();
    return;
  }
  if (call?.worker && call.sessionKey === base.sessionKey && call.channel === base.channel) return;
  const { identity, persistent } = await identityFor(me, base.deviceId);
  if (!call?.worker) {
    const worker = new Worker(new URL('./e2ee-worker.js', import.meta.url), { type: 'module', name: 'call-encryption' });
    call = { ...base, worker, ids: new WeakMap(), transceivers: new Map(), verdicts: new Map(), toasts: [], flags: new Map(), pending: new Map() };
    worker.onmessage = (m) => fromWorker(m.data);
    worker.onerror = () => host.report(new Error('Call encryption stopped working. Leave and rejoin the call.'));
  } else {
    // Moved to another voice channel: a new call, new keys; the worker and transforms stay.
    Object.assign(call, base, { state: null, roster: null });
    call.verdicts.clear();
  }
  call.persistent = persistent;
  call.ready = new Promise((resolve) => { call.announced = resolve; });
  call.worker.postMessage({ type: 'init', channel: base.channel, session: base.session, deviceId: base.deviceId, identity });
  mount();
}

function fromWorker(m) {
  if (!call?.worker) return;
  switch (m.type) {
    case 'announce':
      call.fp = m.fp;
      host.signal('e2ee', { type: 'announce', data: m.announce });
      call.announced?.();
      break;
    case 'relay':
      host.signal('e2ee', { type: 'relay', data: { to: sessionOf(m.to), data: m.data } });
      break;
    case 'state':
      call.state = m;
      review(m).catch(() => {});
      if (!m.encrypted && pref().require) {
        host.leave();
        host.report(new Error('You left: this call isn’t end-to-end encrypted, and you chose to join only encrypted calls (the lock in a call opens that setting).'));
        return;
      }
      mount();
      break;
    case 'keyframe': {
      const t = call.transceivers.get(m.id);
      if (t?.mid) host.keyframe(t.mid);
      break;
    }
    case 'verdict':
      call.verdicts.set(`${m.member}:${m.kind}`, { forged: m.forged, unverified: m.unverified });
      mount();
      break;
    case 'diagnostics':
      call.pending.get(m.id)?.(m);
      call.pending.delete(m.id);
      break;
    case 'error':
      host.report(new Error(`Call encryption: ${m.message}`));
      break;
  }
}

/** Verified, changed and new keys, decided against this device's own records. */
async function review(state) {
  const me = call.me;
  for (const m of state.members) {
    if (m.state !== 'ok' || !m.fp) continue;
    const t = await trust(me, m.user);
    const verified = t.verified.has(m.fp);
    const changed = !verified && t.verified.size > 0;
    const fresh = !verified && t.seen.size > 0 && !t.seen.has(m.fp);
    const before = call?.flags.get(m.key);
    call?.flags.set(m.key, { verified, changed, fresh: fresh || before?.fresh === true });
    if (!t.seen.has(m.fp)) {
      if (fresh && !before) toast(m.user === me ? 'Your other device joined with a new key.' : `${name(m.user)} joined from a new device.`);
      t.seen.add(m.fp);
      await saveTrust(me, m.user, t);
    }
  }
  mount();
}
const name = (user) => host.person(user)?.name ?? 'Someone';

function toast(text) {
  call.toasts.push({ text, until: Date.now() + 6000 });
  while (call.toasts.length > 3) call.toasts.shift();
  mount();
  setTimeout(() => mount(), 6100);
}

// ---- transforms --------------------------------------------------------------------
function attach(target, side, kind, member) {
  if (!call?.worker || !target) return null;
  let id = call.ids.get(target);
  if (id) {
    if (side === 'recv') call.worker.postMessage({ type: 'rebind', id, member, kind });
    return id;
  }
  id = nextId++;
  call.ids.set(target, id);
  const options = { side, id, kind, member };
  if (MODE === 'script') target.transform = new RTCRtpScriptTransform(call.worker, options);
  else {
    const { readable, writable } = target.createEncodedStreams();
    call.worker.postMessage({ type: 'stream', ...options, readable, writable }, [readable, writable]);
  }
  return id;
}
/** One of this device's published tracks (microphone, camera, screen, stream sound). */
export function sender(rtpSender, kind) { attach(rtpSender, 'send', kind, null); }
/** A subscription: the server says it carries `session`'s `kind` track. */
export function receiver(transceiver, session, kind) {
  const id = attach(transceiver.receiver, 'recv', kind, key(session));
  if (id) call.transceivers.set(id, transceiver);
}

export function stop() {
  if (!call) return;
  call.worker?.postMessage({ type: 'stop' });
  call.worker?.terminate();
  for (const resolve of call.pending?.values() ?? []) resolve(null);
  call = null;
  document.querySelector('#e2ee-status')?.remove();
  document.querySelector('.e2ee-dialog')?.close();
  for (const b of document.querySelectorAll('.tile-e2ee')) b.remove();
}

/** Tests: start counting again (the measurement windows). */
export function resetCounters() { call?.worker?.postMessage({ type: 'reset-timing' }); }
/** For tests and the diagnostics panel: what the worker has counted. */
export function diagnostics() {
  if (!call?.worker) return Promise.resolve({ mode: MODE, active: false, unsupported: !!call?.unsupported });
  const id = nextId++;
  return new Promise((resolve) => {
    call.pending.set(id, (d) => resolve(d && {
      mode: MODE, active: true, persistent: call?.persistent, state: call?.state, flags: call ? Object.fromEntries(call.flags) : {},
      verdicts: call ? Object.fromEntries(call.verdicts) : {}, ...d,
    }));
    call.worker.postMessage({ type: 'diagnostics', id });
    setTimeout(() => { call?.pending.get(id)?.(null); call?.pending.delete(id); }, 3000);
  });
}

// ---- what people see ------------------------------------------------------------
function el(tag, attrs = {}, ...children) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v == null || v === false) continue;
    if (k === 'text') e.textContent = v;
    else if (k.startsWith('on')) e[k] = v;
    else e.setAttribute(k, v === true ? '' : v);
  }
  e.append(...children.filter((c) => c != null));
  return e;
}
function glyph(id, cls = 'icon size-4') {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('class', cls);
  svg.setAttribute('aria-hidden', 'true');
  const use = document.createElementNS(svg.namespaceURI, 'use');
  use.setAttribute('href', `#i-${id}`);
  svg.append(use);
  return svg;
}

/** The call's encryption state in words, with what needs attention. */
function summary() {
  if (!call) return null;
  if (call.unsupported) return { level: 'off', label: 'Not end-to-end encrypted', reason: 'Your browser can’t encrypt calls, and this call allows that. Use the StickTalk Windows app or a current Chrome, Edge or Firefox to encrypt.' };
  const s = call.state;
  if (!s) return { level: 'pending', label: 'Encrypting…' };
  const names = (keys) => [...new Set(keys.map((k) => name(sessionOf(k).user_id)))].join(', ');
  if (!s.encrypted) return { level: 'off', label: 'Not end-to-end encrypted', reason: `${names(s.unsupported)}’s app can’t encrypt calls, and this ${call.guild === '0' ? 'conversation' : 'server'} allows unencrypted calls. Audio and video in this call can be read by the server.` };
  const warnings = [];
  for (const m of s.members) {
    const f = call.flags.get(m.key);
    if (f?.changed) warnings.push(`${name(m.user)}’s encryption key changed, or ${name(m.user)} is using a device you haven’t verified. Verify again before trusting this call.`);
    if (m.state === 'invalid') warnings.push(`A device claiming to be ${name(m.user)} sent an invalid key. It gets no keys and can’t hear this call.`);
  }
  for (const [k, v] of call.verdicts) {
    const [user] = k.split(':');
    if (v.forged) warnings.push(`Media claiming to be from ${name(user)} failed its signature check. It may have been forged, and it was muted.`);
    else if (v.unverified) warnings.push(`${name(user)}’s media can’t be verified right now.`);
  }
  if (s.selfAltered) warnings.push('The server changed your own key announcement. Leave the call.');
  const cannot = s.unsupported.length ? `${names(s.unsupported)} can’t hear or see this encrypted call: their app can’t encrypt.` : null;
  const verifiedAll = s.members.length > 0 && s.members.every((m) => m.state !== 'ok' || call.flags.get(m.key)?.verified);
  return { level: warnings.length ? 'warn' : 'on', label: warnings.length ? 'Check encryption' : 'End-to-end encrypted', warnings, cannot, verifiedAll };
}

export function mount() {
  if (!call) return;
  const view = $('.voice-view');
  if (!view) return;
  const stage = view.querySelector('#live-media-stage');
  let bar = view.querySelector('#e2ee-status');
  if (!bar) {
    bar = el('div', { id: 'e2ee-status', class: 'e2ee-status' });
    view.insertBefore(bar, stage);
  }
  const s = summary();
  const icon = s.level === 'off' ? 'lock-open' : s.level === 'warn' ? 'shield' : s.verifiedAll ? 'shield-check' : 'lock';
  const pill = el('button', {
    type: 'button', class: `e2ee-pill is-${s.level}`, 'data-e2ee': 'panel',
    'aria-label': `${s.label}. Show encryption details`, title: 'Encryption details', onclick: () => panel(),
  }, glyph(icon), el('span', { text: s.label }));
  const lines = [];
  if (s.reason) lines.push(el('p', { class: 'e2ee-banner is-off', role: 'status', text: `Not end-to-end encrypted: ${s.reason}` }));
  for (const w of s.warnings ?? []) lines.push(el('p', { class: 'e2ee-banner is-warn', role: 'alert', text: w }));
  if (s.cannot) lines.push(el('p', { class: 'e2ee-banner', role: 'status', text: s.cannot }));
  const now = Date.now();
  if (call.toasts) call.toasts = call.toasts.filter((t) => t.until > now);
  for (const t of call.toasts ?? []) lines.push(el('p', { class: 'e2ee-toast', role: 'status', text: t.text }));
  const signature = JSON.stringify([s.level, s.label, s.reason, s.warnings, s.cannot, s.verifiedAll, (call.toasts ?? []).map((t) => t.text)]);
  if (bar.dataset.signature !== signature) {
    bar.dataset.signature = signature;
    bar.replaceChildren(pill, ...lines);
  }
  // A mark on each person's tile: verified, or a warning.
  for (const tile of view.querySelectorAll('.participant-tile[data-participant]')) {
    if (tile.dataset.self) continue;
    const f = call.flags?.get(tile.dataset.participant);
    const forged = [...(call.verdicts ?? [])].some(([k, v]) => v.forged && k.startsWith(`${tile.dataset.participant}:`));
    const state = forged || f?.changed ? 'warn' : f?.verified ? 'verified' : '';
    let badge = tile.querySelector('.tile-e2ee');
    if (!state) { badge?.remove(); continue; }
    if (badge?.dataset.state === state) continue;
    badge?.remove();
    badge = el('span', { class: `tile-e2ee is-${state}`, 'data-state': state, title: state === 'verified' ? 'Verified' : 'Check encryption', 'aria-label': state === 'verified' ? 'Verified' : 'Encryption warning' },
      glyph(state === 'verified' ? 'shield-check' : 'shield', 'icon size-3.5'));
    tile.append(badge);
  }
  const open = document.querySelector('.e2ee-dialog[open]');
  if (open?.refresh) open.refresh();
}

function dialog(title, cls, ...content) {
  document.querySelector(`.${cls}`)?.close();
  const d = el('dialog', { class: `confirm-dialog e2ee-dialog ${cls}`, 'aria-labelledby': `${cls}-title` },
    el('h2', { id: `${cls}-title`, text: title }), ...content);
  d.addEventListener('close', () => { d.dispatchEvent(new Event('e2ee-closed')); d.remove(); });
  d.addEventListener('click', (e) => { if (e.target === d) d.close(); });
  d.addEventListener('keydown', (e) => e.stopPropagation());
  (document.querySelector('.live-modal') ?? document.body).append(d);
  d.showModal();
  return d;
}
const groups = (code) => el('p', { class: 'e2ee-code', 'aria-label': code.replace(/ /g, ', ') }, ...code.split(' ').map((g) => el('span', { text: g })));

/** The encryption panel: state, the call code, every device holding keys, the settings. */
export function panel() {
  if (!call) return;
  const body = el('div', { class: 'e2ee-panel-body' });
  const d = dialog('End-to-End Encryption', 'e2ee-panel', body,
    el('div', { class: 'confirm-actions' }, el('button', { type: 'button', class: 'button primary', text: 'Done', onclick: () => d.close() })));
  let last = '';
  d.refresh = () => {
    // Re-drawn only when what it shows changed, so focus stays where it was.
    const signature = JSON.stringify([summary(), call?.state, call ? [...call.flags] : [], call?.persistent]);
    if (signature === last) return;
    last = signature;
    render(body, d).catch(() => {});
  };
  d.refresh();
}
async function render(body, d) {
  const focused = document.activeElement?.id || document.activeElement?.dataset?.member;
  const s = summary();
  const parts = [el('p', { class: `e2ee-panel-state is-${s.level}` }, glyph(s.level === 'off' ? 'lock-open' : s.level === 'warn' ? 'shield' : 'lock'),
    el('span', { text: s.level === 'off' ? `Not end-to-end encrypted. ${s.reason}` : s.level === 'pending' ? 'Setting up encryption…'
      : 'Audio, video and screen shares in this call are encrypted on each device. The server forwards them but can’t decrypt them.' }))];
  for (const w of s.warnings ?? []) parts.push(el('p', { class: 'e2ee-banner is-warn', text: w }));
  const st = call.state;
  if (st?.code && st.encrypted) {
    parts.push(el('h3', { class: 'e2ee-heading', text: 'Call Verification Code' }), groups(st.code),
      el('p', { class: 'muted-copy', text: 'Everyone in the call sees the same code. Read it aloud together: if it matches, nobody else is listening in, not even the server.' }));
  }
  if (st) {
    const rows = [el('li', { class: 'e2ee-device' },
      el('span', { class: 'e2ee-device-name' }, el('strong', { text: `${name(call.me)} (you)` }), el('small', { text: `This device · ${core.formatFingerprint(core.fromHex(st.self.fp))}` })),
      el('span', { class: 'e2ee-badge', text: call.persistent ? 'Your key' : 'Key for this tab only' }))];
    for (const m of st.members) {
      const f = call.flags.get(m.key) ?? {};
      const badge = m.state === 'unsupported' ? ['Can’t encrypt', 'is-off'] : m.state === 'invalid' ? ['Invalid key', 'is-warn']
        : f.changed ? ['Key changed', 'is-warn'] : f.verified ? ['Verified', 'is-on'] : f.fresh ? ['New device', 'is-warn'] : ['Not verified', ''];
      const own = m.user === call.me;
      rows.push(el('li', { class: 'e2ee-device', 'data-member': m.key },
        el('span', { class: 'e2ee-device-name' }, el('strong', { text: own ? `${name(m.user)} (your other device)` : name(m.user) }),
          el('small', { text: [m.deviceName, m.fp ? core.formatFingerprint(core.fromHex(m.fp)) : null].filter(Boolean).join(' · ') })),
        el('span', { class: `e2ee-badge ${badge[1]}`, text: badge[0] }),
        m.state === 'ok' ? el('button', { type: 'button', class: 'button secondary e2ee-verify', 'data-e2ee': 'verify', text: f.verified ? 'Details' : 'Verify', 'aria-label': `Verify ${name(m.user)}`, onclick: () => verify(m) }) : null));
    }
    parts.push(el('h3', { class: 'e2ee-heading', text: `Devices With Keys (${1 + st.members.filter((m) => m.state === 'ok').length})` }), el('ul', { class: 'e2ee-devices' }, ...rows));
  }
  const require = el('input', { type: 'checkbox', id: 'e2ee-require', checked: pref().require, onchange: (e) => setPref({ ...pref(), require: e.target.checked }) });
  parts.push(el('label', { class: 'live-checkbox e2ee-option', for: 'e2ee-require' }, require, el('span', { text: 'Only join end-to-end encrypted calls (this device)' })));
  const setting = el('div', { class: 'e2ee-setting' });
  parts.push(setting);
  parts.push(el('p', { class: 'muted-copy e2ee-scope', text: 'Not covered: text chat, the soundboard and entrance sounds, Stage channels, and who is in a call and when. StickTalk’s app is delivered by its server, so this protects you from someone who records traffic or breaks into the server later, not from a server that ships you a modified app.' }));
  body.replaceChildren(...parts);
  if (focused) (document.getElementById(focused) ?? body.querySelector(`[data-member="${CSS.escape(focused)}"] button`))?.focus();
  serverSetting(setting, d).catch(() => setting.replaceChildren());
}
async function serverSetting(box, d) {
  const dm = call.guild === '0';
  const path = dm ? `/api/v1/channels/${call.channel}/e2ee` : `/api/v1/guilds/${call.guild}/e2ee`;
  const { api } = await import('./callextras.js');
  // Asked once each time the panel opens.
  d.setting ??= api(path);
  const s = await d.setting;
  if (!s.can_manage || !box.isConnected) return;
  const input = el('input', { type: 'checkbox', id: 'e2ee-allow', checked: s.allow_unencrypted });
  input.onchange = async () => {
    input.disabled = true;
    try {
      const next = await api(path, { method: 'PUT', json: { allow_unencrypted: input.checked } });
      d.setting = Promise.resolve(next);
    } catch (e) { input.checked = !input.checked; host.report(e); }
    input.disabled = false;
  };
  box.replaceChildren(el('label', { class: 'live-checkbox e2ee-option', for: 'e2ee-allow' }, input,
    el('span', { text: `Allow unencrypted calls in this ${dm ? 'conversation' : 'server'} for apps that can’t encrypt. Everyone in such a call is told it isn’t encrypted.` })));
}

/** Pairwise verification of one device: digits, a QR code, the scanner. */
async function verify(m) {
  const me = { user: call.me, fp: core.fromHex(call.state.self.fp) };
  const peer = { user: m.user, fp: core.fromHex(m.fp) };
  const number = await core.safetyNumber(me, peer);
  const t = await trust(call.me, m.user);
  const status = el('p', { class: 'muted-copy', role: 'status', text: t.verified.has(m.fp) ? `You verified ${name(m.user)} on this device.` : '' });
  const qrBox = el('div', { class: 'e2ee-qr-box' });
  const mark = el('button', { type: 'button', class: 'button primary', 'data-e2ee': 'mark' });
  const scanBtn = el('button', { type: 'button', class: 'button secondary', 'data-e2ee': 'scan', text: 'Scan Their Code' });
  const update = () => { mark.textContent = t.verified.has(m.fp) ? 'Remove Verification' : 'They Match: Mark as Verified'; };
  update();
  mark.onclick = async () => {
    if (t.verified.has(m.fp)) t.verified.delete(m.fp); else t.verified.add(m.fp);
    await saveTrust(call.me, m.user, t);
    status.textContent = t.verified.has(m.fp) ? `${name(m.user)} is verified on this device.` : 'Verification removed.';
    update();
    await review(call.state);
  };
  const d = dialog(`Verify ${name(m.user)}`, 'e2ee-verify-dialog',
    el('p', { text: `Compare these numbers with ${name(m.user)}, in person or on a call you already trust. They match only if nobody has swapped your keys.` }),
    groups(number), qrBox, status,
    el('p', { class: 'muted-copy', text: 'Verification is stored on this device only.' }),
    el('div', { class: 'confirm-actions' }, el('button', { type: 'button', class: 'button secondary', text: 'Close', onclick: () => d.close() }), scanBtn, mark));
  scanBtn.onclick = () => scan(d, status, me, m, t, update);
  const { svg } = await import('./e2ee-qr.js');
  qrBox.replaceChildren(svg(core.verifyPayload(me, peer), `QR code for ${name(m.user)} to scan`),
    el('small', { text: `${name(m.user)} can scan this with Verify → Scan Their Code.` }));
}
async function scan(d, status, me, m, t, update) {
  const video = el('video', { class: 'e2ee-scanner', playsinline: true, muted: true, 'aria-label': 'Camera' });
  d.querySelector('.e2ee-qr-box')?.replaceChildren(video);
  let stream;
  try { stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: 'environment' }, audio: false }); } catch {
    status.textContent = 'The camera isn’t available. Compare the numbers instead.';
    return;
  }
  const stopScan = () => stream.getTracks().forEach((x) => x.stop());
  d.addEventListener('e2ee-closed', stopScan);
  video.srcObject = stream;
  await video.play().catch(() => {});
  status.textContent = `Point the camera at the code on ${name(m.user)}’s screen.`;
  let detect;
  if ('BarcodeDetector' in globalThis && (await BarcodeDetector.getSupportedFormats().catch(() => [])).includes('qr_code')) {
    const detector = new BarcodeDetector({ formats: ['qr_code'] });
    detect = async () => (await detector.detect(video))[0]?.rawValue;
  } else {
    const jsQR = (await import('/public/vendor/jsqr-1.4.0.js')).default;
    const canvas = document.createElement('canvas');
    detect = async () => {
      if (!video.videoWidth) return null;
      canvas.width = video.videoWidth; canvas.height = video.videoHeight;
      const c = canvas.getContext('2d', { willReadFrequently: true });
      c.drawImage(video, 0, 0);
      return jsQR(c.getImageData(0, 0, canvas.width, canvas.height).data, canvas.width, canvas.height, { inversionAttempts: 'dontInvert' })?.data;
    };
  }
  // Runs only while the scanner is open.
  const tick = async () => {
    if (!d.open || !video.isConnected) return stopScan();
    const text = await detect().catch(() => null);
    const p = text ? core.parseVerifyPayload(text) : null;
    if (text && !p) status.textContent = 'That isn’t a StickTalk verification code.';
    if (p) {
      stopScan();
      if (p.user === String(m.user) && p.fp === m.fp && p.peer === String(me.user) && p.peerFp === core.hex(me.fp)) {
        t.verified.add(m.fp);
        await saveTrust(call.me, m.user, t);
        status.textContent = `The keys match. ${name(m.user)} is verified on this device.`;
        update();
        await review(call.state);
      } else status.textContent = `These keys don’t match the ones in this call. Someone may be intercepting it: don’t mark ${name(m.user)} as verified.`;
      return;
    }
    setTimeout(tick, 200);
  };
  tick();
}
