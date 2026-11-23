// Nonsecret device preferences only. No account, message, draft or token storage.
//
// Appearance and accessibility (docs/features/themes.md). Every preference is
// applied as a data attribute or CSS custom property on <html>, so changing
// one is a style recalculation, never a re-render. The theme's tokens are
// derived in Rust (sticktalk_proto::themes) and handed to `applyTheme`; they
// live in one constructed style sheet. A small snapshot of what is applied is
// kept so `theme-boot.js` can paint the same thing before the first frame.
const choice = (value, options, fallback) => options.includes(value) ? value : fallback;
const autoplay = ['always', 'hover', 'never'];
const defaults = {
  density: 'cozy', fontScale: 1, underlineLinks: false, linkPreviews: true, superReactions: true, plainNames: false, reduceCosmetics: false,
  // Theme: a base (dark, light, darker, system) or a preset / custom theme.
  theme: 'dark', themeId: '', themeCode: '',
  // Accessibility.
  saturation: 100, saturationCustom: false, contrastBoost: false, roleColors: 'names', focusAlways: false, motion: 'system',
  autoplayGifs: 'always', autoplayEmoji: 'always', autoplayStickers: 'always', autoplayAvatars: 'hover', autoplayEffects: 'always',
  chatFont: 'inter', messageSpacing: 2, groupSpacing: 16, zoom: 1, sendButton: true,
  // Text to speech.
  ttsPlayback: true, ttsNotify: 'off', ttsRate: 1,
  // Streamer Mode.
  streamer: false, streamerAuto: false, streamerAccount: true, streamerInvites: true, streamerNotes: true, streamerRequests: true,
  streamerPopups: true, streamerSounds: true, streamerPreviews: 'blur',
  appIcon: 'mint',
};
export const APP_ICONS = ['mint', 'ocean', 'violet', 'rose', 'amber', 'graphite', 'snow', 'sunset', 'aurora', 'outline', 'midnight', 'lime'];
export function normalizePreferences(value = {}) {
  const v = value ?? {};
  const number = (x, min, max, fallback) => Number.isFinite(Number(x)) ? Math.min(max, Math.max(min, Number(x))) : fallback;
  return { density: v.density === 'compact' ? 'compact' : 'cozy',
    fontScale: [0.8,0.9,1,1.1,1.25,1.5].includes(Number(v.fontScale)) ? Number(v.fontScale) : 1,
    underlineLinks: v.underlineLinks === true,
    linkPreviews: v.linkPreviews !== false,
    superReactions: v.superReactions !== false,
    // Profiles (styles/profiles.css): other people's name styles and cosmetics.
    plainNames: v.plainNames === true,
    reduceCosmetics: v.reduceCosmetics === true,
    theme: choice(v.theme, ['dark', 'light', 'darker', 'system', 'preset', 'custom', 'shared'], 'dark'),
    themeId: typeof v.themeId === 'string' && /^[a-z0-9]{1,24}$/.test(v.themeId) ? v.themeId : '',
    // A shared theme applied without saving it: its share code.
    themeCode: typeof v.themeCode === 'string' && /^[A-Za-z0-9_-]{1,256}$/.test(v.themeCode) ? v.themeCode : '',
    saturation: Math.round(number(v.saturation, 0, 100, 100)),
    saturationCustom: v.saturationCustom === true,
    contrastBoost: v.contrastBoost === true,
    roleColors: choice(v.roleColors, ['names', 'dot', 'off'], 'names'),
    focusAlways: v.focusAlways === true,
    motion: choice(v.motion, ['system', 'on', 'off'], 'system'),
    autoplayGifs: choice(v.autoplayGifs, autoplay, 'always'),
    autoplayEmoji: choice(v.autoplayEmoji, autoplay, 'always'),
    autoplayStickers: choice(v.autoplayStickers, autoplay, 'always'),
    autoplayAvatars: choice(v.autoplayAvatars, autoplay, 'hover'),
    autoplayEffects: choice(v.autoplayEffects, autoplay, 'always'),
    chatFont: choice(v.chatFont, ['inter', 'system', 'serif', 'mono', 'atkinson', 'dyslexic'], 'inter'),
    messageSpacing: Math.round(number(v.messageSpacing, 0, 16, 2)),
    groupSpacing: Math.round(number(v.groupSpacing, 0, 32, 16)),
    zoom: [0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2].includes(Number(v.zoom)) ? Number(v.zoom) : 1,
    sendButton: v.sendButton !== false,
    ttsPlayback: v.ttsPlayback !== false,
    ttsNotify: choice(v.ttsNotify, ['off', 'channel', 'all'], 'off'),
    ttsRate: [0.5, 0.75, 1, 1.25, 1.5, 2].includes(Number(v.ttsRate)) ? Number(v.ttsRate) : 1,
    streamer: v.streamer === true,
    streamerAuto: v.streamerAuto === true,
    streamerAccount: v.streamerAccount !== false,
    streamerInvites: v.streamerInvites !== false,
    streamerNotes: v.streamerNotes !== false,
    streamerRequests: v.streamerRequests !== false,
    streamerPopups: v.streamerPopups !== false,
    streamerSounds: v.streamerSounds !== false,
    streamerPreviews: choice(v.streamerPreviews, ['blur', 'hide', 'show'], 'blur'),
    appIcon: choice(v.appIcon, APP_ICONS, 'mint') };
}
let preferences = { ...defaults };
// The applied theme: the personal one, and a server's while it is in view.
let theme = { css: '', color: '#111113', dark: true, gradient: false };
let serverTheme = null;
// Turned on by a screen share ("enable automatically"), to be turned off after.
let streamerByShare = false;
const STORE = 'sticktalk-appearance', BOOT = 'sticktalk-boot';
const reduced = matchMedia('(prefers-reduced-motion: reduce)');
const prefersLight = matchMedia('(prefers-color-scheme: light)');
const sheet = globalThis.__sticktalkThemeSheet ?? (() => {
  try { const s = new CSSStyleSheet(); document.adoptedStyleSheets = [...document.adoptedStyleSheets, s]; return s; } catch { return null; }
})();
globalThis.__sticktalkThemeSheet = sheet;
function save() {
  try { localStorage.setItem(STORE, JSON.stringify(preferences)); } catch {}
}
export function preferencesJson() {
  return JSON.stringify(preferences);
}
export function loadPreferences() {
  try {
    const raw = localStorage.getItem(STORE);
    if(raw && raw.length <= 4096) preferences = normalizePreferences(JSON.parse(raw));
  } catch { /* Storage may be disabled in a private browser. */ }
  try {
    const boot = JSON.parse(localStorage.getItem(BOOT) ?? 'null');
    if (boot && typeof boot.css === 'string' && boot.css.length <= 16384)
      theme = { css: boot.css, color: String(boot.color ?? '#111113'), dark: boot.dark !== false, gradient: boot.gradient === true };
  } catch {}
  // The OS settings are followed live: these fire only when they change.
  reduced.addEventListener('change', () => applyPreferences());
  prefersLight.addEventListener('change', () => {
    applyPreferences();
    // "Sync with system" with derived tokens (saturation, contrast boost).
    if (preferences.theme === 'system' && theme.css) tell('appearance:refresh');
  });
  install();
  applyPreferences();
}
export function systemLight() { return prefersLight.matches; }
let installed = false;
const SECRET = '.streamer-invite,.streamer-account,.streamer-note,#p-note,.streamer-request,.request-messages,.markdown a[href*="invite="],section[aria-labelledby="authsec-email-title"] :is(p,strong,code),section[aria-labelledby="authsec-passkeys-title"] li';
function install() {
  if (installed) return;
  installed = true;
  // Streamer Mode: a covered value is revealed by its first click only.
  document.addEventListener('click', (e) => {
    if (!preferences.streamer) return;
    const secret = e.target.closest?.(SECRET);
    if (!secret || secret.classList.contains('revealed')) return;
    e.preventDefault();
    e.stopPropagation();
    secret.classList.add('revealed');
  }, true);
  document.addEventListener('keydown', (e) => {
    if (e.ctrlKey && e.altKey && e.shiftKey && !e.metaKey && e.code === 'KeyS') { e.preventDefault(); toggleStreamer(); }
  });
  addEventListener('sticktalk-sharing', (e) => sharingChanged(e.detail === true));
  // The theme picker's radios and live slider previews.
  document.addEventListener('change', (e) => {
    if (e.target.matches?.('input[data-theme-choice]') && e.target.checked) tell(`appearance:theme,${e.target.value}`);
  });
  document.addEventListener('input', (e) => {
    if (e.target.matches?.('input[type="range"][data-preference]')) previewPreference(e.target);
    const out = e.target.matches?.('input[type="range"][data-output]:not([data-preference])') && document.getElementById(e.target.dataset.output);
    if (out) out.textContent = `${e.target.value}${e.target.dataset.unit ?? ''}`;
  });
  // Synced settings (server themes) and the theme editor: Rust acts on them.
  document.addEventListener('change', (e) => {
    const t = e.target;
    if (t.matches?.('[data-appearance]')) tell(`appearance:${t.dataset.appearance},${t.type === 'checkbox' ? t.checked : t.value}`);
    else if (t.closest?.('[data-theme-editor]')) tell('appearance:editor-input');
  });
  // GIFs that would autoplay are paused unless the setting allows it; with
  // "hover" they play while hovered or focused. No timers.
  const gif = (el) => el?.closest?.('.chat-gif, .gif-result, .gif-category')?.querySelectorAll('video[autoplay]') ?? [];
  document.addEventListener('play', (e) => {
    const v = e.target;
    if (!(v instanceof HTMLVideoElement) || !v.autoplay || !v.closest('.chat-gif, .live-gif-results, .gif-categories, .gif-result')) return;
    if (preferences.autoplayGifs === 'never' || (preferences.autoplayGifs === 'hover' && !v.dataset.hovered)) v.pause();
  }, true);
  const hover = (on) => (e) => {
    if (preferences.autoplayGifs !== 'hover') return;
    for (const v of gif(e.target)) { if (on) { v.dataset.hovered = '1'; v.play().catch(() => {}); } else { delete v.dataset.hovered; v.pause(); } }
  };
  document.addEventListener('pointerover', hover(true));
  document.addEventListener('pointerout', (e) => { if (!e.relatedTarget || !e.target.closest?.('.chat-gif, .gif-result, .gif-category')?.contains(e.relatedTarget)) hover(false)(e); });
  document.addEventListener('focusin', hover(true));
  document.addEventListener('focusout', hover(false));
}
const attrs = () => {
  const p = preferences, a = {};
  a.density = p.density;
  a.underlineLinks = String(p.underlineLinks);
  a.linkPreviews = String(p.linkPreviews);
  a.superReactions = String(p.superReactions);
  a.plainNames = String(p.plainNames);
  a.reduceCosmetics = String(p.reduceCosmetics);
  a.themeBase = ['preset', 'custom', 'shared'].includes(p.theme) ? (theme.dark ? 'dark' : 'light') : p.theme;
  a.motion = p.motion === 'on' || (p.motion === 'system' && reduced.matches) ? 'reduce' : 'full';
  a.roleColors = p.roleColors;
  a.focusAlways = String(p.focusAlways);
  a.chatFont = p.chatFont;
  a.sendButton = String(p.sendButton);
  for (const k of ['Gifs', 'Emoji', 'Stickers', 'Avatars', 'Effects']) a[`autoplay${k}`] = p[`autoplay${k}`];
  // Role names are kept readable on light themes, with saturation and boost.
  const effective = serverTheme ?? theme;
  const light = a.themeBase === 'light' || (a.themeBase === 'system' && prefersLight.matches) || !effective.dark && !!effective.css;
  a.roleAdjust = String(light || p.saturation < 100 || p.contrastBoost);
  a.themeGradient = String(!!effective.gradient);
  a.streamer = String(p.streamer);
  if (p.streamer) {
    a.streamerAccount = String(p.streamerAccount);
    a.streamerInvites = String(p.streamerInvites);
    a.streamerNotes = String(p.streamerNotes);
    a.streamerRequests = String(p.streamerRequests);
    a.streamerPreviews = p.streamerPreviews;
  }
  return a;
};
const vars = () => {
  const p = preferences;
  return {
    '--chat-font-size': `${15 * p.fontScale}px`,
    '--message-spacing': `${p.messageSpacing}px`,
    '--group-spacing': `${p.groupSpacing}px`,
    '--app-zoom': String(p.zoom),
    '--role-saturation': String(p.saturation / 100),
  };
};
function paintTheme() {
  const t = serverTheme ?? theme;
  if (sheet) { try { sheet.replaceSync(t.css ? `:root:root:root{${t.css}}` : ''); } catch {} }
  const meta = document.querySelector('meta[name="theme-color"]');
  // "Sync with system" and the static bases: the rail color of what is shown.
  const color = t.css ? t.color : getComputedStyle(document.documentElement).getPropertyValue('--bg-rail').trim() || t.color;
  if (meta && color) meta.content = color;
  tellShell(color, t.css ? t.dark : !(document.documentElement.dataset.themeBase === 'light' || (document.documentElement.dataset.themeBase === 'system' && prefersLight.matches)));
  return color;
}
// The Windows app's title bar follows the theme (crates/desktop shell IPC).
let shellSent = '';
function tellShell(color, dark) {
  if (!globalThis.__sticktalkHost?.theme || !globalThis.ipc?.postMessage) return;
  const message = JSON.stringify({ shell: 'theme', dark: !!dark, caption: color });
  if (message !== shellSent) { shellSent = message; globalThis.ipc.postMessage(message); }
}
export function applyPreferences() {
  const root = document.documentElement;
  const a = attrs();
  for (const [k, v] of Object.entries(a)) root.dataset[k] = v;
  if (!preferences.streamer) for (const k of ['streamerAccount', 'streamerInvites', 'streamerNotes', 'streamerRequests', 'streamerPreviews']) delete root.dataset[k];
  const v = vars();
  for (const [k, value] of Object.entries(v)) root.style.setProperty(k, value);
  root.style.zoom = preferences.zoom === 1 ? '' : String(preferences.zoom);
  const color = paintTheme();
  document.querySelectorAll('[data-preference]').forEach(field=>{
    const value=preferences[field.dataset.preference];
    if(field.type==='checkbox')field.checked=value;
    else if(field.type==='radio')field.checked=field.value===String(value);
    else field.value=String(value);
    const out = field.dataset.output && document.getElementById(field.dataset.output);
    if (out) out.textContent = format(field.dataset.preference, value);
  });
  document.querySelectorAll('input[data-theme-choice]').forEach(r => { r.checked = r.value === selection(); });
  document.querySelectorAll('input[data-app-icon]').forEach(r => { r.checked = r.value === preferences.appIcon; });
  icon();
  indicator();
  // What theme-boot.js replays before the next first paint.
  try { localStorage.setItem(BOOT, JSON.stringify({ attrs: a, vars: v, zoom: preferences.zoom, css: theme.css, color, dark: theme.dark, gradient: theme.gradient, icon: preferences.appIcon })); } catch {}
}
export function format(key, value) {
  if (key === 'saturation') return `${value}%`;
  if (key === 'messageSpacing' || key === 'groupSpacing') return `${value}px`;
  if (key === 'ttsRate') return `${value}×`;
  return String(value);
}
/// "dark", "preset:aurora", "custom:123": the radio value of the chosen theme.
export function selection() {
  return preferences.theme === 'preset' || preferences.theme === 'custom' ? `${preferences.theme}:${preferences.themeId}` : preferences.theme;
}
const tell = (action) => dispatchEvent(new CustomEvent('sticktalk-live', { detail: action }));
export function setPreference(field) {
  const key=field.dataset.preference;
  if(!Object.hasOwn(defaults,key))return;
  if (field.type === 'radio' && !field.checked) return;
  const raw = field.type === 'checkbox' ? field.checked : field.value;
  preferences=normalizePreferences({...preferences,[key]:raw});
  if (key === 'streamer') streamerByShare = false;
  save();
  applyPreferences();
  // The theme's tokens depend on these: Rust derives them again.
  if (['saturation', 'saturationCustom', 'contrastBoost'].includes(key)) tell('appearance:refresh');
  if (key === 'appIcon') tell('appearance:icon');
}
/// Live preview while a slider moves; `change` stores it.
export function previewPreference(field) {
  const key = field.dataset.preference;
  if (!['messageSpacing', 'groupSpacing', 'saturation', 'ttsRate'].includes(key)) return;
  const out = field.dataset.output && document.getElementById(field.dataset.output);
  if (out) out.textContent = format(key, Number(field.value));
  if (key === 'messageSpacing' || key === 'groupSpacing') document.documentElement.style.setProperty(key === 'messageSpacing' ? '--message-spacing' : '--group-spacing', `${Number(field.value)}px`);
  if (key === 'saturation') document.documentElement.style.setProperty('--role-saturation', String(Number(field.value) / 100));
}
/// Rust: the chosen theme and its derived tokens. `css` empty means a base
/// theme drawn by the static token sheet.
export function applyTheme(kind, id, css, color, dark, gradient) {
  preferences = normalizePreferences({ ...preferences, theme: kind, themeId: kind === 'shared' ? '' : id, themeCode: kind === 'shared' ? id : '' });
  theme = { css, color, dark, gradient };
  save();
  applyPreferences();
}
/// Rust: a server's theme while that server is in view (`css` empty: none).
export function applyServerTheme(css, color, dark, gradient) {
  serverTheme = css ? { css, color, dark, gradient } : null;
  applyPreferences();
}
export function setStreamer(on, byShare = false) {
  if (preferences.streamer === on) return;
  preferences = normalizePreferences({ ...preferences, streamer: on });
  streamerByShare = on && byShare;
  save();
  applyPreferences();
}
export function toggleStreamer() { setStreamer(!preferences.streamer); }
/// A screen share started or stopped in a call (media.js).
export function sharingChanged(sharing) {
  if (sharing && preferences.streamerAuto && !preferences.streamer) setStreamer(true, true);
  else if (!sharing && streamerByShare) setStreamer(false);
}
/// Notification popups and sounds while Streamer Mode is on.
export function alertsAllowed() {
  return JSON.stringify({ popups: !(preferences.streamer && preferences.streamerPopups), sounds: !(preferences.streamer && preferences.streamerSounds) });
}
function icon() {
  const href = `/public/app-icons/${preferences.appIcon}-192.png`;
  for (const link of document.querySelectorAll('link[rel~="icon"],link[rel="apple-touch-icon"]')) if (!link.href.endsWith(href)) link.href = href;
  const manifest = document.querySelector('link[rel="manifest"]');
  const m = preferences.appIcon === 'mint' ? '/public/manifest.webmanifest' : `/public/app-icons/${preferences.appIcon}.webmanifest`;
  if (manifest && !manifest.href.endsWith(m)) manifest.href = m;
  for (const img of document.querySelectorAll('img[data-app-logo]')) if (!img.src.endsWith(href)) img.src = href;
  if (globalThis.__sticktalkHost?.icon && globalThis.ipc?.postMessage) shellIcon(href);
}
// The Windows app's window and taskbar icon: the page draws the chosen icon
// small and sends its pixels (the shell has no image decoder).
let shellIconSent = '';
function shellIcon(href) {
  if (shellIconSent === href) return;
  shellIconSent = href;
  const img = new Image();
  img.onload = () => {
    const size = 32, canvas = document.createElement('canvas');
    canvas.width = canvas.height = size;
    const g = canvas.getContext('2d');
    g.drawImage(img, 0, 0, size, size);
    const rgba = Array.from(g.getImageData(0, 0, size, size).data);
    globalThis.ipc.postMessage(JSON.stringify({ shell: 'icon', size, rgba }));
  };
  img.src = href;
}
function indicator() {
  let el = document.getElementById('streamer-indicator');
  if (!preferences.streamer) { el?.remove(); return; }
  if (el) return;
  el = document.createElement('button');
  el.id = 'streamer-indicator';
  el.className = 'streamer-indicator';
  el.type = 'button';
  el.dataset.live = 'dialog:streamer';
  el.setAttribute('aria-label', 'Streamer Mode is on. Open Streamer Mode settings.');
  el.textContent = 'Streamer Mode';
  document.body.append(el);
}
