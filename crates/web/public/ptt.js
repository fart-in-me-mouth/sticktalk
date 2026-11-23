// Voice input mode for calls: Voice Activity (the default) or Push to Talk,
// plus an optional Push to Mute key. Remembered on this device.
//
// In Push to Talk the microphone sends nothing at all (no packets, not
// silence) until the key is held: the media host takes the track off its
// sender. Listeners exist only while in a call; nothing runs otherwise.
//
// In the StickTalk Windows app the keys also work while another app (a game)
// has focus: the app watches the keyboard and mouse with passive low-level
// hooks while a call is on and tells this page; see crates/desktop/src/ptt.rs.
const STORE = 'sticktalk-voice-input';
const MAX_DELAY = 2000;

function sanitize(binding) {
  if (!binding || typeof binding !== 'object') return null;
  const label = typeof binding.label === 'string' ? binding.label.slice(0, 32) : '';
  if (typeof binding.code === 'string' && /^[A-Za-z0-9]{1,24}$/.test(binding.code)) return { code: binding.code, label: label || binding.code };
  if ([1, 3, 4].includes(binding.button)) return { button: binding.button, label: label || mouseLabel(binding.button) };
  return null;
}
export function normalizeSettings(raw) {
  const delay = Number(raw?.delay);
  return {
    mode: raw?.mode === 'ptt' ? 'ptt' : 'vad',
    talk: sanitize(raw?.talk),
    mute: sanitize(raw?.mute),
    delay: Number.isFinite(delay) ? Math.round(Math.min(MAX_DELAY, Math.max(0, delay))) : 200,
  };
}
let settings = normalizeSettings({});
try { settings = normalizeSettings(JSON.parse(localStorage.getItem(STORE) ?? '{}')); } catch { /* defaults */ }
function save() {
  try { localStorage.setItem(STORE, JSON.stringify(settings)); } catch { /* this session only */ }
}
export const current = () => ({ ...settings });

function mouseLabel(button) {
  return button === 1 ? 'Middle Mouse' : button === 3 ? 'Mouse 4' : 'Mouse 5';
}
export function keyLabel(e) {
  if (e.code === 'Space') return 'Space';
  if (e.key && e.key.length === 1) return e.key.toUpperCase();
  return (e.key && e.key !== 'Unidentified' ? e.key : e.code).replace(/^Arrow/, '').slice(0, 24);
}
// Keys that type something: they do not act while you type in a text field.
const typing = /^(Key[A-Z]|Digit\d|Numpad\d|Space|Minus|Equal|Bracket(Left|Right)|Backslash|Semicolon|Quote|Backquote|Comma|Period|Slash|Enter|Backspace|Tab|IntlBackslash)$/;
const editable = (t) => t?.closest?.('input,textarea,select,[contenteditable=""],[contenteditable="true"]');
export function matches(binding, e) {
  if (!binding) return false;
  if (binding.code) return e.code === binding.code && !(typing.test(binding.code) && editable(e.target));
  return e.button === binding.button;
}

// ---- in a call ---------------------------------------------------------------
let gate = null; // The media host's "transmit or not" hook, while in a call.
let usage = {}; // KeyboardEvent.code -> USB HID usage, for the Windows app.
const held = { talk: false, mute: false, touch: false };
let talking = false; // The microphone is open (key held, or within the release delay).
let release = null;
let host = false;

const hostIpc = () =>
  globalThis.__sticktalkHost?.ptt && globalThis.ipc?.postMessage ? globalThis.ipc : null;
function hostBinding(b) {
  if (!b) return null;
  if (b.button) return { button: b.button };
  const u = usage[b.code];
  return u ? { usage: u } : null;
}

export function transmitting() {
  return !!gate && (settings.mode !== 'ptt' || talking) && !held.mute;
}
function changed() {
  gate?.();
  mount();
}
function press(which, down) {
  if (which === 'mute') {
    if (held.mute === down) return;
    held.mute = down;
    return changed();
  }
  held[which] = down;
  const any = held.talk || held.touch;
  if (any) {
    clearTimeout(release);
    release = null;
    if (!talking) { talking = true; changed(); }
  } else if (talking && !release) {
    // The release delay keeps the end of a word from being cut off.
    const end = () => { release = null; if (!held.talk && !held.touch) { talking = false; changed(); } };
    if (settings.delay) release = setTimeout(end, settings.delay);
    else end();
  }
}
function releaseAll() {
  held.talk = held.touch = false;
  press('talk', false);
  press('mute', false);
}
const onKey = (e) => {
  if (e.repeat) return;
  const down = e.type === 'keydown';
  if (settings.mode === 'ptt' && matches(settings.talk, e)) press('talk', down);
  if (matches(settings.mute, e)) press('mute', down);
};
const onMouse = (e) => {
  const down = e.type === 'mousedown';
  if (settings.mode === 'ptt' && matches(settings.talk, e)) press('talk', down);
  if (matches(settings.mute, e)) press('mute', down);
};
// The phone's hold-to-talk button (and Space/Enter on it, for keyboards).
const onPointer = (e) => {
  if (e.type === 'pointerdown') {
    if (!e.target.closest?.('#live-ptt')) return;
    e.preventDefault();
    try { e.target.closest('#live-ptt').setPointerCapture(e.pointerId); } catch { /* released by pointerup anywhere */ }
    press('touch', true);
  } else if (held.touch) press('touch', false);
};
const onButtonKey = (e) => {
  if (!e.target.closest?.('#live-ptt') || !['Space', 'Enter'].includes(e.code) || e.repeat) return;
  e.preventDefault();
  press('touch', e.type === 'keydown');
};
const onHost = (e) => {
  const d = e.detail;
  if (d && (d.ptt === 'talk' || d.ptt === 'mute')) press(d.ptt, !!d.down);
};
const onBlur = () => { if (!host) releaseAll(); };

function install() {
  uninstall();
  addEventListener('pointerdown', onPointer, true);
  addEventListener('pointerup', onPointer, true);
  addEventListener('pointercancel', onPointer, true);
  addEventListener('keydown', onButtonKey, true);
  addEventListener('keyup', onButtonKey, true);
  const wanted = (settings.mode === 'ptt' && settings.talk) || settings.mute;
  if (!wanted) return;
  const ipc = hostIpc();
  const talk = settings.mode === 'ptt' ? hostBinding(settings.talk) : null;
  const mute = hostBinding(settings.mute);
  if (ipc && (talk || mute)) {
    host = true;
    addEventListener('sticktalk-host', onHost);
    ipc.postMessage(JSON.stringify({ ptt: 'start', talk, mute }));
    return;
  }
  addEventListener('keydown', onKey, true);
  addEventListener('keyup', onKey, true);
  addEventListener('mousedown', onMouse, true);
  addEventListener('mouseup', onMouse, true);
  // A key let go while another window had focus never reports its release here.
  addEventListener('blur', onBlur);
}
function uninstall() {
  removeEventListener('pointerdown', onPointer, true);
  removeEventListener('pointerup', onPointer, true);
  removeEventListener('pointercancel', onPointer, true);
  removeEventListener('keydown', onButtonKey, true);
  removeEventListener('keyup', onButtonKey, true);
  removeEventListener('keydown', onKey, true);
  removeEventListener('keyup', onKey, true);
  removeEventListener('mousedown', onMouse, true);
  removeEventListener('mouseup', onMouse, true);
  removeEventListener('blur', onBlur);
  if (host) {
    removeEventListener('sticktalk-host', onHost);
    hostIpc()?.postMessage(JSON.stringify({ ptt: 'stop' }));
    host = false;
  }
}

// The media host calls these as a call starts and ends.
export function attach(onChange, keyUsage = {}) {
  gate = onChange;
  usage = keyUsage;
  install();
  changed();
}
export function detach() {
  uninstall();
  clearTimeout(release);
  release = null;
  held.talk = held.mute = held.touch = false;
  talking = false;
  gate = null;
  mount();
}

// The transmit indicator, and on phones the hold-to-talk button itself.
export function mount() {
  const button = document.getElementById('live-ptt');
  if (!button) return;
  const show = !!gate && (settings.mode === 'ptt' || held.mute);
  button.hidden = !show;
  button.classList.toggle('hidden', !show);
  const live = transmitting() && settings.mode === 'ptt';
  button.classList.toggle('is-live', live);
  button.classList.toggle('is-muted', held.mute);
  button.setAttribute('aria-pressed', String(held.touch));
  const key = settings.talk?.label;
  const text = held.mute ? 'Muted' : live ? 'Transmitting' : key ? `Push to Talk · ${key}` : 'Hold to Talk';
  const label = button.querySelector('.ptt-label');
  if (label && label.textContent !== text) label.textContent = text;
  button.setAttribute('aria-label', held.mute ? 'Muted while the push-to-mute key is held' : live ? 'Transmitting' : 'Hold to talk');
  const view = document.querySelector('.voice-view');
  if (view) view.dataset.ptt = !gate ? '' : held.mute ? 'muted' : live ? 'talking' : settings.mode === 'ptt' ? 'idle' : '';
}

function update(next) {
  settings = normalizeSettings({ ...settings, ...next });
  save();
  if (gate) { install(); releaseAll(); changed(); }
}

// ---- Voice & Video settings ----------------------------------------------------
let recording = null;
function record(which, button, refresh) {
  if (recording) recording.cancel();
  const previous = button.textContent;
  button.textContent = 'Press a key or mouse button…';
  button.setAttribute('aria-pressed', 'true');
  const done = (binding) => {
    removeEventListener('keydown', key, true);
    removeEventListener('mousedown', mouse, true);
    recording = null;
    button.setAttribute('aria-pressed', 'false');
    button.textContent = previous;
    if (binding !== undefined) update({ [which]: binding });
    refresh();
  };
  const key = (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.code === 'Escape') return done();
    if (!e.code) return;
    done({ code: e.code, label: keyLabel(e) });
  };
  const mouse = (e) => {
    if (![1, 3, 4].includes(e.button)) return;
    e.preventDefault();
    e.stopPropagation();
    done({ button: e.button, label: mouseLabel(e.button) });
  };
  addEventListener('keydown', key, true);
  addEventListener('mousedown', mouse, true);
  recording = { cancel: () => done() };
}

export function settingsView(ringtone) {
  const box = document.createElement('section');
  box.className = 'voice-input-settings';
  box.setAttribute('aria-labelledby', 'voice-input-title');
  const refresh = () => box.replaceWith(settingsView(ringtone));
  const heading = document.createElement('h3');
  heading.id = 'voice-input-title';
  heading.className = 'category mt-8 mb-2';
  heading.textContent = 'Input Mode';
  const modes = document.createElement('div');
  modes.className = 'voice-input-modes';
  modes.setAttribute('role', 'radiogroup');
  modes.setAttribute('aria-labelledby', 'voice-input-title');
  for (const [mode, text, hint] of [
    ['vad', 'Voice Activity', 'Your microphone sends when you speak.'],
    ['ptt', 'Push to Talk', 'Your microphone sends only while you hold your key.'],
  ]) {
    const label = document.createElement('label');
    label.className = 'voice-input-mode';
    const input = document.createElement('input');
    input.type = 'radio';
    input.name = 'voice-input-mode';
    input.value = mode;
    input.checked = settings.mode === mode;
    input.onchange = () => { update({ mode }); refresh(); };
    const copy = document.createElement('span');
    const strong = document.createElement('strong');
    strong.textContent = text;
    const small = document.createElement('small');
    small.textContent = hint;
    copy.append(strong, small);
    label.append(input, copy);
    modes.append(label);
  }
  box.append(heading, modes);
  const shortcut = (which, title, help) => {
    const row = document.createElement('div');
    row.className = 'voice-shortcut';
    const label = document.createElement('span');
    label.className = 'voice-shortcut-label';
    const strong = document.createElement('strong');
    strong.textContent = title;
    const small = document.createElement('small');
    small.textContent = help;
    label.append(strong, small);
    const value = document.createElement('kbd');
    value.textContent = settings[which]?.label ?? 'Not set';
    const recordButton = document.createElement('button');
    recordButton.type = 'button';
    recordButton.className = 'button secondary';
    recordButton.textContent = settings[which] ? 'Change Keybind' : 'Record Keybind';
    recordButton.setAttribute('aria-label', `${recordButton.textContent}: ${title}`);
    recordButton.onclick = () => record(which, recordButton, refresh);
    row.append(label, value, recordButton);
    if (settings[which]) {
      const clear = document.createElement('button');
      clear.type = 'button';
      clear.className = 'button link-button';
      clear.textContent = 'Clear';
      clear.setAttribute('aria-label', `Clear ${title}`);
      clear.onclick = () => { update({ [which]: null }); refresh(); };
      row.append(clear);
    }
    return row;
  };
  if (settings.mode === 'ptt') {
    box.append(shortcut('talk', 'Push to Talk Shortcut', 'Hold this key or mouse button to talk.'));
    const delay = document.createElement('label');
    delay.className = 'field voice-release-delay';
    const caption = document.createElement('span');
    caption.textContent = 'Push to Talk Release Delay';
    const slider = document.createElement('input');
    slider.type = 'range';
    slider.min = '0';
    slider.max = String(MAX_DELAY);
    slider.step = '20';
    slider.value = String(settings.delay);
    const readout = document.createElement('output');
    const show = () => {
      readout.textContent = `${slider.value} ms`;
      slider.setAttribute('aria-valuetext', `${slider.value} milliseconds`);
    };
    show();
    slider.oninput = show;
    slider.onchange = () => update({ delay: Number(slider.value) });
    delay.append(caption, slider, readout);
    box.append(delay);
  }
  box.append(shortcut('mute', 'Push to Mute Shortcut', 'Hold this key or mouse button to mute yourself for a moment.'));
  const note = document.createElement('p');
  note.className = 'voice-input-note';
  note.textContent = globalThis.__sticktalkHost?.ptt
    ? 'These keys also work while you play a game or use another app.'
    : 'In the browser these keys work while StickTalk is focused. In the StickTalk Windows app they also work while you use another app. On a phone, hold the talk button in a call.';
  box.append(note);
  if (ringtone) {
    const toggle = document.createElement('label');
    toggle.className = 'live-checkbox mt-6';
    const copy = document.createElement('span');
    copy.textContent = 'Play a ringtone for incoming calls';
    const input = document.createElement('input');
    input.type = 'checkbox';
    input.checked = ringtone.enabled();
    input.onchange = () => ringtone.set(input.checked);
    toggle.append(copy, input);
    const small = document.createElement('small');
    small.className = 'voice-input-note';
    small.textContent = 'Calls never ring out loud while your status is Do Not Disturb.';
    box.append(toggle, small);
  }
  return box;
}
