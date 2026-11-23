// Server identity in the browser (docs/features/identity.md): cropping a
// server banner, icon or invite background before upload, the tag badge
// picker and the tag preview. Loaded on first use by live.js. No timers and
// no animation loop: everything runs in response to an input or a click.
const $ = (s, root = document) => root.querySelector(s);
let emit = () => {};

// ---- Colors (the same rule as crates/proto/src/identity.rs) -------------
const channel = (c) => { c /= 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; };
const lum = (rgb) => 0.2126 * channel((rgb >> 16) & 255) + 0.7152 * channel((rgb >> 8) & 255) + 0.0722 * channel(rgb & 255);
const contrast = (a, b) => { const [x, y] = [lum(a), lum(b)]; return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
const num = (hex) => parseInt(String(hex || '').replace('#', ''), 16) || 0;

// ---- Tag editor ------------------------------------------------------------
/// Keep the preview, badge picker and contrast warning in line with the form.
export function sync(form) {
  if (!form?.matches?.('[data-identity-tag]')) return;
  const text = $('#id-tag-text', form)?.value.trim() || 'TAG';
  const c1 = $('#id-tag-c1', form)?.value, c2 = $('#id-tag-c2', form)?.value;
  const glyph = $('#id-tag-glyph', form)?.value || 'leaf';
  const root = form.closest('.settings-content') ?? document;
  for (const chip of root.querySelectorAll('[data-tag-preview], .glyph-choice')) {
    const badge = $('.tag-badge', chip);
    if (badge) { badge.style.setProperty('--tag-c1', c1); badge.style.setProperty('--tag-c2', c2); }
  }
  const preview = $('[data-tag-preview]', root);
  if (preview) {
    const t = $('.tag-text', preview);
    if (t && t.firstChild?.nodeType === 3) t.firstChild.data = text;
    $('.tag-glyph', preview)?.style.setProperty('--tag-glyph', `url(/public/tags/${glyph}.svg)`);
  }
  for (const b of form.querySelectorAll('[data-identity-glyph]')) {
    const on = b.dataset.identityGlyph === glyph;
    b.setAttribute('aria-pressed', String(on));
    b.classList.toggle('border-primary', on);
    b.classList.toggle('bg-secondary', on);
  }
  const faint = contrast(num(c1), num(c2)) < 3;
  const warning = $('[data-identity-contrast]', form);
  if (warning) warning.hidden = !faint;
  const submit = $('[type=submit]', form);
  if (submit) submit.disabled = faint;
}
export function click(el) {
  const form = el.closest('form');
  const input = form && $('#id-tag-glyph', form);
  if (!input) return;
  input.value = el.dataset.identityGlyph;
  input.dataset.dirty = 'true';
  form.dataset.dirty = 'true';
  sync(form);
}

// ---- Cropping ----------------------------------------------------------------
const MAX_BYTES = 10 * 1024 * 1024;
// Aspect, output size and type of each slot's still picture.
const SLOTS = {
  banner: { title: 'Crop Banner', aspect: 5 / 2, size: [1500, 600], type: 'image/jpeg', stage: 'min(440px, 100%)' },
  icon: { title: 'Crop Server Icon', aspect: 1, size: [512, 512], type: 'image/webp', stage: 'min(280px, 100%)', round: true },
  background: { title: 'Crop Invite Background', aspect: 16 / 9, size: [1920, 1080], type: 'image/jpeg', stage: 'min(440px, 100%)' },
};
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
function place() {
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
/// object-position x% y%, scale(zoom) about x% y%).
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
    return Math.min(100, Math.max(0, (pos / 100 * span + d) / span * 100));
  };
  crop.x = move(crop.x, dx, stage.width, g.dw);
  crop.y = move(crop.y, dy, stage.height, g.dh);
  draw();
}
function close() {
  crop?.bitmap.close?.();
  crop?.dialog.remove();
  crop = null;
}
function upload(file, args) {
  let input = document.getElementById('identity-upload');
  if (!input) {
    input = document.createElement('input');
    input.type = 'file'; input.id = 'identity-upload'; input.hidden = true;
    document.body.append(input);
  }
  const dt = new DataTransfer();
  dt.items.add(file);
  input.files = dt.files;
  emit(`id:upload,${args}`);
}
async function apply() {
  const c = crop, spec = SLOTS[c.slot];
  const zoom = Math.round(c.zoom), x = Math.round(c.x), y = Math.round(c.y);
  if (c.animated) {
    // An animation is uploaded as it is and cropped where it is shown.
    close();
    upload(c.file, `${c.slot},${zoom},${x},${y}`);
    return;
  }
  const [w, h] = spec.size;
  const canvas = document.createElement('canvas');
  canvas.width = w; canvas.height = h;
  render(canvas.getContext('2d'), w, h);
  let blob = await new Promise((r) => canvas.toBlob(r, spec.type, 0.9));
  if (!blob || blob.type !== spec.type) blob = await new Promise((r) => canvas.toBlob(r, 'image/png'));
  close();
  if (!blob) { emit('id:error,That picture could not be prepared. Try another one.'); return; }
  upload(new File([blob], `${c.slot}.${blob.type.split('/')[1]}`, { type: blob.type }), `${c.slot},100,50,50`);
}
function open(state) {
  close();
  const spec = SLOTS[state.slot];
  const dialog = document.createElement('dialog');
  dialog.id = 'identity-crop';
  dialog.className = 'confirm-dialog crop-dialog';
  dialog.setAttribute('aria-labelledby', 'identity-crop-title');
  dialog.innerHTML = `<h2 id="identity-crop-title"></h2><p></p>
    <div class="crop-stage" tabindex="0" role="group" aria-label="Drag or use the arrow keys to move the picture"><canvas></canvas><div class="crop-mask"></div></div>
    <label class="field mb-0"><span>Zoom</span><input type="range" min="100" max="400" step="1" data-crop-zoom></label>
    <div class="confirm-actions"><button type="button" class="button secondary" data-crop-cancel>Cancel</button><button type="button" class="button primary" id="identity-crop-apply">Apply</button></div>`;
  $('h2', dialog).textContent = spec.title;
  $('p', dialog).textContent = state.animated ? 'This picture moves. It stays animated; the crop is applied wherever it is shown.' : 'Drag to move, and zoom to fit.';
  const stage = $('.crop-stage', dialog);
  stage.style.width = spec.stage;
  stage.style.aspectRatio = String(spec.aspect);
  $('.crop-mask', dialog).classList.toggle('round', !!spec.round);
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
  $('[data-crop-cancel]', dialog).addEventListener('click', close);
  $('#identity-crop-apply', dialog).addEventListener('click', () => void apply());
  dialog.addEventListener('cancel', (e) => { e.preventDefault(); close(); });
  place();
  stage.focus();
}
/// A picture was chosen in one of the Appearance page's inputs.
export async function choose(input) {
  const file = input.files?.[0], slot = input.dataset.identitySlot;
  input.value = '';
  if (!file || !SLOTS[slot]) return;
  if (file.size > MAX_BYTES) { emit('id:error,That picture is larger than 10 MB.'); return; }
  let bitmap;
  try { bitmap = await createImageBitmap(file); }
  catch { emit('id:error,That file is not a picture StickTalk can use. Choose a PNG, JPEG, WebP or GIF.'); return; }
  open({ file, bitmap, animated: await animatedFile(file), slot });
}

let installed = false;
/// After every render: forms re-read their state, a crop dialog is put back.
export function mount(send) {
  emit = send;
  if (!installed) {
    installed = true;
    addEventListener('resize', () => { if (crop) draw(); });
  }
  for (const form of document.querySelectorAll('form[data-identity-tag]')) sync(form);
  place();
}
