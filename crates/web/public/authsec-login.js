// Signed-out account security (docs/features/authsec.md): "Sign in with a
// passkey", sign-in by QR code, the email password reset, links from email and
// QR codes, and the accounts already signed in on this browser. Loaded by the
// progressive login (boot.js) after first paint. No bearer token ever reaches
// script: every sign-in here ends in an HttpOnly cookie set by the server.
const $ = (id) => document.getElementById(id);
const post = (path, body) => fetch(`/api/v1/${path}`, {
  method: 'POST', credentials: 'same-origin', cache: 'no-store',
  headers: {'Content-Type': 'application/json'}, body: JSON.stringify(body ?? {}),
});
const el = (tag, attrs = {}, ...children) => {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'class') e.className = v;
    else if (k === 'text') e.textContent = v;
    else if (v !== false && v != null) e.setAttribute(k, v === true ? '' : v);
  }
  e.append(...children.filter(c => c != null));
  return e;
};
// The sign-in page has no icon sprite yet (it arrives with the app), so the
// two Lucide icons used here are drawn inline.
const ICONS = {
  'key-round': [['path', {d: 'M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4z'}], ['circle', {cx: '16.5', cy: '7.5', r: '.5', fill: 'currentColor'}]],
  smartphone: [['rect', {width: '14', height: '20', x: '5', y: '2', rx: '2', ry: '2'}], ['path', {d: 'M12 18h.01'}]],
};
const icon = (name, cls = 'icon size-5') => {
  const NS = 'http://www.w3.org/2000/svg', svg = document.createElementNS(NS, 'svg');
  svg.setAttribute('class', cls); svg.setAttribute('aria-hidden', 'true'); svg.setAttribute('viewBox', '0 0 24 24');
  for (const [tag, attrs] of ICONS[name] ?? []) {
    const shape = document.createElementNS(NS, tag);
    for (const [k, v] of Object.entries(attrs)) shape.setAttribute(k, v);
    svg.append(shape);
  }
  return svg;
};

// ----- base64url and WebAuthn JSON ------------------------------------------------
export const fromB64 = (s) => {
  const b = atob(s.replace(/-/g, '+').replace(/_/g, '/') + '==='.slice((s.length + 3) % 4));
  return Uint8Array.from(b, c => c.charCodeAt(0)).buffer;
};
export const toB64 = (buf) => {
  let s = ''; for (const b of new Uint8Array(buf)) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
};
export function requestOptions(json) {
  const o = JSON.parse(json).publicKey;
  o.challenge = fromB64(o.challenge);
  o.allowCredentials = (o.allowCredentials ?? []).map(c => ({...c, id: fromB64(c.id)}));
  return o;
}
export function assertionJSON(c) {
  const r = c.response;
  return JSON.stringify({
    id: c.id, rawId: toB64(c.rawId), type: c.type, extensions: {},
    response: {
      authenticatorData: toB64(r.authenticatorData), clientDataJSON: toB64(r.clientDataJSON),
      signature: toB64(r.signature), userHandle: r.userHandle ? toB64(r.userHandle) : null,
    },
  });
}
export const passkeysSupported = () => !!(window.PublicKeyCredential && navigator.credentials?.get);

/** Passwordless sign-in. Resolves true when the session cookie is set. */
export async function passkeySignIn() {
  const start = await post('auth/passkey/start');
  if (!start.ok) throw Error('Passkey sign-in is unavailable right now.');
  const {challenge_id, public_key} = await start.json();
  let credential;
  try {
    credential = await navigator.credentials.get({publicKey: requestOptions(public_key)});
  } catch (e) {
    if (e?.name === 'NotAllowedError' || e?.name === 'AbortError') return false;
    throw Error('This device could not use a passkey.');
  }
  if (!credential) return false;
  const done = await post('browser/session/passkey', {challenge_id, credential: assertionJSON(credential)});
  if (done.ok) return true;
  const code = (await done.json().catch(() => ({}))).code;
  throw Error({
    limit_exceeded: 'This account is signed in on 5 devices, the most allowed. Log out on one of them first.',
    rate_limited: 'Too many attempts. Wait a minute and try again.',
  }[code] ?? 'That passkey isn’t registered for an account on this server.');
}

// ----- QR code -----------------------------------------------------------------
/** An SVG of a QR matrix ('1' = dark), with the standard four-module quiet zone. */
export function qrSvg(size, modules, label) {
  const NS = 'http://www.w3.org/2000/svg', n = size + 8;
  const svg = document.createElementNS(NS, 'svg');
  svg.setAttribute('viewBox', `0 0 ${n} ${n}`); svg.setAttribute('role', 'img');
  svg.setAttribute('aria-label', label); svg.setAttribute('shape-rendering', 'crispEdges');
  svg.setAttribute('class', 'authsec-qr block size-44 rounded-lg');
  svg.dataset.size = String(size); svg.dataset.modules = modules;
  const bg = document.createElementNS(NS, 'rect');
  bg.setAttribute('width', n); bg.setAttribute('height', n); bg.setAttribute('fill', '#fff');
  let d = '';
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++)
    if (modules.charCodeAt(y * size + x) === 49) d += `M${x + 4} ${y + 4}h1v1h-1z`;
  const path = document.createElementNS(NS, 'path');
  path.setAttribute('d', d); path.setAttribute('fill', '#000');
  svg.append(bg, path);
  return svg;
}

// The sign-in buckets are per address, shared by everyone behind it. A busy
// bucket must not throw away an approval: the ticket is checked only after the
// rate limit and stays valid for 60 s, so wait as told and try again.
async function redeemQr(ticket) {
  const until = Date.now() + 55000;
  for (;;) {
    const r = await post('browser/session/qr', {ticket});
    if (r.status !== 429) return r;
    const wait = Number((await r.clone().json().catch(() => ({}))).retry_after_ms) || 2000;
    if (Date.now() + wait > until) return r;
    await new Promise((done) => setTimeout(done, Math.min(Math.max(wait, 250), 10000)));
  }
}
// qrHold: a final message is showing; only "Show a New Code" starts again.
let socket = null, qrPanel = null, qrWanted = false, qrHold = false;
function qrState(...children) {
  const body = qrPanel?.querySelector('[data-qr-body]');
  if (body) body.replaceChildren(...children);
}
function closeQr() {
  if (socket) { socket.onclose = null; socket.close(); socket = null; }
  // A hidden code is never left on screen to be shown again stale.
  if (!qrHold) qrState();
}
function openQr(loadApp) {
  qrHold = false;
  closeQr();
  qrState(el('p', {class: 'text-sm text-subtle', role: 'status', text: 'Getting a code…'}));
  const ws = new WebSocket(`${location.origin.replace(/^http/, 'ws')}/api/v1/auth/qr/socket`);
  socket = ws;
  const retry = (text) => (qrHold = true) && qrState(
    el('p', {class: 'text-sm text-subtle', role: 'status', text}),
    el('button', {type: 'button', class: 'button secondary mt-3', 'data-qr-retry': true, text: 'Show a New Code'}),
  );
  ws.onmessage = async (event) => {
    let m; try { m = JSON.parse(event.data); } catch { return; }
    if (m.type === 'code') {
      qrState(qrSvg(m.size, m.modules, 'QR code to sign in to StickTalk'),
        el('p', {class: 'mt-4 text-sm text-subtle', text: 'Codes change every 2 minutes.'}));
    } else if (m.type === 'scanned') {
      qrState(el('span', {class: 'grid size-16 place-items-center rounded-full bg-muted text-primary'}, icon('smartphone', 'icon size-8')),
        el('p', {class: 'mt-4 font-semibold', role: 'status', text: 'Check your other device'}),
        el('p', {class: 'mt-1 text-sm text-subtle', text: 'Confirm signing in there. Nothing happens until you do.'}));
    } else if (m.type === 'rejected') {
      qrState(el('p', {class: 'text-sm text-subtle', role: 'status', text: 'Sign-in was declined. Here’s a new code.'}));
    } else if (m.type === 'approved') {
      qrHold = true;
      closeQr();
      qrState(el('p', {class: 'text-sm', role: 'status', text: 'Signing you in…'}));
      const r = await redeemQr(m.ticket);
      if (r.ok) loadApp();
      else retry((await r.json().catch(() => ({}))).code === 'limit_exceeded'
        ? 'That account is signed in on 5 devices, the most allowed. Log out on one of them first.'
        : 'Signing in didn’t finish. Try again.');
    } else if (m.type === 'expired') {
      socket = null; retry('The QR code expired.');
    }
  };
  ws.onclose = () => { if (socket === ws) { socket = null; retry('QR sign-in isn’t available right now.'); } };
}
function buildQrPanel(loadApp) {
  qrPanel = el('section', {
    class: 'authsec-qr-card hidden w-[min(440px,100%)] flex-col items-center rounded-xl border bg-background p-8 text-center shadow-sm phone:mt-6 phone:w-full phone:max-w-[480px] phone:rounded-none phone:border-0 phone:bg-transparent phone:p-0 phone:shadow-none',
    'aria-labelledby': 'authsec-qr-title',
  },
  el('h2', {id: 'authsec-qr-title', class: 'mb-2 text-xl font-semibold', text: 'Log in with QR code'}),
  el('p', {class: 'mb-6 text-sm text-subtle', text: 'Scan this with StickTalk on a phone where you’re already signed in: Settings, Password & Security, Scan QR Code. Or use your camera app.'}),
  el('div', {class: 'grid min-h-44 place-items-center', 'data-qr-body': true}));
  const toggle = el('button', {
    type: 'button', class: 'button secondary authsec-qr-toggle mt-4 hidden h-12 w-full text-[15px]', 'aria-expanded': 'false',
  }, icon('smartphone'), 'Log In with a QR Code');
  qrPanel.addEventListener('click', e => { if (e.target.closest('[data-qr-retry]')) openQr(loadApp); });
  toggle.onclick = () => {
    qrWanted = !qrWanted;
    toggle.setAttribute('aria-expanded', String(qrWanted));
    sync();
    if (qrWanted) qrPanel.scrollIntoView({block: 'nearest'});
  };
  return toggle;
}

// ----- Screens -------------------------------------------------------------------
let ctx = null, options = {passkeys: false, qr_login: false, email_reset: false};
const phone = () => matchMedia('(max-width: 767px)').matches;
/** Called by boot.js whenever it switches between log in, register and reset. */
function sync(otherForm = ctx?.otherForm?.()) {
  if (!ctx) return;
  const signIn = !otherForm && !ctx.form.hidden;
  const pk = $('authsec-passkey');
  if (pk) pk.hidden = !(signIn && options.passkeys && passkeysSupported());
  const toggle = document.querySelector('.authsec-qr-toggle');
  const wide = !phone();
  // Narrowing the window never drops a code that is already showing or waiting.
  if (!wide && signIn && (socket || qrHold) && !qrPanel?.classList.contains('hidden')) {
    qrWanted = true;
    toggle?.setAttribute('aria-expanded', 'true');
  }
  if (toggle) toggle.lastChild.textContent = qrWanted ? 'Hide QR Code' : 'Log In with a QR Code';
  if (toggle) toggle.classList.toggle('hidden', !(signIn && options.qr_login && !wide));
  const showQr = signIn && options.qr_login && (wide || qrWanted);
  qrPanel?.classList.toggle('hidden', !showQr);
  qrPanel?.classList.toggle('flex', showQr);
  if (showQr && !socket && !qrHold) openQr(ctx.loadApp);
  if (!showQr) closeQr();
}

function panel(title, subtitle, ...content) {
  const card = el('section', {
    class: 'authsec-panel auth-card w-full rounded-xl border bg-background p-8 shadow-sm phone:my-auto phone:w-full phone:max-w-[480px] phone:rounded-none phone:border-0 phone:bg-transparent phone:p-0 phone:shadow-none',
    'aria-labelledby': 'authsec-panel-title',
  },
  el('h1', {id: 'authsec-panel-title', class: 'mb-2 text-[28px] leading-tight phone:text-center', text: title}),
  el('p', {class: 'mb-8 text-[15px] text-subtle phone:text-center', text: subtitle}), ...content);
  return card;
}
function showPanel(card) {
  document.querySelector('.authsec-panel')?.remove();
  ctx.form.hidden = true;
  document.querySelector('.authsec-accounts')?.setAttribute('hidden', '');
  ctx.column.append(card);
  sync();
  card.querySelector('input')?.focus();
}
function closePanel(message, error) {
  document.querySelector('.authsec-panel')?.remove();
  ctx.form.hidden = false;
  document.querySelector('.authsec-accounts')?.removeAttribute('hidden');
  sync();
  if (message) ctx.say(message, error);
}
const status = () => el('p', {class: 'auth-status my-3 text-[13px] text-muted-foreground empty:hidden', role: 'status'});
function tell(node, text, error = false) {
  node.textContent = text;
  node.classList.toggle('text-destructive', error);
  node.setAttribute('role', error ? 'alert' : 'status');
}
const field = (id, label, attrs) => el('div', {class: 'field mb-6'},
  el('label', {for: id, text: label}),
  el('input', {class: 'h-12 text-base touch:min-h-[52px]', id, ...attrs}));
const back = (text = 'Back to Log In') => {
  const b = el('button', {type: 'button', class: 'mt-6 inline-flex min-h-11 items-center px-1 font-semibold text-primary hover:underline', text});
  b.onclick = () => closePanel();
  return b;
};

function emailResetPanel() {
  const note = status();
  const form = el('form', {}, field('authsec-login', 'Username or Email', {autocomplete: 'username', autocapitalize: 'none', spellcheck: 'false', required: true, maxlength: '254'}),
    el('button', {type: 'submit', class: 'button primary h-12 w-full text-[15px]', text: 'Email Me a Link'}), note);
  form.onsubmit = async (e) => {
    e.preventDefault();
    const submit = form.querySelector('[type=submit]'); submit.disabled = true;
    tell(note, 'Sending…');
    try {
      const r = await post('auth/email-reset/request', {login: $('authsec-login').value.trim()});
      if (r.status === 202) tell(note, 'If that matches an account with a confirmed email address, a reset link is on its way. It works once, for 30 minutes.');
      else tell(note, (await r.json().catch(() => ({}))).code === 'rate_limited' ? 'Too many requests. Wait a while and try again.' : 'Enter your username or email address.', true);
    } catch { tell(note, 'Can’t reach the server. Check your connection and try again.', true); }
    finally { submit.disabled = false; }
  };
  const code = el('a', {class: 'mt-4 block text-[13px]', href: '/?reset', text: 'Have a reset code from the person who runs this server?'});
  return panel('Reset your password', 'We’ll email a link to the address you confirmed on your account.', form, code, back());
}

function newPasswordPanel(token) {
  const note = status();
  const form = el('form', {},
    el('input', {type: 'text', name: 'username', autocomplete: 'username', hidden: true}),
    field('authsec-new', 'New Password', {type: 'password', autocomplete: 'new-password', required: true, maxlength: '128'}),
    field('authsec-confirm', 'Confirm New Password', {type: 'password', autocomplete: 'new-password', required: true, maxlength: '128'}),
    el('button', {type: 'submit', class: 'button primary h-12 w-full text-[15px]', text: 'Set New Password'}), note);
  form.onsubmit = async (e) => {
    e.preventDefault();
    const password = $('authsec-new').value;
    if (password !== $('authsec-confirm').value) { tell(note, 'The two passwords don’t match.', true); return; }
    const submit = form.querySelector('[type=submit]'); submit.disabled = true;
    tell(note, 'Setting your new password…');
    try {
      const r = await post('auth/email-reset/complete', {token, new_password: password});
      if (r.status === 204) { closePanel('Your password was changed and every device was signed out. Log in with the new one.'); return; }
      const code = (await r.json().catch(() => ({}))).code;
      tell(note, code === 'invalid_credentials'
        ? 'This reset link has expired or was already used. Ask for a new one.'
        : code === 'rate_limited' ? 'Too many attempts. Wait a minute and try again.' : 'Choose a password of up to 128 characters.', true);
    } catch { tell(note, 'Can’t reach the server. Check your connection and try again.', true); }
    finally { submit.disabled = false; }
  };
  return panel('Choose a new password', 'Resetting signs you out on every device.', form, back('Cancel'));
}

function avatarFor(user) {
  const name = user.display_name || user.username;
  const span = el('span', {class: 'grid size-10 flex-none place-items-center overflow-hidden rounded-full bg-primary/20 text-base font-semibold text-primary', 'aria-hidden': 'true'});
  if (user.avatar_id) span.append(el('img', {class: 'size-full object-cover', src: `/api/v1/files/${user.avatar_id}/thumbnail`, alt: ''}));
  else span.textContent = [...name][0].toUpperCase();
  return span;
}
async function chooser() {
  document.querySelector('.authsec-accounts')?.remove();
  let list;
  try {
    const r = await fetch('/api/v1/browser/accounts', {credentials: 'same-origin', cache: 'no-store'});
    list = r.ok ? await r.json() : null;
  } catch { return; }
  const waiting = (list?.accounts ?? []).filter(a => !a.active);
  if (!waiting.length) return;
  const rows = waiting.map(({user}) => {
    const name = user.display_name || user.username;
    const choose = el('button', {type: 'button', class: 'flex min-h-14 min-w-0 flex-1 items-center gap-3 rounded-lg px-2 text-left hover:bg-accent', 'aria-label': `Continue as ${name}`},
      avatarFor(user), el('span', {class: 'flex min-w-0 flex-col'}, el('strong', {class: 'truncate font-semibold', text: name}), el('small', {class: 'truncate text-subtle', text: `@${user.username}`})));
    choose.onclick = async () => {
      choose.disabled = true;
      const r = await post('browser/accounts/switch', {user_id: user.id}).catch(() => null);
      if (r?.ok) ctx.loadApp(); else { choose.disabled = false; ctx.say('That account was signed out. Log in again.', true); chooser(); }
    };
    const remove = el('button', {type: 'button', class: 'button danger-text', 'aria-label': `Sign ${name} out of this browser`, text: 'Sign Out'});
    remove.onclick = async () => { remove.disabled = true; await post('browser/accounts/logout', {user_id: user.id}).catch(() => null); chooser(); };
    return el('li', {class: 'flex items-center gap-2 border-b py-1 last:border-b-0'}, choose, remove);
  });
  const card = el('section', {class: 'authsec-accounts w-[min(440px,100%)] rounded-xl border bg-background p-6 shadow-sm phone:w-full phone:max-w-[480px] phone:rounded-none phone:border-0 phone:bg-transparent phone:px-0 phone:pt-0 phone:pb-6 phone:shadow-none', 'aria-labelledby': 'authsec-accounts-title'},
    el('h2', {id: 'authsec-accounts-title', class: 'category mb-2', text: 'Signed in on this browser'}), el('ul', {class: 'grid'}, ...rows),
    el('p', {class: 'mt-3 text-[13px] text-subtle', text: 'Or log in to another account below.'}));
  ctx.column.prepend(card);
}

/** Wire the sign-in screen. `ctx`: {form, say, loadApp, otherForm()} from boot.js. */
export async function install(context) {
  if (ctx) return;
  ctx = context;
  const page = ctx.form.closest('main') ?? ctx.form.parentElement;
  // One column (account chooser + form), and the QR card beside it on wide screens.
  ctx.row = el('div', {class: 'authsec-row flex w-full items-stretch justify-center gap-6 phone:flex-col phone:items-center phone:gap-0'});
  ctx.column = el('div', {class: 'flex w-[min(440px,100%)] flex-col gap-4 phone:w-full phone:max-w-[480px] phone:gap-0'});
  ctx.form.replaceWith(ctx.row);
  ctx.column.append(ctx.form);
  ctx.row.append(ctx.column);
  ctx.form.classList.add('w-full!');
  const toggle = buildQrPanel(ctx.loadApp);
  ctx.row.append(qrPanel);
  const passkey = el('button', {type: 'button', id: 'authsec-passkey', class: 'button secondary mt-3 h-12 w-full border text-[15px]', hidden: true}, icon('key-round'), 'Sign In with a Passkey');
  passkey.onclick = async () => {
    passkey.disabled = true;
    ctx.say('Waiting for your passkey…');
    try { if (await passkeySignIn()) ctx.loadApp(); else ctx.say(''); }
    catch (e) { ctx.say(e.message, true); }
    finally { passkey.disabled = false; }
  };
  $('boot-submit').after(passkey);
  (document.getElementById('boot-google') ?? passkey).after(toggle);
  page.classList.add('authsec-ready');
  addEventListener('resize', () => sync(), {passive: true});
  try {
    const r = await fetch('/api/v1/auth/options', {credentials: 'same-origin', cache: 'no-store'});
    if (r.ok) options = await r.json();
  } catch {}
  // With email configured, "Forgot your password?" emails a link instead.
  $('boot-forgot')?.addEventListener('click', (e) => {
    if (!options.email_reset) return;
    e.preventDefault(); e.stopImmediatePropagation();
    showPanel(emailResetPanel());
  }, true);
  const link = globalThis.__sticktalkLink;
  if (link?.kind === 'reset') { globalThis.__sticktalkLink = null; showPanel(newPasswordPanel(link.token)); }
  else if (link?.kind === 'verify-email') {
    globalThis.__sticktalkLink = null;
    const r = await post('auth/email/verify', {token: link.token}).catch(() => null);
    ctx.say(r?.status === 204 ? 'Your email address is confirmed. You can use it to reset your password.' : 'That confirmation link has expired or was already used.', r?.status !== 204);
  } else if (link?.kind === 'qr') {
    ctx.say('Log in here to approve signing in on your other device.');
  }
  globalThis.__sticktalkAuthsecSync = sync;
  sync();
  chooser();
}
