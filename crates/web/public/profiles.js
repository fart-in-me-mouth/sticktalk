// Profile editor in the browser (docs/features/profiles.md): cropping avatars
// and banners before upload, the cosmetic pickers, badge ordering, and the
// live preview. Loaded on first use by live.js. No timers, no animation loop:
// everything runs in response to an input, a click or a render.
const $ = (s, root = document) => root.querySelector(s);
const $$ = (s, root = document) => [...root.querySelectorAll(s)];
let emit = () => {};

// ---- Colors (the same rules as crates/proto/src/profiles.rs) -------------
const channel = (c) => { c /= 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; };
const lum = (rgb) => 0.2126 * channel((rgb >> 16) & 255) + 0.7152 * channel((rgb >> 8) & 255) + 0.0722 * channel(rgb & 255);
export const contrast = (a, b) => { const [x, y] = [lum(a), lum(b)]; return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
const darken = (rgb, p) => { const k = 100 - p; const c = (s) => Math.floor(((rgb >> s) & 255) * k / 100); return (c(16) << 16) | (c(8) << 8) | c(0); };
const LIGHT = 0xE8EAED, DARK = 0x111113, NAME_SURFACE = 0x2E2E34;
export function cardTheme(p, a) {
  const worst = (t, s) => Math.min(contrast(t, darken(p, s)), contrast(t, darken(a, s)));
  if (worst(DARK, 0) >= 7) return { text: DARK, scrim: 0 };
  for (let s = 0; s <= 90; s += 5) if (worst(LIGHT, s) >= 7) return { text: LIGHT, scrim: s };
  return { text: LIGHT, scrim: 90 };
}
export const legible = (rgb) => contrast(rgb, NAME_SURFACE) >= 4.5;
const num = (hex) => parseInt(String(hex || '').replace('#', ''), 16) || 0;
const hex = (n) => '#' + n.toString(16).padStart(6, '0');

// ---- Editor: dirty state, preview -----------------------------------------
const field = (form, id) => form.querySelector('#' + id);
const val = (form, id) => field(form, id)?.value ?? '';
const on = (form, id) => !!field(form, id)?.checked;
const changed = (el) => el.type === 'checkbox' ? String(el.checked) !== el.dataset.saved : el.value !== el.dataset.saved;
// Text the app rendered is changed in place, never replaced: the app keeps
// its own nodes, so its next render still finds them.
function text(el, value) {
  if (!el) return;
  const node = el.firstChild;
  if (node?.nodeType === 3 && !node.nextSibling) node.data = value;
  else if (!el.firstChild) el.append(value);
}
function choice(form, id) {
  const el = field(form, id);
  return el ? (el.value || el.dataset.fallback || '') : '';
}
function nameStyle(form) {
  if (on(form, 'pf-name-on')) {
    const effect = val(form, 'pf-name-effect');
    const two = ['gradient', 'shimmer', 'outline'].includes(effect);
    const c1 = val(form, 'pf-name-c1'), c2 = val(form, 'pf-name-c2');
    return { cls: `styled-name name-font-${val(form, 'pf-name-font')} name-effect-${effect}`, vars: `--name-c1:${c1};${two ? `--name-c2:${c2}` : ''}`, colors: two ? [c1, c2] : [c1], two };
  }
  const fallback = form.dataset.fallbackStyle;
  if (fallback) { const [cls, vars] = fallback.split('|'); return { cls, vars, colors: [] }; }
  return null;
}
function paintName(el, value, style) {
  const span = el?.firstElementChild;
  if (!span) return;
  text(span, value);
  span.className = style ? `${style.cls} is-self` : '';
  span.style.cssText = style ? style.vars : '';
}
function paintCard(form, root) {
  const card = $('[data-pv="card"]', root);
  if (!card) return;
  let p = null, a = null;
  if (on(form, 'pf-colors-on')) { p = num(val(form, 'pf-primary')); a = num(val(form, 'pf-accent')); }
  else if (form.dataset.fallbackPrimary || form.dataset.fallbackAccent) {
    p = form.dataset.fallbackPrimary ? num(form.dataset.fallbackPrimary) : null;
    a = form.dataset.fallbackAccent ? num(form.dataset.fallbackAccent) : null;
  }
  if (p === null && a === null) {
    card.classList.remove('tinted', 'dark-text');
    for (const v of ['--card-p', '--card-a', '--card-text', '--card-scrim']) card.style.removeProperty(v);
    return;
  }
  p ??= a; a ??= p;
  const t = cardTheme(p, a);
  card.classList.add('tinted');
  card.classList.toggle('dark-text', t.text === DARK);
  card.style.setProperty('--card-p', hex(p));
  card.style.setProperty('--card-a', hex(a));
  card.style.setProperty('--card-text', hex(t.text));
  card.style.setProperty('--card-scrim', String(t.scrim / 100));
}
function paintCosmetic(root, key, prefix, value) {
  for (const el of $$(`[data-pv="${key}"]`, root)) {
    const base = el.dataset.pvBase ??= [...el.classList].filter((c) => !c.startsWith(prefix) && c !== 'is-self').join(' ');
    el.className = value ? `${base} ${prefix}${value} is-self` : base;
    el.hidden = !value;
  }
}
/// Bring a form's save bar, warnings and preview in line with its fields.
export function sync(form) {
  if (!form?.matches?.('[data-profiles-editor]')) return;
  const dirty = [...form.elements].some((el) => el.dataset.saved !== undefined && changed(el));
  form.dataset.dirty = String(dirty);
  const bar = $('.profile-save-bar', form);
  if (bar) bar.hidden = !dirty;
  for (const b of $$('[data-profiles^="pick,"]', form)) {
    const [, id, value] = b.dataset.profiles.split(',');
    b.setAttribute('aria-pressed', String(val(form, id) === value));
  }
  if (form.dataset.liveForm === 'profiles-badges') return syncBadges(form);
  const root = form.closest('.profile-editor') ?? document;
  const style = nameStyle(form);
  const shown = (form.dataset.liveForm === 'profiles-user' ? val(form, 'f-name') : val(form, 'pf-nickname')).trim() || form.dataset.fallbackName || '';
  for (const key of ['name', 'msg-name', 'row-name']) paintName($(`[data-pv="${key}"]`, root), shown, style);
  const pronouns = val(form, 'pf-pronouns').trim() || form.dataset.fallbackPronouns || '';
  const pv = $('[data-pv="pronouns"]', root);
  text(pv, pronouns ? ` · ${pronouns}` : '');
  const status = $('[data-preview-status]', root);
  if (status && field(form, 'f-value')) text(status, val(form, 'f-value'));
  const aboutField = field(form, 'pf-about');
  if (aboutField) {
    const about = aboutField.value.trim() || form.dataset.fallbackAbout || '';
    const box = $('[data-pv="about-box"]', root);
    if (box) box.hidden = !about;
    // Markdown is drawn by the app once saved; while typing, the text itself.
    const typed = changed(aboutField) || (!aboutField.value.trim() && !!form.dataset.fallbackAbout);
    const draft = $('[data-pv="about-draft"]', root), shownAbout = $('[data-pv="about"]', root);
    if (draft && shownAbout) { draft.textContent = about; draft.hidden = !typed; shownAbout.hidden = typed; }
    const count = $('[data-pv="about-count"]', form);
    if (count) count.textContent = `${aboutField.value.length}/${aboutField.maxLength}`;
  }
  const banner = $('[data-pv="banner"]', root);
  if (banner && field(form, 'pf-banner-color')) banner.style.background = val(form, 'pf-banner-color');
  paintCard(form, root);
  paintCosmetic(root, 'deco', 'deco-', choice(form, 'pf-deco'));
  paintCosmetic(root, 'plate', 'plate-', choice(form, 'pf-plate'));
  paintCosmetic(root, 'effect', 'effect-', choice(form, 'pf-effect'));
  paintCosmetic(root, 'frame', 'frame-', choice(form, 'pf-frame'));
  // Name style: second color only where the effect uses it; readable colors only.
  const styled = on(form, 'pf-name-on');
  for (const id of ['pf-name-font', 'pf-name-effect', 'pf-name-c1', 'pf-name-c2']) {
    const el = field(form, id);
    if (el) el.disabled = !styled;
  }
  const c2 = field(form, 'pf-name-c2')?.closest('label');
  if (c2) c2.hidden = !(style?.two ?? false);
  const warning = $('[data-pv="name-warning"]', form);
  const bad = styled && style.colors.some((c) => !legible(num(c)));
  if (warning) warning.hidden = !bad;
  const submit = $('.profile-save-bar [type=submit]', form);
  if (submit) submit.disabled = bad;
}
export function reset(form) {
  for (const el of form.elements) {
    if (el.dataset.saved === undefined) continue;
    if (el.type === 'checkbox') el.checked = el.dataset.saved === 'true';
    else el.value = el.dataset.saved;
    delete el.dataset.dirty;
  }
  delete form.dataset.dirty;
  sync(form);
}

// ---- Badges ---------------------------------------------------------------
function syncBadges(form) {
  const rows = $$('.badge-row', form);
  rows.forEach((row, i) => {
    $('[data-profiles="badge-up"]', row).disabled = i === 0;
    $('[data-profiles="badge-down"]', row).disabled = i === rows.length - 1;
  });
  const shown = rows.filter((r) => $('[data-profiles-badge]', r).checked).map((r) => r.dataset.badge).slice(0, 6);
  const hidden = field(form, 'pf-badges');
  if (hidden) hidden.value = shown.join(',');
  const full = shown.length >= 6;
  for (const box of $$('[data-profiles-badge]', form)) box.disabled = full && !box.checked;
}
function moveBadge(button, up) {
  const row = button.closest('.badge-row');
  const other = up ? row.previousElementSibling : row.nextElementSibling;
  if (!other) return;
  if (up) other.before(row); else other.after(row);
  const form = row.closest('form');
  form.dataset.dirty = 'true';
  sync(form);
  $(`[data-profiles="${up ? 'badge-up' : 'badge-down'}"]`, row).focus();
}

// ---- Cropping -------------------------------------------------------------
const MAX_BYTES = 10 * 1024 * 1024;
async function animatedFile(file) {
  const bytes = new Uint8Array(await file.slice(0, 1 << 20).arrayBuffer());
  const ascii = (i, s) => [...s].every((c, k) => bytes[i + k] === c.charCodeAt(0));
  if (ascii(0, 'GIF8')) {
    let frames = 0;
    for (let i = 0; i + 2 < bytes.length && frames < 2; i++) if (bytes[i] === 0x21 && bytes[i + 1] === 0xF9 && bytes[i + 2] === 4) frames++;
    return frames > 1;
  }
  if (bytes[0] === 0x89 && ascii(1, 'PNG')) {
    for (let i = 8; i + 8 < bytes.length;) {
      const len = (bytes[i] << 24 | bytes[i + 1] << 16 | bytes[i + 2] << 8 | bytes[i + 3]) >>> 0;
      if (ascii(i + 4, 'acTL')) return true;
      if (ascii(i + 4, 'IDAT')) return false;
      i += 12 + len;
    }
    return false;
  }
  if (ascii(0, 'RIFF') && ascii(8, 'WEBP')) {
    for (let i = 12; i + 4 < Math.min(bytes.length, 4096); i++) if (ascii(i, 'ANIM')) return true;
  }
  return false;
}
let crop = null;
function placeCrop() {
  if (!crop || crop.dialog.isConnected) return;
  crop.dialog.removeAttribute('open');
  ($('.live-modal') ?? document.body).append(crop.dialog);
  crop.dialog.showModal();
  draw();
}
function geometry(vw, vh) {
  const { bitmap: b, zoom, x, y } = crop;
  const s0 = Math.max(vw / b.width, vh / b.height);
  const dw = b.width * s0, dh = b.height * s0, z = zoom / 100;
  return { dw, dh, z, ox: (vw - dw) * x / 100, oy: (vh - dh) * y / 100, fx: vw * x / 100, fy: vh * y / 100 };
}
/// The same mapping as the CSS that shows a crop (object-fit: cover,
/// object-position x% y%, scale(zoom) about x% y%), so what you see is what
/// everyone sees.
function render(ctx, vw, vh) {
  const g = geometry(vw, vh);
  ctx.save();
  ctx.clearRect(0, 0, vw, vh);
  ctx.translate(g.fx, g.fy); ctx.scale(g.z, g.z); ctx.translate(-g.fx, -g.fy);
  ctx.drawImage(crop.bitmap, g.ox, g.oy, g.dw, g.dh);
  ctx.restore();
}
function draw() {
  if (!crop) return;
  const canvas = $('canvas', crop.dialog), box = canvas.getBoundingClientRect();
  const ratio = devicePixelRatio || 1;
  canvas.width = Math.max(1, Math.round(box.width * ratio));
  canvas.height = Math.max(1, Math.round(box.height * ratio));
  render(canvas.getContext('2d'), canvas.width, canvas.height);
  $('[data-crop-zoom]', crop.dialog).value = String(crop.zoom);
}
function pan(dx, dy) {
  const stage = $('.crop-stage', crop.dialog).getBoundingClientRect();
  const g = geometry(stage.width, stage.height);
  const move = (pos, d, view, size) => {
    const span = view - size * g.z;
    if (span >= -0.5) return 50;
    const left = pos / 100 * span + d;
    return Math.min(100, Math.max(0, left / span * 100));
  };
  crop.x = move(crop.x, dx, stage.width, g.dw);
  crop.y = move(crop.y, dy, stage.height, g.dh);
  draw();
}
function closeCrop() {
  crop?.bitmap.close?.();
  crop?.dialog.remove();
  crop = null;
}
function uploadFile(file, args) {
  let input = document.getElementById('profiles-upload');
  if (!input) {
    input = document.createElement('input');
    input.type = 'file'; input.id = 'profiles-upload'; input.hidden = true;
    document.body.append(input);
  }
  const dt = new DataTransfer();
  dt.items.add(file);
  input.files = dt.files;
  emit(`profiles:upload,${args}`);
}
async function applyCrop() {
  const c = crop;
  const zoom = Math.round(c.zoom), x = Math.round(c.x), y = Math.round(c.y);
  const head = `${c.slot},${c.guild}`;
  if (c.animated) {
    // An animation is uploaded as it is and cropped where it is shown.
    closeCrop();
    uploadFile(c.file, `${head},${zoom},${x},${y}`);
    return;
  }
  const [w, h] = c.slot === 'banner' ? [1500, 600] : [512, 512];
  const canvas = document.createElement('canvas');
  canvas.width = w; canvas.height = h;
  render(canvas.getContext('2d'), w, h);
  const type = c.slot === 'banner' ? 'image/jpeg' : 'image/webp';
  let blob = await new Promise((r) => canvas.toBlob(r, type, 0.92));
  if (!blob || blob.type !== type) blob = await new Promise((r) => canvas.toBlob(r, 'image/png'));
  closeCrop();
  if (!blob) { emit('profiles:error,That image could not be prepared. Try another one.'); return; }
  const ext = blob.type.split('/')[1];
  uploadFile(new File([blob], `${c.slot}.${ext}`, { type: blob.type }), `${head},100,50,50`);
}
function openCrop(state) {
  closeCrop();
  const banner = state.slot === 'banner';
  const dialog = document.createElement('dialog');
  dialog.id = 'profiles-crop';
  dialog.className = 'confirm-dialog crop-dialog';
  dialog.setAttribute('aria-labelledby', 'profiles-crop-title');
  dialog.innerHTML = `<h2 id="profiles-crop-title"></h2><p></p>
    <div class="crop-stage" tabindex="0" role="group" aria-label="Drag or use the arrow keys to move the picture"><canvas></canvas><div class="crop-mask"></div></div>
    <label class="field mb-0"><span>Zoom</span><input type="range" min="100" max="400" step="1" data-crop-zoom></label>
    <div class="confirm-actions"><button type="button" class="button secondary" data-crop-cancel>Cancel</button><button type="button" class="button primary" id="profiles-crop-apply">Apply</button></div>`;
  $('h2', dialog).textContent = banner ? 'Crop Banner' : 'Crop Avatar';
  $('p', dialog).textContent = state.animated ? 'This picture moves. It stays animated; the crop is applied wherever it is shown.' : 'Drag to move, and zoom to fit.';
  const stage = $('.crop-stage', dialog);
  stage.style.width = banner ? 'min(440px, 100%)' : 'min(280px, 100%)';
  stage.style.aspectRatio = banner ? '5 / 2' : '1';
  $('.crop-mask', dialog).classList.toggle('round', !banner);
  crop = { ...state, dialog, zoom: 100, x: 50, y: 50, drag: null };
  $('[data-crop-zoom]', dialog).addEventListener('input', (e) => { crop.zoom = Number(e.target.value); draw(); });
  stage.addEventListener('pointerdown', (e) => { crop.drag = [e.clientX, e.clientY]; stage.setPointerCapture(e.pointerId); });
  stage.addEventListener('pointermove', (e) => {
    if (!crop?.drag) return;
    const [px, py] = crop.drag; crop.drag = [e.clientX, e.clientY];
    pan(e.clientX - px, e.clientY - py);
  });
  stage.addEventListener('pointerup', () => { if (crop) crop.drag = null; });
  stage.addEventListener('keydown', (e) => {
    const step = e.shiftKey ? 24 : 6;
    const moves = { ArrowLeft: [step, 0], ArrowRight: [-step, 0], ArrowUp: [0, step], ArrowDown: [0, -step] };
    if (moves[e.key]) { e.preventDefault(); pan(...moves[e.key]); }
    if (e.key === '+' || e.key === '=') { crop.zoom = Math.min(400, crop.zoom + 10); draw(); }
    if (e.key === '-') { crop.zoom = Math.max(100, crop.zoom - 10); draw(); }
  });
  $('[data-crop-cancel]', dialog).addEventListener('click', closeCrop);
  $('#profiles-crop-apply', dialog).addEventListener('click', () => void applyCrop());
  dialog.addEventListener('cancel', (e) => { e.preventDefault(); closeCrop(); });
  placeCrop();
  stage.focus();
}
/// A picture was chosen in #live-avatar or #live-banner.
export async function choose(input, slot) {
  const file = input.files?.[0];
  const guild = input.dataset.profilesGuild || '0';
  input.value = '';
  if (!file) return;
  if (file.size > MAX_BYTES) { emit('profiles:error,That picture is larger than 10 MB.'); return; }
  let bitmap;
  try { bitmap = await createImageBitmap(file); }
  catch { emit('profiles:error,That file is not a picture StickTalk can use. Choose a PNG, JPEG, WebP or GIF.'); return; }
  openCrop({ file, bitmap, animated: await animatedFile(file), slot, guild });
}

// ---- Wiring ---------------------------------------------------------------
export function click(el) {
  const action = el.dataset.profiles;
  if (action.startsWith('pick,')) {
    const [, id, value] = action.split(',');
    const form = el.closest('form');
    const input = field(form, id);
    if (!input) return;
    input.value = value;
    input.dataset.dirty = 'true';
    form.dataset.dirty = 'true';
    sync(form);
  } else if (action === 'badge-up' || action === 'badge-down') moveBadge(el, action === 'badge-up');
}
let installed = false;
function install() {
  if (installed) return;
  installed = true;
  const changed = (e) => {
    if (e.target.matches?.('[data-profiles-guild-select]') && e.type === 'change') { emit(`profiles:guild,${e.target.value}`); return; }
    if (e.target.id === 'live-banner' && e.type === 'change') { void choose(e.target, 'banner'); return; }
    const form = e.target.closest?.('form[data-profiles-editor]');
    if (form) sync(form);
  };
  document.addEventListener('input', changed);
  document.addEventListener('change', changed);
  // A save changes only what the fields say is saved (data-saved): no element
  // comes or goes, so the page's own after-render pass does not run. Without
  // this the "unsaved changes" bar stayed up after every save but the first.
  new MutationObserver((records) => {
    const forms = new Set();
    for (const r of records) {
      const form = r.target.closest?.('form[data-profiles-editor]');
      if (form) forms.add(form);
    }
    for (const form of forms) sync(form);
  }).observe(document.body, { subtree: true, attributes: true, attributeFilter: ['data-saved'] });
  addEventListener('resize', () => { if (crop) draw(); });
}
/// After every render: forms re-read their state, a crop dialog is put back.
export function mount(send) {
  emit = send;
  install();
  for (const form of $$('form[data-profiles-editor]')) sync(form);
  placeCrop();
}
