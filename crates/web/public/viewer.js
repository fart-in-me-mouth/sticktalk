// Gesture-loaded viewer. No original request or decoded image survives closing.
const $ = (s) => document.querySelector(s);
let root, image, original, source, scale = 1, x = 0, y = 0, fit = 1, generation = 0;
let priorFocus, pointers = new Map(), gesture;
const icon = (name) => {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.classList.add('icon'); svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('aria-hidden', 'true');
  const use = document.createElementNS(svg.namespaceURI, 'use');
  use.setAttribute('href', `#i-${name}`); svg.append(use); return svg;
};
function paint() {
  image.style.transform = `translate(${x}px,${y}px) scale(${scale / fit})`;
  root.querySelector('[data-zoom]').textContent = `${Math.round(scale * 100)}%`;
}
function zoom(next, cx = innerWidth / 2, cy = innerHeight / 2) {
  next = Math.max(.1, Math.min(8, next));
  x = (x - cx + innerWidth / 2) * next / scale + cx - innerWidth / 2;
  y = (y - cy + innerHeight / 2) * next / scale + cy - innerHeight / 2;
  scale = next; paint();
}
function reset() { scale = fit; x = y = 0; paint(); }
export function closeViewer() {
  generation++; pointers.clear(); gesture = null;
  if (original) { original.onload = original.onerror = null; original.src = ''; }
  if (image) image.src = '';
  root?.remove(); root = image = original = source = null;
  window.removeEventListener('keydown', keys);
  window.removeEventListener('resize', resize);
  priorFocus?.focus();
}
function resize() {
  if (!root || !image.naturalWidth) return;
  fit = Math.min(1, (innerWidth - 96) / image.naturalWidth, (innerHeight - 144) / image.naturalHeight);
  image.style.width = `${image.naturalWidth * fit}px`;
  image.style.height = `${image.naturalHeight * fit}px`;
  reset();
}
function navigate(step) {
  const all = [...document.querySelectorAll('[data-view-image]')];
  const i = all.indexOf(source);
  if (all[i + step]) openViewer(all[i + step]);
}
function keys(e) {
  if (e.key === 'Escape') closeViewer();
  else if (['+', '=', '-', '0', 'ArrowLeft', 'ArrowRight'].includes(e.key)) {
    e.preventDefault();
    if (e.key === '0') reset();
    else if (e.key === 'ArrowLeft') navigate(-1);
    else if (e.key === 'ArrowRight') navigate(1);
    else zoom(scale * (e.key === '-' ? .8 : 1.25));
  } else if (e.key === 'Tab') {
    const buttons = [...root.querySelectorAll('button,a[href]')];
    const i = buttons.indexOf(document.activeElement);
    e.preventDefault(); buttons[(i + (e.shiftKey ? -1 : 1) + buttons.length) % buttons.length].focus();
  }
}
export function openViewer(target) {
  closeViewer(); source = target; priorFocus = document.activeElement;
  const token = ++generation;
  root = document.createElement('div'); root.id = 'live-viewer'; root.className = 'live-viewer';
  root.dataset.channel = $('#live-messages')?.dataset.channel ?? '';
  root.setAttribute('role', 'dialog'); root.setAttribute('aria-modal', 'true'); root.setAttribute('aria-label', 'Image viewer');
  const toolbar = document.createElement('div'); toolbar.className = 'live-viewer-toolbar';
  const button = (label, glyph, action) => {
    const b = document.createElement('button'); b.className = 'icon-button';
    b.title = label; b.setAttribute('aria-label', label); b.append(icon(glyph)); b.onclick = action; toolbar.append(b); return b;
  };
  button('Previous image', 'arrow-left', () => navigate(-1));
  button('Next image', 'arrow-right', () => navigate(1));
  button('Zoom Out', 'minus', () => zoom(scale * .8));
  const percent = document.createElement('button'); percent.dataset.zoom = ''; percent.className = 'zoom-reset'; percent.title = 'Reset zoom'; percent.onclick = reset; toolbar.append(percent);
  button('Zoom In', 'plus', () => zoom(scale * 1.25));
  const save = document.createElement('a'); save.className = 'icon-button'; save.href = target.dataset.viewImage;
  save.download = target.dataset.filename; save.title = 'Save Image'; save.setAttribute('aria-label', 'Save Image'); save.append(icon('download')); toolbar.append(save);
  button('Close', 'x', closeViewer);
  image = new Image(); image.alt = target.dataset.filename; image.decoding = 'async'; image.draggable = false;
  image.className = 'viewer-loading';
  image.onload = resize;
  if (target.querySelector('img')) image.src = target.querySelector('img').src;
  original = new Image(); original.decoding = 'async';
  original.onload = () => {
    if (!root || generation !== token) return;
    image.src = original.src; image.classList.remove('viewer-loading'); root.querySelector('[role=status]').textContent = '';
  };
  original.onerror = () => { if (generation === token) root.querySelector('[role=status]').textContent = 'Could not load original.'; };
  // This is the only original-image request; the normal chat loads thumbnails.
  original.src = target.dataset.viewImage;
  const footer = document.createElement('footer');
  const label = document.createElement('span');
  const all = [...document.querySelectorAll('[data-view-image]')];
  label.textContent = `${target.dataset.filename} · ${Number(target.dataset.bytes ?? 0).toLocaleString()} bytes · ${all.indexOf(target) + 1} / ${all.length}`;
  const link = document.createElement('a'); link.href = target.dataset.viewImage; link.target = '_blank'; link.rel = 'noopener noreferrer'; link.textContent = 'Open original';
  const status = document.createElement('span'); status.setAttribute('role', 'status'); status.textContent = 'Loading original…';
  footer.append(label, link, status); root.append(toolbar, image, footer); document.body.append(root);
  toolbar.querySelector('button').focus();
  root.onclick = (e) => { if (e.target === root) closeViewer(); };
  root.onwheel = (e) => { e.preventDefault(); zoom(scale * (e.deltaY < 0 ? 1.2 : 1 / 1.2), e.clientX, e.clientY); };
  image.ondblclick = () => { if (Math.abs(scale - fit) < .001) zoom(1); else reset(); };
  image.onpointerdown = (e) => {
    e.preventDefault(); image.setPointerCapture(e.pointerId); pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    gesture = snapshot();
  };
  image.onpointermove = (e) => {
    if (!pointers.has(e.pointerId)) return;
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    const points = [...pointers.values()], center = midpoint(points);
    if (points.length === 2 && gesture.distance) {
      const next = Math.max(.1, Math.min(8, gesture.scale * distance(points) / gesture.distance));
      x = (gesture.x - gesture.cx + innerWidth / 2) * next / gesture.scale + center.x - innerWidth / 2;
      y = (gesture.y - gesture.cy + innerHeight / 2) * next / gesture.scale + center.y - innerHeight / 2;
      scale = next;
    } else if (scale > fit) { x = gesture.x + center.x - gesture.cx; y = gesture.y + center.y - gesture.cy; }
    paint();
  };
  const release = (e) => { pointers.delete(e.pointerId); gesture = pointers.size ? snapshot() : null; };
  image.onpointerup = image.onpointercancel = image.onlostpointercapture = release;
  window.addEventListener('keydown', keys); window.addEventListener('resize', resize);
}
function midpoint(points) { return { x: points.reduce((s,p) => s + p.x, 0) / points.length, y: points.reduce((s,p) => s + p.y, 0) / points.length }; }
function distance(p) { return p.length === 2 ? Math.hypot(p[0].x-p[1].x, p[0].y-p[1].y) : 0; }
function snapshot() { const p = [...pointers.values()], c = midpoint(p); return { x, y, cx: c.x, cy: c.y, scale, distance: distance(p) }; }
