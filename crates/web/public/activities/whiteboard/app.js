// Whiteboard: one shared canvas. Each finished stroke, shape, text or sticky
// note is one key of the session's document (`i/<pid>-<n>`), so a change
// sends only that item; strokes are simplified before they are sent. While
// someone draws, their stroke and cursor travel as ephemeral messages
// (at most 15 a second) and are drawn on an overlay. Undo and redo are per
// person. Text in items is in a `text` property, so the server screens it.
(async () => {
  const st = window.StickTalk;
  const app = document.getElementById('app');
  const el = (tag, cls = '', text = '') => {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text) e.textContent = text;
    return e;
  };
  const ctx0 = await st.ready();
  const me = () => st.context().you;

  /* theme-lint: artwork */
  const COLORS = ['#f2f2f5', '#ff6b6b', '#ffb84d', '#ffe066', '#51cf66', '#4dabf7', '#9775fa', '#212126'];
  const NOTES = ['#ffe066', '#ffc9c9', '#b2f2bb', '#a5d8ff'];
  const CURSOR = ['#ff6b6b', '#ffb84d', '#51cf66', '#4dabf7', '#9775fa', '#f783ac', '#63e6be', '#ffd43b'];
  /* theme-lint: end */
  const TOOLS = [
    ['pen', 'Pen', '✎'], ['hl', 'Highlighter', '▬'], ['eraser', 'Eraser', '⌫'], ['rect', 'Rectangle', '▭'],
    ['ellipse', 'Ellipse', '◯'], ['line', 'Line', '╱'], ['arrow', 'Arrow', '➜'], ['text', 'Text', 'T'],
    ['note', 'Sticky Note', '▤'], ['hand', 'Pan', '✋'],
  ];

  // ---- the page -------------------------------------------------------------------
  app.replaceChildren();
  const bar = el('div', 'toolbar');
  bar.setAttribute('role', 'toolbar');
  bar.setAttribute('aria-label', 'Whiteboard tools');
  const stage = el('div', 'stage');
  const base = el('canvas', 'base');
  const over = el('canvas', 'over');
  const editor = el('textarea', 'editor');
  editor.hidden = true;
  editor.maxLength = 500;
  editor.setAttribute('aria-label', 'Text');
  const meter = el('span', 'muted meter');
  const error = el('span', 'error');
  error.setAttribute('role', 'status');
  stage.append(base, over, editor);
  app.append(bar, stage);

  let tool = 'pen';
  let color = 0;
  const toolButtons = new Map();
  for (const [id, label, glyph] of TOOLS) {
    const b = el('button', 'tool', glyph);
    b.type = 'button';
    b.title = label;
    b.setAttribute('aria-label', label);
    b.setAttribute('aria-pressed', String(id === tool));
    b.onclick = () => setTool(id);
    toolButtons.set(id, b);
    bar.append(b);
  }
  const swatches = el('div', 'swatches');
  swatches.setAttribute('role', 'radiogroup');
  swatches.setAttribute('aria-label', 'Color');
  COLORS.forEach((c, i) => {
    const b = el('button', 'swatch');
    b.type = 'button';
    b.style.background = c;
    b.setAttribute('role', 'radio');
    b.setAttribute('aria-label', `Color ${i + 1}`);
    b.setAttribute('aria-checked', String(i === color));
    b.onclick = () => {
      color = i;
      for (const s of swatches.children) s.setAttribute('aria-checked', String(s === b));
    };
    swatches.append(b);
  });
  const button = (text, label, fn) => {
    const b = el('button', '', text);
    b.type = 'button';
    b.setAttribute('aria-label', label);
    b.title = label;
    b.onclick = fn;
    return b;
  };
  const undoButton = button('↶', 'Undo', () => undo());
  const redoButton = button('↷', 'Redo', () => redo());
  bar.append(swatches, undoButton, redoButton,
    button('−', 'Zoom Out', () => zoomAt(innerWidth / 2, innerHeight / 2, 1 / 1.25)),
    button('+', 'Zoom In', () => zoomAt(innerWidth / 2, innerHeight / 2, 1.25)),
    button('⤢', 'Fit Board', () => fit()),
    button('PNG', 'Export as PNG', () => exportPng()));
  const clearButton = button('Clear', 'Clear the Board', () => clearAll());
  bar.append(clearButton, meter, error);
  function setTool(id) {
    tool = id;
    for (const [k, b] of toolButtons) b.setAttribute('aria-pressed', String(k === id));
    over.style.cursor = id === 'hand' ? 'grab' : id === 'text' ? 'text' : 'crosshair';
  }
  setTool('pen');

  // ---- view: pan and zoom ----------------------------------------------------------
  let view = { x: 0, y: 0, z: 1 };
  let dpr = 1;
  const toWorld = (sx, sy) => [(sx - view.x) / view.z, (sy - view.y) / view.z];
  function resize() {
    dpr = Math.min(2, window.devicePixelRatio || 1);
    const r = stage.getBoundingClientRect();
    for (const c of [base, over]) {
      c.width = Math.max(1, Math.round(r.width * dpr));
      c.height = Math.max(1, Math.round(r.height * dpr));
    }
    redraw();
  }
  new ResizeObserver(resize).observe(stage);
  function zoomAt(sx, sy, f) {
    const r = stage.getBoundingClientRect();
    sx -= r.left;
    sy -= r.top;
    const z = Math.min(8, Math.max(0.1, view.z * f));
    view = { z, x: sx - (sx - view.x) * (z / view.z), y: sy - (sy - view.y) * (z / view.z) };
    redraw();
  }
  function fit() {
    const b = bounds();
    const r = stage.getBoundingClientRect();
    if (!b) { view = { x: 0, y: 0, z: 1 }; return redraw(); }
    const z = Math.min(4, Math.max(0.1, Math.min(r.width / (b.w + 80), r.height / (b.h + 80))));
    view = { z, x: (r.width - b.w * z) / 2 - b.x * z, y: (r.height - b.h * z) / 2 - b.y * z };
    redraw();
  }

  // ---- items ----------------------------------------------------------------------
  const items = new Map(); // key -> value
  let counter = 0;
  const nextKey = () => `i/${me()}-${++counter}`;
  const ordered = () => [...items.entries()].sort((a, b) => (a[1].z ?? 0) - (b[1].z ?? 0) || (a[0] < b[0] ? -1 : 1));
  function decode(p) {
    const out = [];
    if (!Array.isArray(p) || p.length < 2) return out;
    let x = p[0], y = p[1];
    out.push([x, y]);
    for (let i = 2; i + 1 < p.length; i += 2) { x += p[i]; y += p[i + 1]; out.push([x, y]); }
    return out;
  }
  function encode(points) {
    const out = [];
    let px = 0, py = 0;
    points.forEach(([x, y], i) => {
      x = Math.round(x); y = Math.round(y);
      if (i === 0) out.push(x, y); else out.push(x - px, y - py);
      px = x; py = y;
    });
    return out;
  }
  // Ramer–Douglas–Peucker: fewer points, same line.
  function simplify(points, tolerance) {
    if (points.length < 3) return points;
    const keep = new Uint8Array(points.length);
    keep[0] = keep[points.length - 1] = 1;
    const stack = [[0, points.length - 1]];
    while (stack.length) {
      const [a, b] = stack.pop();
      const [ax, ay] = points[a], [bx, by] = points[b];
      const dx = bx - ax, dy = by - ay, len = Math.hypot(dx, dy) || 1;
      let best = -1, far = tolerance;
      for (let i = a + 1; i < b; i++) {
        const d = Math.abs(dy * points[i][0] - dx * points[i][1] + bx * ay - by * ax) / len;
        if (d > far) { far = d; best = i; }
      }
      if (best > 0) { keep[best] = 1; stack.push([a, best], [best, b]); }
    }
    return points.filter((_, i) => keep[i]);
  }

  function drawItem(g, it, live = false) {
    g.save();
    const c = COLORS[it.c] ?? COLORS[0];
    g.strokeStyle = c;
    g.fillStyle = c;
    g.lineCap = g.lineJoin = 'round';
    g.lineWidth = it.w ?? 3;
    if (it.t === 'pen' || it.t === 'hl') {
      const pts = it.pts ?? decode(it.p);
      if (it.t === 'hl') { g.globalAlpha = 0.35; g.lineWidth = (it.w ?? 3) * 4; }
      g.beginPath();
      pts.forEach(([x, y], i) => (i ? g.lineTo(x, y) : g.moveTo(x, y)));
      if (pts.length === 1) g.lineTo(pts[0][0] + 0.1, pts[0][1]);
      g.stroke();
    } else if (it.t === 'rect') {
      g.strokeRect(it.x, it.y, it.w2, it.h2);
    } else if (it.t === 'ellipse') {
      g.beginPath();
      g.ellipse(it.x + it.w2 / 2, it.y + it.h2 / 2, Math.abs(it.w2 / 2), Math.abs(it.h2 / 2), 0, 0, Math.PI * 2);
      g.stroke();
    } else if (it.t === 'line' || it.t === 'arrow') {
      g.beginPath();
      g.moveTo(it.x, it.y);
      g.lineTo(it.x2, it.y2);
      g.stroke();
      if (it.t === 'arrow') {
        const a = Math.atan2(it.y2 - it.y, it.x2 - it.x), s = 10 + (it.w ?? 3) * 2;
        g.beginPath();
        g.moveTo(it.x2, it.y2);
        g.lineTo(it.x2 - s * Math.cos(a - 0.45), it.y2 - s * Math.sin(a - 0.45));
        g.lineTo(it.x2 - s * Math.cos(a + 0.45), it.y2 - s * Math.sin(a + 0.45));
        g.closePath();
        g.fill();
      }
    } else if (it.t === 'text') {
      g.font = `${it.s ?? 24}px ${getComputedStyle(document.body).fontFamily}`;
      g.textBaseline = 'top';
      String(it.text ?? '').split('\n').forEach((line, i) => g.fillText(line, it.x, it.y + i * (it.s ?? 24) * 1.25));
    } else if (it.t === 'note') {
      g.fillStyle = NOTES[it.c % NOTES.length] ?? NOTES[0];
      g.shadowColor = 'rgba(0,0,0,0.25)';
      g.shadowBlur = live ? 0 : 6;
      g.fillRect(it.x, it.y, 200, 150);
      g.shadowBlur = 0;
      g.fillStyle = COLORS[7];
      g.font = `16px ${getComputedStyle(document.body).fontFamily}`;
      g.textBaseline = 'top';
      wrap(g, String(it.text ?? ''), it.x + 12, it.y + 12, 176, 20, 6);
    }
    g.restore();
  }
  function wrap(g, text, x, y, width, line, max) {
    let row = 0;
    for (const para of text.split('\n')) {
      let current = '';
      for (const word of para.split(' ')) {
        const test = current ? `${current} ${word}` : word;
        if (g.measureText(test).width > width && current) {
          if (row >= max) return;
          g.fillText(current, x, y + row++ * line);
          current = word;
        } else current = test;
      }
      if (row >= max) return;
      g.fillText(current, x, y + row++ * line);
    }
  }
  function box(it) {
    if (it.t === 'pen' || it.t === 'hl') {
      const pts = decode(it.p);
      if (!pts.length) return null;
      let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
      for (const [x, y] of pts) { x0 = Math.min(x0, x); y0 = Math.min(y0, y); x1 = Math.max(x1, x); y1 = Math.max(y1, y); }
      const pad = (it.w ?? 3) * (it.t === 'hl' ? 2 : 1);
      return { x: x0 - pad, y: y0 - pad, w: x1 - x0 + 2 * pad, h: y1 - y0 + 2 * pad };
    }
    if (it.t === 'rect' || it.t === 'ellipse') return { x: Math.min(it.x, it.x + it.w2), y: Math.min(it.y, it.y + it.h2), w: Math.abs(it.w2), h: Math.abs(it.h2) };
    if (it.t === 'line' || it.t === 'arrow') return { x: Math.min(it.x, it.x2) - 6, y: Math.min(it.y, it.y2) - 6, w: Math.abs(it.x2 - it.x) + 12, h: Math.abs(it.y2 - it.y) + 12 };
    if (it.t === 'note') return { x: it.x, y: it.y, w: 200, h: 150 };
    if (it.t === 'text') {
      const lines = String(it.text ?? '').split('\n');
      return { x: it.x, y: it.y, w: Math.max(...lines.map((l) => l.length)) * (it.s ?? 24) * 0.6, h: lines.length * (it.s ?? 24) * 1.25 };
    }
    return null;
  }
  function bounds() {
    let b = null;
    for (const it of items.values()) {
      const r = box(it);
      if (!r) continue;
      b = b ? { x: Math.min(b.x, r.x), y: Math.min(b.y, r.y), x2: Math.max(b.x2, r.x + r.w), y2: Math.max(b.y2, r.y + r.h) } : { x: r.x, y: r.y, x2: r.x + r.w, y2: r.y + r.h };
    }
    return b && { x: b.x, y: b.y, w: b.x2 - b.x, h: b.y2 - b.y };
  }
  function hit(wx, wy, r) {
    const near = (pts, w) => pts.some(([x, y], i) => {
      if (i === 0) return Math.hypot(x - wx, y - wy) < r + w;
      const [px, py] = pts[i - 1];
      const dx = x - px, dy = y - py, len2 = dx * dx + dy * dy || 1;
      const t = Math.max(0, Math.min(1, ((wx - px) * dx + (wy - py) * dy) / len2));
      return Math.hypot(px + t * dx - wx, py + t * dy - wy) < r + w;
    });
    const out = [];
    for (const [k, it] of items) {
      let yes = false;
      if (it.t === 'pen' || it.t === 'hl') yes = near(decode(it.p), (it.w ?? 3) * (it.t === 'hl' ? 2 : 0.5));
      else if (it.t === 'line' || it.t === 'arrow') yes = near([[it.x, it.y], [it.x2, it.y2]], it.w ?? 3);
      else {
        const b = box(it);
        yes = !!b && wx >= b.x - r && wx <= b.x + b.w + r && wy >= b.y - r && wy <= b.y + b.h + r;
      }
      if (yes) out.push(k);
    }
    return out;
  }

  // ---- drawing: everything is drawn only when something changed --------------------
  let frame = 0;
  const live = new Map();    // pid -> { item, at }
  const cursors = new Map(); // pid -> { x, y }
  let draft = null;          // what I am drawing now
  const erasing = new Set(); // items under the eraser, hidden until it lifts
  function redraw() {
    if (frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      paintBase();
      paintOver();
    });
  }
  function overlay() {
    if (frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      paintOver();
    });
  }
  let baseDirty = true;
  function paintBase() {
    const g = base.getContext('2d');
    g.setTransform(1, 0, 0, 1, 0, 0);
    g.clearRect(0, 0, base.width, base.height);
    g.setTransform(view.z * dpr, 0, 0, view.z * dpr, view.x * dpr, view.y * dpr);
    for (const [k, it] of ordered()) if (!erasing.has(k)) drawItem(g, it);
    baseDirty = false;
  }
  function paintOver() {
    if (baseDirty) paintBase();
    const g = over.getContext('2d');
    g.setTransform(1, 0, 0, 1, 0, 0);
    g.clearRect(0, 0, over.width, over.height);
    g.setTransform(view.z * dpr, 0, 0, view.z * dpr, view.x * dpr, view.y * dpr);
    for (const { item } of live.values()) drawItem(g, item, true);
    if (draft) drawItem(g, draft, true);
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.font = `12px ${getComputedStyle(document.body).fontFamily}`;
    for (const [pid, c] of cursors) {
      const p = st.participant(pid);
      if (!p) continue;
      const sx = c.x * view.z + view.x, sy = c.y * view.z + view.y;
      g.fillStyle = CURSOR[pid % CURSOR.length];
      g.beginPath();
      g.moveTo(sx, sy); g.lineTo(sx + 12, sy + 4); g.lineTo(sx + 4, sy + 12); g.closePath(); g.fill();
      const w = g.measureText(p.name).width + 10;
      g.fillRect(sx + 12, sy + 12, w, 18);
      g.fillStyle = COLORS[7];
      g.fillText(p.name, sx + 17, sy + 25);
    }
  }

  // ---- shared state ---------------------------------------------------------------
  function load(entries, reset) {
    if (reset) items.clear();
    for (const e of entries) {
      if (!e.key.startsWith('i/')) continue;
      if (e.value == null) items.delete(e.key);
      else items.set(e.key, e.value);
      // A finished item replaces its author's preview.
      if (live.get(e.by)?.done) live.delete(e.by);
    }
    const used = st.entries('').reduce((n, e) => n + e.key.length + JSON.stringify(e.value).length, 0);
    meter.textContent = `Board ${Math.min(100, Math.round(used / 2621.44))}% full`;
    clearButton.hidden = !st.isHost();
    baseDirty = true;
    redraw();
  }
  st.on('state', ({ reset, entries }) => load(entries, reset));
  st.on('participants', () => { clearButton.hidden = !st.isHost(); overlay(); });
  load(st.entries('i/'), true);
  for (const k of items.keys()) {
    const m = new RegExp(`^i/${ctx0.you}-(\\d+)$`).exec(k);
    if (m) counter = Math.max(counter, Number(m[1]));
  }

  // Others' cursors and strokes in progress.
  st.on('message', ({ from, data }) => {
    if (!data || typeof data !== 'object') return;
    if (Array.isArray(data.cur)) cursors.set(from, { x: Number(data.cur[0]) || 0, y: Number(data.cur[1]) || 0 });
    if (data.gone) cursors.delete(from);
    if (data.live && typeof data.live === 'object') {
      const it = data.live;
      const l = live.get(from);
      if (it.p) {
        // A stroke arrives in pieces: each repeats the last point it continues from.
        const pts = decode(it.p);
        if (!l || l.key !== it.key || !it.from) live.set(from, { key: it.key, item: { t: it.t, c: it.c, w: it.w, pts } });
        else if (l.item.pts.length < 4000) l.item.pts.push(...pts.slice(1));
      } else {
        live.set(from, { key: it.key, item: it });
      }
    }
    if (data.done) {
      const l = live.get(from);
      if (l) l.done = true;
    }
    overlay();
  });

  // Ephemeral updates: at most one message every 66 ms, sent only while
  // something moves. A stroke in progress is sent as the points added since
  // the last message (a message is at most 2 KB).
  let outgoing = null, sendTimer = 0, liveDirty = false;
  function queue(patch) {
    if (patch) outgoing = { ...(outgoing ?? {}), ...patch };
    if (sendTimer) return;
    sendTimer = setTimeout(flush, 66);
  }
  function flush() {
    sendTimer = 0;
    const m = outgoing ?? {};
    outgoing = null;
    if (liveDirty && draft) {
      liveDirty = false;
      if (draft.pts) {
        const from = Math.max(0, (draft.sent ?? 0) - 1);
        const chunk = draft.pts.slice(from, from + 120);
        m.live = { key: draft.key, t: draft.t, c: draft.c, w: draft.w, from, p: encode(chunk) };
        draft.sent = from + chunk.length;
        if (draft.sent < draft.pts.length) { liveDirty = true; queue(); }
      } else {
        m.live = { key: draft.key, t: draft.t, c: draft.c, w: draft.w, x: draft.x, y: draft.y, w2: draft.w2, h2: draft.h2, x2: draft.x2, y2: draft.y2 };
      }
    }
    if (Object.keys(m).length) st.send(m).catch(() => {});
  }

  // ---- undo and redo (my own changes only) -------------------------------------------
  const undoStack = [], redoStack = [];
  function track(op) {
    undoStack.push(op);
    if (undoStack.length > 100) undoStack.shift();
    redoStack.length = 0;
    buttons();
  }
  function buttons() {
    undoButton.disabled = !undoStack.length;
    redoButton.disabled = !redoStack.length;
  }
  buttons();
  async function apply(ops) {
    try {
      await st.batch(ops);
      error.textContent = '';
      return true;
    } catch (e) {
      error.textContent = e.message;
      return false;
    }
  }
  async function undo() {
    const op = undoStack.pop();
    if (!op) return;
    // Undo: put back what was there before.
    await apply(op.map((o) => ({ key: o.key, value: o.before ?? null })));
    redoStack.push(op);
    buttons();
  }
  async function redo() {
    const op = redoStack.pop();
    if (!op) return;
    await apply(op.map((o) => ({ key: o.key, value: o.after ?? null })));
    undoStack.push(op);
    buttons();
  }
  async function commit(item) {
    const key = nextKey();
    item.z = st.serverNow();
    items.set(key, item);
    baseDirty = true;
    redraw();
    if (await apply([{ key, value: item }])) track([{ key, before: null, after: item }]);
    else { items.delete(key); baseDirty = true; redraw(); }
    queue({ done: true });
  }

  // ---- input ----------------------------------------------------------------------
  const pointers = new Map();
  let gesture = null;
  over.addEventListener('pointerdown', (e) => {
    over.setPointerCapture(e.pointerId);
    pointers.set(e.pointerId, [e.clientX, e.clientY]);
    const r = over.getBoundingClientRect();
    const [wx, wy] = toWorld(e.clientX - r.left, e.clientY - r.top);
    if (pointers.size === 2) {
      draft = null;
      const [a, b] = [...pointers.values()];
      gesture = { pinch: Math.hypot(a[0] - b[0], a[1] - b[1]), mid: [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2] };
      return overlay();
    }
    if (tool === 'hand' || e.button === 1) {
      gesture = { pan: [e.clientX - view.x, e.clientY - view.y] };
      return;
    }
    if (tool === 'eraser') {
      gesture = { erase: new Set() };
      eraseAt(wx, wy);
      return;
    }
    // The canvas would take focus from the text box on mouse down.
    if (tool === 'text' || tool === 'note') e.preventDefault();
    if (tool === 'text' || tool === 'note') return edit(null, { t: tool, x: Math.round(wx), y: Math.round(wy), c: tool === 'note' ? 0 : color, s: 24 });
    const w = tool === 'hl' ? 4 : 3;
    if (tool === 'pen' || tool === 'hl') draft = { t: tool, c: color, w, pts: [[wx, wy]] };
    else draft = { t: tool, c: color, w, x: Math.round(wx), y: Math.round(wy), w2: 0, h2: 0, x2: Math.round(wx), y2: Math.round(wy) };
    draft.key = `live-${me()}-${counter + 1}`;
    gesture = { draw: true, sent: 0 };
    overlay();
  });
  over.addEventListener('pointermove', (e) => {
    const r = over.getBoundingClientRect();
    const [wx, wy] = toWorld(e.clientX - r.left, e.clientY - r.top);
    queue({ cur: [Math.round(wx), Math.round(wy)] });
    if (!pointers.has(e.pointerId)) return;
    pointers.set(e.pointerId, [e.clientX, e.clientY]);
    if (gesture?.pinch && pointers.size === 2) {
      const [a, b] = [...pointers.values()];
      const d = Math.hypot(a[0] - b[0], a[1] - b[1]);
      const mid = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
      view.x += mid[0] - gesture.mid[0];
      view.y += mid[1] - gesture.mid[1];
      zoomAt(mid[0], mid[1], d / gesture.pinch);
      gesture = { pinch: d, mid };
      return;
    }
    if (gesture?.pan) {
      view.x = e.clientX - gesture.pan[0];
      view.y = e.clientY - gesture.pan[1];
      return redraw();
    }
    if (gesture?.erase) return eraseAt(wx, wy);
    if (!draft) return;
    if (draft.pts) {
      const last = draft.pts.at(-1);
      if (Math.hypot(last[0] - wx, last[1] - wy) * view.z < 1.5) return;
      if (draft.pts.length < 2000) draft.pts.push([wx, wy]);
    } else {
      draft.w2 = Math.round(wx) - draft.x;
      draft.h2 = Math.round(wy) - draft.y;
      draft.x2 = Math.round(wx);
      draft.y2 = Math.round(wy);
    }
    liveDirty = true;
    queue();
    overlay();
  });
  const end = (e) => {
    pointers.delete(e.pointerId);
    if (gesture?.erase) {
      const keys = [...gesture.erase];
      erasing.clear();
      baseDirty = true;
      redraw();
      if (keys.length) eraseCommit(keys);
    }
    if (pointers.size === 0) gesture = null;
    if (!draft) return;
    const d = draft;
    draft = null;
    let item;
    if (d.pts) {
      const pts = simplify(d.pts, 1.2 / view.z).slice(0, 1500);
      item = { t: d.t, c: d.c, w: d.w, p: encode(pts) };
    } else {
      if (Math.abs(d.w2) < 3 && Math.abs(d.h2) < 3) { queue({ done: true }); return overlay(); }
      item = { t: d.t, c: d.c, w: d.w, x: d.x, y: d.y, w2: d.w2, h2: d.h2, x2: d.x2, y2: d.y2 };
    }
    commit(item);
    overlay();
  };
  over.addEventListener('pointerup', end);
  over.addEventListener('pointercancel', end);
  over.addEventListener('pointerleave', () => queue({ gone: true }));
  over.addEventListener('wheel', (e) => {
    e.preventDefault();
    if (e.ctrlKey || e.metaKey || Math.abs(e.deltaY) > Math.abs(e.deltaX) * 2 && !e.shiftKey) zoomAt(e.clientX, e.clientY, Math.exp(-e.deltaY / 400));
    else { view.x -= e.deltaX; view.y -= e.deltaY; redraw(); }
  }, { passive: false });
  over.addEventListener('dblclick', (e) => {
    const r = over.getBoundingClientRect();
    const [wx, wy] = toWorld(e.clientX - r.left, e.clientY - r.top);
    const key = hit(wx, wy, 4).reverse().find((k) => ['note', 'text'].includes(items.get(k)?.t));
    if (key) edit(key, items.get(key));
  });
  function eraseAt(wx, wy) {
    let changed = false;
    for (const k of hit(wx, wy, 8 / view.z)) {
      if (gesture.erase.has(k) || gesture.erase.size >= 32) continue;
      gesture.erase.add(k);
      erasing.add(k);
      changed = true;
    }
    if (changed) { baseDirty = true; redraw(); }
  }
  async function eraseCommit(keys) {
    const ops = keys.filter((k) => items.has(k)).slice(0, 32);
    if (!ops.length) return;
    const before = ops.map((k) => ({ key: k, before: items.get(k), after: null }));
    if (await apply(ops.map((k) => ({ key: k, value: null })))) track(before);
  }
  let armed = 0;
  async function clearAll() {
    if (!st.isHost()) return;
    // Pressed twice: the first press asks (the sandbox has no dialogs).
    if (!armed) {
      clearButton.textContent = 'Clear for Everyone?';
      armed = setTimeout(() => { armed = 0; clearButton.textContent = 'Clear'; }, 4000);
      return;
    }
    clearTimeout(armed);
    armed = 0;
    clearButton.textContent = 'Clear';
    const keys = [...items.keys()];
    for (let i = 0; i < keys.length; i += 32) await apply(keys.slice(i, i + 32).map((key) => ({ key, value: null })));
    undoStack.length = redoStack.length = 0;
    buttons();
  }

  // Text and sticky notes are typed in place.
  function edit(key, it) {
    const sx = it.x * view.z + view.x, sy = it.y * view.z + view.y;
    editor.hidden = false;
    editor.value = it.text ?? '';
    editor.style.left = `${sx}px`;
    editor.style.top = `${sy}px`;
    editor.style.width = it.t === 'note' ? `${200 * view.z}px` : '260px';
    editor.style.height = it.t === 'note' ? `${150 * view.z}px` : '64px';
    editor.dataset.kind = it.t;
    requestAnimationFrame(() => editor.focus());
    const done = async (save) => {
      editor.onblur = editor.onkeydown = null;
      editor.hidden = true;
      const text = editor.value.trim().slice(0, 500);
      if (!save) return;
      if (!text && !key) return;
      if (key) {
        const before = items.get(key);
        const after = text ? { ...before, text } : null;
        if (await apply([{ key, value: after }])) track([{ key, before, after }]);
      } else commit({ ...it, text });
    };
    editor.onkeydown = (e) => {
      if (e.key === 'Escape') { e.preventDefault(); done(false); }
      else if (e.key === 'Enter' && (it.t === 'text' ? !e.shiftKey : e.ctrlKey || e.metaKey)) { e.preventDefault(); done(true); }
    };
    editor.onblur = () => done(true);
  }

  addEventListener('keydown', (e) => {
    if (e.target === editor) return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'z') { e.preventDefault(); e.shiftKey ? redo() : undo(); }
    else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'y') { e.preventDefault(); redo(); }
  });

  // ---- export ------------------------------------------------------------------------
  function exportPng() {
    const b = bounds() ?? { x: 0, y: 0, w: 800, h: 600 };
    const pad = 32, scale = Math.min(2, 4096 / Math.max(b.w + 2 * pad, b.h + 2 * pad));
    const c = document.createElement('canvas');
    c.width = Math.max(1, Math.round((b.w + 2 * pad) * scale));
    c.height = Math.max(1, Math.round((b.h + 2 * pad) * scale));
    const g = c.getContext('2d');
    g.fillStyle = getComputedStyle(document.documentElement).getPropertyValue('--bg') || COLORS[7];
    g.fillRect(0, 0, c.width, c.height);
    g.setTransform(scale, 0, 0, scale, (pad - b.x) * scale, (pad - b.y) * scale);
    for (const [, it] of ordered()) drawItem(g, it);
    st.download('whiteboard', c.toDataURL('image/png')).catch((e) => { error.textContent = e.message; });
  }

  resize();
  document.body.dataset.ready = 'true';
})();
