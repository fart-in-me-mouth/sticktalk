// Signed-in account security and personal data (docs/features/authsec.md):
// passkeys, the account's email address, scanning a QR code to sign in another
// device, the account switcher and "Request my data". Rust renders empty
// [data-authsec-root] containers and [data-authsec] buttons; live.js loads
// this module the first time one appears or is pressed. Everything is drawn
// from cached state on each mount, because a render replaces the containers.
import {fromB64, toB64, requestOptions, passkeysSupported} from './authsec-login.js';

const $ = (s, root = document) => root.querySelector(s);
const el = (tag, attrs = {}, ...children) => {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'class') e.className = v;
    else if (k === 'text') e.textContent = v;
    else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
    else if (v !== false && v != null) e.setAttribute(k, v === true ? '' : v);
  }
  e.append(...children.filter(c => c != null && c !== false));
  return e;
};
const icon = (name, cls = 'icon size-5') => {
  const NS = 'http://www.w3.org/2000/svg', svg = document.createElementNS(NS, 'svg');
  svg.setAttribute('class', cls); svg.setAttribute('aria-hidden', 'true'); svg.setAttribute('viewBox', '0 0 24 24');
  const use = document.createElementNS(NS, 'use'); use.setAttribute('href', `#i-${name}`); svg.append(use);
  return svg;
};
const csrf = () => document.cookie.split(';').map(v => v.trim()).find(v => v.startsWith('__Host-sticktalk_csrf='))?.slice(22) ?? '';
// Snowflake IDs do not fit in a JS number: keep every *id as its exact digits.
const parse = (text) => JSON.parse(text.replace(/("(?:\w+_)?id"\s*:\s*)(\d{16,})/g, '$1"$2"'));
/** Same-origin API call. `raw` is JSON text (so IDs keep every digit). */
async function api(method, path, raw) {
  const headers = {};
  if (raw !== undefined) headers['Content-Type'] = 'application/json';
  if (method !== 'GET') headers['X-StickTalk-CSRF'] = csrf();
  let r;
  try { r = await fetch(`/api/v1/${path}`, {method, credentials: 'same-origin', cache: 'no-store', headers, body: raw}); }
  catch { throw Object.assign(Error('Can’t reach the server. Check your connection and try again.'), {code: 'offline'}); }
  const text = r.status === 204 ? '' : await r.text();
  let data = null; try { data = text ? parse(text) : null; } catch {}
  if (!r.ok) throw Object.assign(Error(messageFor(data?.code)), {code: data?.code, status: r.status, retry: data?.retry_after_ms});
  return data;
}
const json = (v) => JSON.stringify(v);
function messageFor(code) {
  return {
    rate_limited: 'Too many attempts. Wait a minute and try again.',
    limit_exceeded: 'You’ve reached the limit for this.',
    feature_disabled: 'This server doesn’t offer that.',
    not_found: 'That is no longer available.',
    conflict: 'Something changed meanwhile. Try again.',
    already_exists: 'That passkey is already registered.',
    invalid_request: 'Check what you entered and try again.',
    unavailable: 'The server is busy. Try again shortly.',
  }[code] ?? 'Something went wrong. Try again.';
}
const when = (ms) => new Date(ms).toLocaleString(undefined, {dateStyle: 'medium', timeStyle: 'short'});
const day = (ms) => new Date(ms).toLocaleDateString(undefined, {dateStyle: 'medium'});
const size = (n) => n < 1024 * 1024 ? `${Math.max(1, Math.round(n / 1024))} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`;
const note = (text = '', error = false) => el('p', {class: `authsec-note mt-3 text-[13px] ${error ? 'text-destructive' : 'text-subtle'} empty:hidden`, role: error ? 'alert' : 'status', text});

let emit = () => {};
let methods = null, passkeys = null, email = null, exportStatus = null, flash = {};
const roots = () => document.querySelectorAll('[data-authsec-root]');

// ----- Re-authentication -----------------------------------------------------------
// Sensitive actions need fresh proof of the existing login: the password, or
// Google. The server sets the proof as an HttpOnly cookie for five minutes,
// bound to this session and this one operation.
async function loadMethods() {
  methods ??= await api('GET', 'auth/methods').catch(() => ({has_password: true, google_linked: false}));
  return methods;
}
/** Run `action`; if the server asks for re-authentication, ask once, then retry. */
async function withReauth(operation, action) {
  try { return await action(); }
  catch (e) { if (e.code !== 'invalid_credentials') throw e; }
  await reauth(operation);
  return action();
}
const cancelled = () => Object.assign(Error(''), {code: 'cancelled'});
/** "Confirm it's you" as a dialog, so a page render cannot take it away. */
function reauth(operation) {
  return new Promise(async (resolve, reject) => {
    const m = await loadMethods();
    let done = false;
    const status = note();
    const google = m.google_linked ? el('a', {class: 'button secondary', href: `/api/v1/browser/google/start?operation=${operation}`}, 'Verify with Google') : null;
    const password = m.has_password ? el('div', {class: 'field'}, el('label', {for: 'authsec-reauth-password', text: 'Current Password'}),
      el('input', {id: 'authsec-reauth-password', type: 'password', autocomplete: 'current-password', maxlength: '128', required: true})) : null;
    const form = el('form', {class: 'authsec-reauth'},
      el('p', {text: m.has_password ? 'Enter your password to continue.' : 'Verify with Google to continue, then try again.'}),
      el('input', {type: 'text', name: 'username', autocomplete: 'username', hidden: true}),
      password, status, actions(el('button', {type: 'button', class: 'button secondary', text: 'Cancel', onclick: () => d.close()}), google,
        m.has_password ? el('button', {type: 'submit', class: 'button primary', text: 'Continue'}) : null));
    form.onsubmit = async (e) => {
      e.preventDefault();
      const submit = $('[type=submit]', form); submit.disabled = true;
      try {
        await api('POST', 'auth/reauth', json({operation, current_password: $('#authsec-reauth-password', form).value}));
        done = true; d.close(); resolve();
      } catch (err) {
        status.className = 'authsec-note mt-3 text-[13px] text-destructive'; status.setAttribute('role', 'alert');
        status.textContent = err.code === 'invalid_credentials' ? 'That password isn’t right.' : err.message;
        submit.disabled = false;
      }
    };
    const d = showDialog('Confirm it’s you', form);
    d.addEventListener('authsec-closed', () => { if (!done) reject(cancelled()); });
    ($('input:not([hidden])', form) ?? $('a,button', form))?.focus();
  });
}

// ----- Passkeys ---------------------------------------------------------------------
function creationOptions(text) {
  const o = JSON.parse(text).publicKey;
  o.challenge = fromB64(o.challenge);
  o.user = {...o.user, id: fromB64(o.user.id)};
  o.excludeCredentials = (o.excludeCredentials ?? []).map(c => ({...c, id: fromB64(c.id)}));
  return o;
}
function attestationJSON(c) {
  return JSON.stringify({
    // No extension outputs: the server asks for none it relies on.
    id: c.id, rawId: toB64(c.rawId), type: c.type, extensions: {},
    response: {attestationObject: toB64(c.response.attestationObject), clientDataJSON: toB64(c.response.clientDataJSON)},
  });
}
function defaultName() {
  const ua = navigator.userAgent;
  const os = /iPhone|iPad/.test(ua) ? 'iPhone or iPad' : /Android/.test(ua) ? 'Android' : /Windows/.test(ua) ? 'Windows' : /Mac OS X/.test(ua) ? 'Mac' : /CrOS/.test(ua) ? 'Chromebook' : 'this device';
  return `Passkey on ${os}`;
}
async function addPasskey(name) {
  const start = await withReauth('passkeys', () => api('POST', 'users/@me/passkeys/register/start', json({name})));
  let credential;
  try { credential = await navigator.credentials.create({publicKey: creationOptions(start.public_key)}); }
  catch (e) {
    if (e?.name === 'InvalidStateError') throw Error('This device already has a passkey for your account.');
    if (e?.name === 'NotAllowedError' || e?.name === 'AbortError') throw Object.assign(Error('Adding the passkey was cancelled.'), {code: 'cancelled'});
    throw Error('This device couldn’t create a passkey.');
  }
  return api('POST', 'users/@me/passkeys/register/finish', json({challenge_id: start.challenge_id, credential: attestationJSON(credential)}));
}
function passkeySection() {
  const status = note(flash.passkeys ?? '', !!flash.passkeysError);
  const list = el('ul', {class: 'authsec-passkeys mt-4 grid', 'aria-label': 'Your passkeys'});
  for (const p of passkeys ?? []) {
    const rename = el('button', {type: 'button', class: 'icon-button', 'aria-label': `Rename ${p.name}`, onclick: () => renamePasskey(p)}, icon('pencil', 'icon size-4'));
    const remove = el('button', {type: 'button', class: 'button danger-text', 'aria-label': `Remove ${p.name}`, text: 'Remove', onclick: () => removePasskey(p)});
    list.append(el('li', {class: 'settings-list-row flex flex-wrap items-center gap-3 border-b py-4', 'data-passkey': p.id},
      el('span', {class: 'grid size-10 flex-none place-items-center rounded-full bg-muted text-subtle'}, icon('key-round')),
      el('span', {class: 'flex min-w-24 flex-1 flex-col gap-1 wrap-anywhere'},
        el('strong', {class: 'font-semibold', text: p.name}),
        el('small', {class: 'text-subtle', text: `Added ${day(p.created_at)} · ${p.last_used_at ? `Last used ${when(p.last_used_at)}` : 'Not used yet'}${p.backed_up ? ' · Synced' : ''}`})),
      rename, remove));
  }
  const supported = passkeysSupported();
  const add = el('button', {type: 'button', class: 'button secondary mt-4', disabled: !supported || (passkeys?.length ?? 0) >= 10}, icon('plus', 'icon size-4'), 'Add a Passkey');
  add.onclick = async () => {
    const name = await promptDialog('Add a Passkey', 'Passkey Name', defaultName(),
      {hint: 'So you can tell your passkeys apart, for example “Work laptop”.', submit: 'Continue', id: 'authsec-passkey-name'});
    if (!name) return;
    setFlash('passkeys', 'Follow your device’s prompt…');
    try {
      await addPasskey(name);
      passkeys = await api('GET', 'users/@me/passkeys');
      setFlash('passkeys', `Added “${name}”. You can now sign in with it on the login screen.`);
    } catch (err) { setFlash('passkeys', err.code === 'cancelled' ? '' : err.message, err.code !== 'cancelled'); }
  };
  return el('section', {class: 'authsec-section', 'aria-labelledby': 'authsec-passkeys-title'},
    el('h2', {id: 'authsec-passkeys-title', text: 'Passkeys'}),
    el('p', {class: 'muted-copy mt-2', text: 'Sign in without typing your password, using your fingerprint, face, screen lock or a security key. Passkeys are optional: your password and Google sign-in keep working.'}),
    supported ? null : el('p', {class: 'muted-copy mt-2', text: 'This browser doesn’t support passkeys.'}),
    (passkeys?.length ?? 0) ? list : el('p', {class: 'mt-4 text-sm', text: 'No passkeys yet.'}),
    add, status);
}
async function renamePasskey(p) {
  const name = await promptDialog('Rename Passkey', 'Passkey Name', p.name);
  if (!name) return;
  try { passkeys = await api('PATCH', `users/@me/passkeys/${p.id}`, json({name})); setFlash('passkeys', 'Renamed.'); }
  catch (e) { setFlash('passkeys', e.message, true); }
}
async function removePasskey(p) {
  if (!await confirmDialog(`Remove “${p.name}”?`, 'You won’t be able to sign in with it anymore. You can add it again later.', 'Remove Passkey')) return;
  try {
    passkeys = await withReauth('passkeys', () => api('DELETE', `users/@me/passkeys/${p.id}`));
    setFlash('passkeys', `Removed “${p.name}”.`);
  } catch (e) { if (e.code !== 'cancelled') setFlash('passkeys', e.message, true); }
}

// ----- Email ----------------------------------------------------------------------
function emailSection() {
  const status = note(flash.email ?? '', !!flash.emailError);
  const head = [el('h2', {id: 'authsec-email-title', text: 'Email for Password Reset'})];
  if (!email?.smtp_enabled) {
    return el('section', {class: 'authsec-section', 'aria-labelledby': 'authsec-email-title'}, ...head,
      el('p', {class: 'muted-copy mt-2', text: 'This server doesn’t send email. If you forget your password, the person who runs it can give you a reset code.'}));
  }
  const edit = (label) => el('button', {type: 'button', class: 'button secondary', text: label, onclick: async () => {
    const value = await promptDialog(label, 'Email Address', email.pending ?? email.email ?? '',
      {type: 'email', maxlength: '254', autocomplete: 'email', submit: 'Send Confirmation Link', id: 'authsec-email',
        hint: 'We’ll email a link to confirm it’s yours.'});
    if (!value) return;
    try {
      email = await withReauth('email', () => api('PUT', 'users/@me/email', json({email: value})));
      setFlash('email', `We sent a link to ${email.pending}. Open it within 24 hours to confirm the address.`);
    } catch (err) { if (err.code !== 'cancelled') setFlash('email', err.code === 'invalid_request' ? 'That doesn’t look like an email address.' : err.message, true); }
  }});
  const remove = el('button', {type: 'button', class: 'button danger-text', text: 'Remove', onclick: async () => {
    if (!await confirmDialog('Remove your email address?', 'You won’t be able to reset your password by email.', 'Remove')) return;
    try { email = await withReauth('email', () => api('DELETE', 'users/@me/email')); setFlash('email', 'Removed.'); }
    catch (e) { if (e.code !== 'cancelled') setFlash('email', e.message, true); }
  }});
  const current = email.email
    ? el('p', {class: 'mt-4 flex flex-wrap items-center gap-2 text-sm'}, el('strong', {class: 'wrap-anywhere', text: email.email}),
      el('span', {class: 'rounded-full bg-muted px-2 py-0.5 text-xs text-subtle'}, icon('check', 'icon mr-1 inline size-3'), 'Confirmed'))
    : el('p', {class: 'mt-4 text-sm', text: 'No email address yet.'});
  const pending = email.pending ? el('p', {class: 'mt-2 text-[13px] text-subtle wrap-anywhere', text: `Waiting for you to open the link sent to ${email.pending}.`}) : null;
  return el('section', {class: 'authsec-section', 'aria-labelledby': 'authsec-email-title'}, ...head,
    el('p', {class: 'muted-copy mt-2', text: 'Add an address to reset a forgotten password by email. Only you can see it.'}),
    current, pending, el('div', {class: 'mt-4 flex flex-wrap gap-2'}, edit(email.email || email.pending ? 'Change Email' : 'Add Email'), email.email || email.pending ? remove : null), status);
}

// ----- QR code: scan, then confirm -------------------------------------------------
function qrSection() {
  return el('section', {class: 'authsec-section', 'aria-labelledby': 'authsec-qr-scan-title'},
    el('h2', {id: 'authsec-qr-scan-title', text: 'Sign In on Another Device'}),
    el('p', {class: 'muted-copy mt-2', text: 'On a computer’s StickTalk login page, choose the QR code and scan it here. You’ll see which device is asking before anything happens.'}),
    el('button', {type: 'button', class: 'button secondary mt-4', 'data-authsec': 'scan'}, icon('smartphone', 'icon size-4'), 'Scan QR Code'));
}

let dialog = null;
/** One app dialog at a time, kept inside any open settings sheet (see live.js). */
function showDialog(title, ...content) {
  dialog?.close();
  const d = el('dialog', {id: 'authsec-dialog', class: 'confirm-dialog authsec-dialog', 'aria-labelledby': 'authsec-dialog-title'},
    el('h2', {id: 'authsec-dialog-title', text: title}), ...content);
  d.addEventListener('close', () => { d.remove(); if (dialog === d) dialog = null; d.dispatchEvent(new Event('authsec-closed')); });
  d.addEventListener('click', (e) => { if (e.target === d) d.close(); });
  dialog = d;
  place();
  return d;
}
function place() {
  if (!dialog || dialog.isConnected) return;
  dialog.removeAttribute('open');
  ($('.live-modal') ?? document.body).append(dialog);
  dialog.showModal();
}
const actions = (...buttons) => el('div', {class: 'confirm-actions'}, ...buttons);
function confirmDialog(title, body, label) {
  return new Promise((resolve) => {
    let answer = false;
    const d = showDialog(title, el('p', {text: body}), actions(
      el('button', {type: 'button', class: 'button secondary', text: 'Cancel', onclick: () => d.close()}),
      el('button', {type: 'button', class: 'button danger', text: label, onclick: () => { answer = true; d.close(); }})));
    d.addEventListener('authsec-closed', () => resolve(answer));
  });
}
function promptDialog(title, label, value, {type = 'text', maxlength = '64', hint, submit = 'Save', id = 'authsec-prompt', autocomplete = 'off'} = {}) {
  return new Promise((resolve) => {
    let answer = null;
    const input = el('input', {id, type, maxlength, value, autocomplete, required: true, 'aria-describedby': hint ? `${id}-hint` : null});
    const form = el('form', {}, el('div', {class: 'field'}, el('label', {for: id, text: label}), input,
      hint ? el('small', {id: `${id}-hint`, class: 'text-[13px] text-subtle', text: hint}) : null), actions(
      el('button', {type: 'button', class: 'button secondary', text: 'Cancel', onclick: () => d.close()}),
      el('button', {type: 'submit', class: 'button primary', text: submit})));
    form.onsubmit = (e) => { e.preventDefault(); answer = input.value.trim(); d.close(); };
    const d = showDialog(title, form);
    d.addEventListener('authsec-closed', () => resolve(answer));
    input.select();
  });
}

let scanning = null;
function stopScan() {
  if (!scanning) return;
  clearTimeout(scanning.timer);
  for (const t of scanning.stream?.getTracks() ?? []) t.stop();
  scanning = null;
}
async function scan() {
  const video = el('video', {class: 'authsec-scanner mt-4 aspect-square w-full rounded-lg bg-black object-cover', playsinline: true, muted: true, 'aria-label': 'Camera'});
  const status = note('Point the camera at the QR code on the other device’s login page.');
  const d = showDialog('Scan QR Code', video, status, actions(el('button', {type: 'button', class: 'button secondary', text: 'Cancel', onclick: () => d.close()})));
  d.addEventListener('authsec-closed', stopScan);
  const state = scanning = {stream: null, timer: 0};
  try {
    if (!navigator.mediaDevices?.getUserMedia) throw Error('unavailable');
    state.stream = await navigator.mediaDevices.getUserMedia({video: {facingMode: 'environment'}, audio: false});
    if (scanning !== state) { for (const t of state.stream.getTracks()) t.stop(); return; }
    video.srcObject = state.stream;
    await video.play();
  } catch {
    status.textContent = 'Can’t use the camera. Allow camera access, or open the code with your camera app instead.';
    status.className = 'authsec-note mt-3 text-[13px] text-destructive';
    return;
  }
  // The browser's own detector when there is one, otherwise jsQR (loaded now).
  let detect;
  if ('BarcodeDetector' in window && (await BarcodeDetector.getSupportedFormats().catch(() => [])).includes('qr_code')) {
    const detector = new BarcodeDetector({formats: ['qr_code']});
    detect = async () => (await detector.detect(video))[0]?.rawValue;
  } else {
    const jsQR = (await import('/public/vendor/jsqr-1.4.0.js')).default;
    const canvas = el('canvas'), g = canvas.getContext('2d', {willReadFrequently: true});
    detect = async () => {
      const w = video.videoWidth, h = video.videoHeight;
      if (!w || !h) return null;
      const scale = Math.min(1, 640 / Math.max(w, h));
      canvas.width = Math.round(w * scale); canvas.height = Math.round(h * scale);
      g.drawImage(video, 0, 0, canvas.width, canvas.height);
      const img = g.getImageData(0, 0, canvas.width, canvas.height);
      return jsQR(img.data, img.width, img.height, {inversionAttempts: 'dontInvert'})?.data;
    };
  }
  // Only while this dialog is open: a few frames a second, then the camera stops.
  const tick = async () => {
    if (scanning !== state) return;
    const text = await detect().catch(() => null);
    if (scanning !== state) return;
    const token = text && qrToken(text);
    if (token) { d.close(); confirmQr(token); return; }
    if (text) { status.textContent = 'That QR code isn’t a StickTalk sign-in code for this server.'; status.className = 'authsec-note mt-3 text-[13px] text-destructive'; }
    state.timer = setTimeout(tick, 150);
  };
  tick();
}
/** A sign-in code for THIS server only: another origin is refused outright. */
function qrToken(text) {
  let url; try { url = new URL(text); } catch { return null; }
  if (url.origin !== location.origin) return null;
  return /^#qr=([A-Za-z0-9_-]{43})$/.exec(url.hash)?.[1] ?? null;
}
async function confirmQr(token) {
  let info;
  try { info = await api('POST', 'auth/qr/inspect', json({token})); }
  catch (e) {
    showDialog('Code Expired', el('p', {text: e.status === 404 ? 'This code has expired or was already used. Show a new one on the other device and scan again.' : e.message}),
      actions(el('button', {type: 'button', class: 'button primary', text: 'OK', onclick: () => dialog?.close()})));
    return;
  }
  let decided = false;
  const yes = el('button', {type: 'button', class: 'button primary', text: 'Yes, Sign In'});
  const no = el('button', {type: 'button', class: 'button secondary', text: 'Cancel'});
  const d = showDialog('Sign in on another device?',
    el('div', {class: 'authsec-qr-request mt-4 flex items-start gap-3 rounded-lg border bg-card p-4'},
      el('span', {class: 'grid size-10 flex-none place-items-center rounded-full bg-muted text-subtle'}, icon('monitor')),
      el('span', {class: 'flex min-w-0 flex-col gap-1'},
        el('strong', {class: 'font-semibold wrap-anywhere', text: info.device}),
        el('small', {class: 'text-subtle wrap-anywhere', text: `IP address ${info.ip}`}),
        el('small', {class: info.same_network ? 'text-subtle' : 'text-warning', text: info.same_network ? 'On the same network as this device' : 'Not on the same network as this device'}),
        el('small', {class: 'text-subtle', text: `Asked ${when(info.created_at)}`}))),
    el('p', {class: 'authsec-qr-warning mt-4! text-foreground!', text: 'Only continue if you are signing in on that device yourself, right now. If someone sent you this code or asked you to scan it, cancel: they would get into your account.'}),
    actions(no, yes));
  no.onclick = () => { decided = true; api('POST', 'auth/qr/reject', json({token})).catch(() => {}); d.close(); };
  yes.onclick = async () => {
    decided = true; yes.disabled = true;
    try {
      await api('POST', 'auth/qr/approve', json({token}));
      showDialog('Signed In', el('p', {text: `${info.device} is now signed in to your account. You can sign it out any time from Devices.`}),
        actions(el('button', {type: 'button', class: 'button primary', text: 'Done', onclick: () => dialog?.close()})));
    } catch (e) {
      showDialog('Didn’t Sign In', el('p', {text: e.status === 404 ? 'The code expired before you confirmed. Scan the new one.' : e.message}),
        actions(el('button', {type: 'button', class: 'button primary', text: 'OK', onclick: () => dialog?.close()})));
    }
  };
  // Closing the question any other way counts as "no".
  d.addEventListener('authsec-closed', () => { if (!decided) api('POST', 'auth/qr/reject', json({token})).catch(() => {}); });
}

// ----- Account switcher ---------------------------------------------------------------
function avatar(user) {
  const name = user.display_name || user.username;
  const span = el('span', {class: 'grid size-10 flex-none place-items-center overflow-hidden rounded-full bg-primary/20 text-base font-semibold text-primary', 'aria-hidden': 'true'});
  if (user.avatar_id) span.append(el('img', {class: 'size-full object-cover', src: `/api/v1/files/${user.avatar_id}/thumbnail`, alt: ''}));
  else span.textContent = [...name][0].toUpperCase();
  return span;
}
async function switcher() {
  emit('close');
  let list;
  try { list = await api('GET', 'browser/accounts'); }
  catch (e) { emit(`media-error:${e.message}`); return; }
  const status = note();
  const rows = list.accounts.map(({user, active}) => {
    const name = user.display_name || user.username;
    const label = el('span', {class: 'flex min-w-0 flex-1 flex-col'}, el('strong', {class: 'truncate font-semibold', text: name}), el('small', {class: 'truncate text-subtle', text: `@${user.username}`}));
    const main = active
      ? el('span', {class: 'flex min-h-14 min-w-0 flex-1 items-center gap-3 px-2', 'aria-current': 'true'}, avatar(user), label,
        el('span', {class: 'flex items-center gap-1 text-xs font-medium text-primary'}, icon('check', 'icon size-4'), 'Active'))
      : el('button', {type: 'button', class: 'flex min-h-14 min-w-0 flex-1 items-center gap-3 rounded-lg px-2 text-left hover:bg-accent', 'aria-label': `Switch to ${name}`, onclick: async (e) => {
        const button = e.currentTarget; button.disabled = true;
        try { await api('POST', 'browser/accounts/switch', `{"user_id":${user.id}}`); location.reload(); }
        catch (err) { status.textContent = err.status === 404 ? `${name} was signed out. Log in again with “Add an Account”.` : err.message; button.disabled = false; }
      }}, avatar(user), label);
    const out = el('button', {type: 'button', class: 'button danger-text', 'aria-label': `Sign out ${name}`, text: 'Sign Out', onclick: async () => {
      try {
        await api('POST', 'browser/accounts/logout', `{"user_id":${user.id}}`);
        if (active) location.assign('/'); else switcher();
      } catch (err) { status.textContent = err.message; }
    }});
    return el('li', {class: 'flex items-center gap-2 border-b py-1 last:border-b-0', 'data-account': user.username}, main, out);
  });
  const full = list.accounts.length >= list.max;
  const add = el('button', {type: 'button', class: 'button primary', disabled: full, text: 'Add an Account', onclick: async () => {
    try { await api('POST', 'browser/accounts/add'); location.assign('/'); }
    catch (e) { status.textContent = e.message; }
  }});
  const d = showDialog('Switch Accounts',
    el('ul', {class: 'authsec-accounts mt-4 grid', 'aria-label': 'Accounts signed in on this browser'}, ...rows),
    el('p', {text: full ? `This browser keeps up to ${list.max} accounts signed in. Sign one out to add another.` : 'Each account stays signed in here with its own session, so anyone using this browser can switch between them.'}),
    status, actions(el('button', {type: 'button', class: 'button secondary', text: 'Close', onclick: () => d.close()}), add));
}

// ----- Personal data export ----------------------------------------------------------
function dataSection() {
  const status = note(flash.data ?? '', !!flash.dataError);
  const e = exportStatus?.export;
  const now = Date.now();
  const describe = !e ? null : {
    queued: ['Your request is waiting to start.', null],
    running: ['We’re putting your data together. This page updates when it’s ready.', null],
    ready: [`Ready${e.byte_size ? ` · ${size(e.byte_size)}` : ''}. Download it before ${when(e.expires_at)}, when it is deleted.`, 'download'],
    failed: ['That export didn’t finish. You can request it again.', null],
    expired: ['Your last export has been deleted.', null],
  }[e.state];
  const next = exportStatus?.next_request_at;
  const request = el('button', {type: 'button', class: 'button primary', disabled: !!(next && next > now) || e?.state === 'queued' || e?.state === 'running', text: 'Request My Data'});
  request.onclick = async () => {
    request.disabled = true;
    try {
      const answer = await withReauth('data_export', () => api('POST', 'users/@me/data-export'));
      // A quick export's pushed progress can arrive before this answer does.
      const rank = (s) => ({queued: 0, running: 1}[s] ?? 2);
      const seen = exportStatus?.export;
      if (!(seen?.id === answer.export?.id && rank(seen.state) > rank(answer.export.state))) exportStatus = answer;
      setFlash('data', 'Requested. You can leave this page; come back to download it.');
    } catch (err) {
      setFlash('data', err.code === 'cancelled' ? '' : err.code === 'conflict' ? 'An export is already being prepared.' : err.message, err.code !== 'cancelled');
    }
  };
  return el('section', {class: 'authsec-section', 'aria-label': 'Request My Data'},
    el('p', {class: 'text-[15px]', text: 'Get a copy of what StickTalk stores for your account:'}),
    el('ul', {class: 'mt-3 ml-5 list-disc text-sm text-subtle [&>li]:mt-1'},
      el('li', {text: 'Your account, profile and sign-in methods'}), el('li', {text: 'Your settings, saved messages, favorites and folders'}),
      el('li', {text: 'The servers you’re in, your friends and blocked people'}), el('li', {text: 'Every message you’ve written, with your attachments'})),
    el('p', {class: 'muted-copy mt-3', text: 'It’s a ZIP file of JSON files with an index.html you can open in a browser. You can request one a day; each can be downloaded for 7 days.'}),
    describe ? el('div', {class: 'authsec-export mt-6 flex flex-wrap items-center gap-3 rounded-lg border bg-card p-4', 'data-state': e.state},
      el('span', {class: 'grid size-10 flex-none place-items-center rounded-full bg-muted text-subtle'}, icon(e.state === 'ready' ? 'download' : e.state === 'failed' ? 'x' : 'clock')),
      el('span', {class: 'flex min-w-48 flex-1 flex-col gap-1'}, el('strong', {class: 'font-semibold', text: `Requested ${when(e.requested_at)}`}), el('small', {class: 'text-subtle', role: 'status', text: describe[0]})),
      describe[1] ? el('a', {class: 'button primary', href: `/api/v1/users/@me/data-export/${e.id}/download`, download: '', text: 'Download'}) : null) : null,
    el('div', {class: 'mt-6 flex flex-wrap items-center gap-3'}, request,
      next && next > now ? el('small', {class: 'text-subtle', text: `You can request another after ${when(next)}.`}) : null),
    status);
}

// ----- Mounting -------------------------------------------------------------------------
function setFlash(key, text, error = false) {
  flash[key] = text;
  flash[`${key}Error`] = error;
  render(true);
}
function render(force = false) {
  for (const root of roots()) {
    if (root.dataset.ready && !force) continue;
    root.dataset.ready = 'true';
    const kind = root.dataset.authsecRoot;
    const sections = kind === 'data'
      ? (exportStatus ? [dataSection()] : [el('p', {class: 'muted-copy', role: 'status', text: 'Loading…'})])
      : passkeys && email
        ? [passkeySection(), el('div', {class: 'settings-divider'}), emailSection(), el('div', {class: 'settings-divider'}), qrSection()]
        : [el('p', {class: 'muted-copy', role: 'status', text: 'Loading…'})];
    const focus = document.activeElement && root.contains(document.activeElement) ? document.activeElement.id : null;
    root.replaceChildren(...sections);
    if (focus) document.getElementById(focus)?.focus();
  }
}
let loading = null;
async function load(kind) {
  if (kind === 'data') exportStatus = await api('GET', 'users/@me/data-export').catch(() => ({export: null, next_request_at: null}));
  else {
    [passkeys, email] = await Promise.all([
      api('GET', 'users/@me/passkeys').catch(() => []),
      api('GET', 'users/@me/email').catch(() => ({smtp_enabled: false})),
    ]);
  }
  render(true);
}

/** Called by live.js after every render while a root is shown or a link waits. */
export function mount(emitter) {
  emit = emitter;
  place();
  const kinds = new Set([...roots()].map(r => r.dataset.authsecRoot));
  for (const kind of kinds) {
    const have = kind === 'data' ? exportStatus : passkeys && email;
    // Fetch once per page opening; a fresh root after leaving the page refetches.
    if (!have && !loading) loading = load(kind).finally(() => { loading = null; });
  }
  // Leaving the pages forgets them, so opening one again shows fresh data.
  if (!kinds.size) { passkeys = email = exportStatus = null; flash = {}; }
  render();
  const link = globalThis.__sticktalkLink;
  if (link && $('.live-shell')) {
    globalThis.__sticktalkLink = null;
    if (link.kind === 'qr') confirmQr(link.token);
    else if (link.kind === 'verify-email') {
      api('POST', 'auth/email/verify', json({token: link.token}))
        .then(() => showDialog('Email Confirmed', el('p', {text: 'You can now use this address to reset your password.'}), actions(el('button', {type: 'button', class: 'button primary', text: 'OK', onclick: () => dialog?.close()}))))
        .catch(() => showDialog('Link Expired', el('p', {text: 'That confirmation link has expired or was already used. Send a new one from Password & Security.'}), actions(el('button', {type: 'button', class: 'button primary', text: 'OK', onclick: () => dialog?.close()}))));
    }
  }
}
/** A [data-authsec] button was pressed. */
export function action(name, emitter) {
  emit = emitter;
  if (name === 'switcher') switcher();
  else if (name === 'scan') scan();
}
// Export progress, pushed by the server over the app's WebSocket (live/authsec.rs).
addEventListener('sticktalk-authsec', (e) => {
  let detail; try { detail = parse(e.detail); } catch { return; }
  if (!exportStatus) return;
  // Progress replaces the note about the request.
  flash.data = ''; flash.dataError = false;
  exportStatus = {...exportStatus, export: detail, next_request_at: detail.state === 'failed' ? null : detail.requested_at + 86400000};
  render(true);
});
