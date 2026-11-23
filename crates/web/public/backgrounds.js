// Camera backgrounds: none, two strengths of blur, StickTalk's own scenes, and
// up to ten pictures or short videos of your own, stored on your server.
// Loaded only when a background is chosen; the segmentation model runs in a
// worker (backgrounds-worker.js) on this device, and is released as soon as the
// effect is turned off or the camera stops.
let host = null, tools = null;
export function bind(h, t) {
  host = h;
  tools = t;
}
const element = (tag, props = {}, ...children) => {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...children);
  return e;
};
// A helper video is in the page (invisible) so every browser delivers its frames.
function offscreen(video) {
  video.className = 'call-hidden-media';
  video.setAttribute('aria-hidden', 'true');
  document.body.append(video);
  return video;
}

// ---- StickTalk's own scenes, drawn here: no image files, no third-party art ----
function seeded(seed) {
  let s = seed >>> 0;
  return () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
}
function gradient(ctx, w, h, stops, vertical = true) {
  const g = vertical ? ctx.createLinearGradient(0, 0, 0, h) : ctx.createLinearGradient(0, 0, w, 0);
  stops.forEach(([at, color]) => g.addColorStop(at, color));
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, w, h);
}
function hills(ctx, w, h, base, amplitude, color, phase) {
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.moveTo(0, h);
  for (let x = 0; x <= w; x += w / 48) ctx.lineTo(x, base + Math.sin(x / w * Math.PI * 2 + phase) * amplitude + Math.sin(x / w * 7 + phase * 3) * amplitude * 0.3);
  ctx.lineTo(w, h);
  ctx.fill();
}
export const SCENES = {
  dusk: ['Dusk', (ctx, w, h) => {
    gradient(ctx, w, h, [[0, '#2b1a4a'], [0.55, '#c0566b'], [1, '#f2a65a']]);
    ctx.fillStyle = 'rgba(255,214,150,0.9)';
    ctx.beginPath();
    ctx.arc(w * 0.7, h * 0.58, h * 0.12, 0, Math.PI * 2);
    ctx.fill();
    hills(ctx, w, h, h * 0.72, h * 0.04, '#5a2f52', 0.6);
    hills(ctx, w, h, h * 0.82, h * 0.05, '#341d3d', 2.1);
  }],
  ocean: ['Ocean', (ctx, w, h) => {
    gradient(ctx, w, h, [[0, '#9fd3e6'], [0.45, '#5aa6c8'], [1, '#14466b']]);
    for (let i = 0; i < 7; i++) {
      ctx.strokeStyle = `rgba(255,255,255,${0.08 + i * 0.02})`;
      ctx.lineWidth = h * 0.006;
      ctx.beginPath();
      const y = h * (0.5 + i * 0.07);
      for (let x = 0; x <= w; x += w / 60) ctx.lineTo(x, y + Math.sin(x / w * 12 + i) * h * 0.01);
      ctx.stroke();
    }
  }],
  aurora: ['Aurora', (ctx, w, h) => {
    gradient(ctx, w, h, [[0, '#050b1a'], [1, '#0d1f2e']]);
    const r = seeded(7);
    ctx.fillStyle = 'rgba(255,255,255,0.8)';
    for (let i = 0; i < 140; i++) ctx.fillRect(r() * w, r() * h * 0.6, 1.5, 1.5);
    for (const [color, y, phase] of [['rgba(61,220,151,0.35)', 0.3, 0], ['rgba(60,180,220,0.25)', 0.38, 1.7], ['rgba(140,90,220,0.2)', 0.25, 3.1]]) {
      ctx.strokeStyle = color;
      ctx.lineWidth = h * 0.09;
      ctx.lineCap = 'round';
      ctx.filter = `blur(${Math.round(h / 40)}px)`;
      ctx.beginPath();
      for (let x = -w * 0.1; x <= w * 1.1; x += w / 40) ctx.lineTo(x, h * y + Math.sin(x / w * 4 + phase) * h * 0.08);
      ctx.stroke();
      ctx.filter = 'none';
    }
    hills(ctx, w, h, h * 0.86, h * 0.03, '#03070d', 1);
  }],
  studio: ['Studio', (ctx, w, h) => {
    const g = ctx.createRadialGradient(w * 0.5, h * 0.4, h * 0.1, w * 0.5, h * 0.5, w * 0.7);
    g.addColorStop(0, '#5b5d66');
    g.addColorStop(1, '#1c1d21');
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, w, h);
  }],
  forest: ['Forest', (ctx, w, h) => {
    gradient(ctx, w, h, [[0, '#cfe3d4'], [1, '#6f9c7f']]);
    const r = seeded(11);
    for (const [shade, count, size] of [['#4f7a5f', 18, 0.28], ['#2f5a43', 14, 0.38], ['#1a3b2b', 10, 0.5]]) {
      ctx.fillStyle = shade;
      for (let i = 0; i < count; i++) {
        const x = r() * w, tall = h * size * (0.8 + r() * 0.4);
        ctx.beginPath();
        ctx.moveTo(x, h - tall);
        ctx.lineTo(x - tall * 0.25, h);
        ctx.lineTo(x + tall * 0.25, h);
        ctx.fill();
      }
    }
  }],
  grid: ['Night Grid', (ctx, w, h) => {
    gradient(ctx, w, h, [[0, '#120726'], [0.55, '#2a0f4d'], [0.56, '#09030f'], [1, '#09030f']]);
    ctx.strokeStyle = 'rgba(61,220,151,0.55)';
    ctx.lineWidth = Math.max(1, h / 360);
    const horizon = h * 0.56;
    for (let i = -12; i <= 12; i++) {
      ctx.beginPath();
      ctx.moveTo(w / 2 + i * w * 0.02, horizon);
      ctx.lineTo(w / 2 + i * w * 0.16, h);
      ctx.stroke();
    }
    for (let i = 1; i < 10; i++) {
      const y = horizon + (h - horizon) * (i / 10) ** 2;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(w, y);
      ctx.stroke();
    }
  }],
  paper: ['Paper', (ctx, w, h) => {
    gradient(ctx, w, h, [[0, '#efe6d6'], [1, '#d9ccb4']], false);
    const r = seeded(3);
    for (let i = 0; i < 2500; i++) {
      ctx.fillStyle = `rgba(90,70,40,${r() * 0.05})`;
      ctx.fillRect(r() * w, r() * h, 2, 2);
    }
  }],
};
function sceneCanvas(id, w, h) {
  const c = document.createElement('canvas');
  c.width = w;
  c.height = h;
  SCENES[id][1](c.getContext('2d'), w, h);
  return c;
}

// ---- the processing pipeline -----------------------------------------------------
let pipeline = null, notice = '';
async function backgroundMessage(choice) {
  if (choice === 'blur-light' || choice === 'blur-strong')
    return { background: { kind: 'blur', strength: choice === 'blur-strong' ? 'strong' : 'light' } };
  if (choice.startsWith('scene:') && SCENES[choice.slice(6)])
    return { background: { kind: 'image' }, bitmap: await createImageBitmap(sceneCanvas(choice.slice(6), 1280, 720)) };
  if (choice.startsWith('custom:')) {
    const id = choice.slice(7), item = (await customList()).find((b) => b.id === id);
    if (!item) throw new Error('That background was deleted.');
    if (item.kind === 'video') return { background: { kind: 'video' }, video: `/api/v1/users/@me/camera-backgrounds/${id}` };
    const blob = await (await fetch(`/api/v1/users/@me/camera-backgrounds/${id}`, { credentials: 'same-origin', cache: 'no-store' })).blob();
    // Large pictures are scaled to what a camera frame needs.
    const bitmap = await createImageBitmap(blob);
    const s = Math.min(1, 1920 / bitmap.width, 1080 / bitmap.height);
    if (s >= 1) return { background: { kind: 'image' }, bitmap };
    const scaled = await createImageBitmap(bitmap, { resizeWidth: Math.round(bitmap.width * s), resizeHeight: Math.round(bitmap.height * s), resizeQuality: 'high' });
    bitmap.close();
    return { background: { kind: 'image' }, bitmap: scaled };
  }
  return { background: { kind: 'none' } };
}
async function start(raw, choice) {
  const track = raw.getVideoTracks()[0];
  if (!track) throw new Error('No camera picture.');
  if (!globalThis.OffscreenCanvas || !globalThis.Worker) throw new Error('This browser cannot run background effects.');
  const worker = new Worker(new URL('./backgrounds-worker.js', import.meta.url), { type: 'module', name: 'camera-background' });
  const p = { raw, worker, stopped: false, choice, video: null, stride: 1, scale: 1 };
  const ready = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Background effects took too long to start.')), 20000);
    worker.onmessage = (e) => {
      const m = e.data;
      if (m.type === 'ready') { clearTimeout(timer); p.delegate = m.delegate; resolve(); }
      else if (m.type === 'error') { clearTimeout(timer); reject(new Error(m.message)); }
      else if (m.type === 'load') loadChanged(p, m);
      else if (m.type === 'frame') p.onFrame?.(m.bitmap);
    };
    worker.onerror = () => { clearTimeout(timer); reject(new Error('Background effects could not start.')); };
  });
  worker.postMessage({ type: 'init' });
  try {
    await ready;
    await setChoice(p, choice);
  } catch (e) {
    worker.terminate();
    throw e;
  }
  const fps = track.getSettings().frameRate || 30;
  if (globalThis.MediaStreamTrackProcessor && globalThis.MediaStreamTrackGenerator) {
    // Chromium (and the Windows app): frames go straight to the worker and back.
    const processor = new MediaStreamTrackProcessor({ track, maxBufferSize: 2 });
    const generator = new MediaStreamTrackGenerator({ kind: 'video' });
    worker.postMessage({ type: 'stream', readable: processor.readable, writable: generator.writable }, [processor.readable, generator.writable]);
    p.output = new MediaStream([generator]);
    p.generator = generator;
  } else {
    // Elsewhere: each camera frame as a bitmap, one in flight at a time.
    const video = offscreen(element('video', { muted: true, playsInline: true }));
    video.srcObject = new MediaStream([track]);
    await video.play();
    const canvas = document.createElement('canvas');
    const out = canvas.getContext('bitmaprenderer');
    let busy = false;
    p.onFrame = (bitmap) => {
      if (canvas.width !== bitmap.width) canvas.width = bitmap.width;
      if (canvas.height !== bitmap.height) canvas.height = bitmap.height;
      out.transferFromImageBitmap(bitmap);
      busy = false;
    };
    const pump = () => {
      if (p.stopped) return;
      if (!busy && video.videoWidth) {
        busy = true;
        createImageBitmap(video).then((bitmap) => {
          if (p.stopped) return bitmap.close();
          worker.postMessage({ type: 'frame', bitmap, timestamp: performance.now() }, [bitmap]);
        }).catch(() => { busy = false; });
      }
      video.requestVideoFrameCallback(pump);
    };
    video.requestVideoFrameCallback(pump);
    p.fallback = video;
    p.output = canvas.captureStream(fps);
  }
  return p;
}
async function setChoice(p, choice) {
  stopVideo(p);
  const m = await backgroundMessage(choice);
  p.choice = choice;
  if (m.video) {
    // A video background plays muted on this page; its frames go to the worker.
    const video = offscreen(element('video', { muted: true, loop: true, playsInline: true, src: m.video }));
    await video.play();
    p.video = video;
    const pump = () => {
      if (p.stopped || p.video !== video) return;
      createImageBitmap(video).then((bitmap) => {
        if (p.stopped || p.video !== video) return bitmap.close();
        p.worker.postMessage({ type: 'background-frame', bitmap }, [bitmap]);
      }).catch(() => {});
      video.requestVideoFrameCallback(pump);
    };
    p.worker.postMessage({ type: 'background', background: { kind: 'image' } });
    video.requestVideoFrameCallback(pump);
  } else {
    p.worker.postMessage({ type: 'background', ...m }, m.bitmap ? [m.bitmap] : []);
  }
}
function stopVideo(p) {
  if (!p.video) return;
  p.video.pause();
  p.video.removeAttribute('src');
  p.video.load();
  p.video.remove();
  p.video = null;
}
function loadChanged(p, m) {
  p.stride = m.stride;
  p.scale = m.scale;
  const busy = m.stride > 1 || m.scale < 1;
  const next = !busy ? ''
    : m.scale < 1 ? 'Your device is busy, so the background effect runs at a lower resolution to keep your camera smooth.'
    : 'Your device is busy, so the background follows you a little more slowly to keep your camera smooth.';
  if (next && next !== notice) tools.toast(next, '✨', 6000);
  notice = next;
  const status = document.querySelector('.backgrounds-dialog .backgrounds-status');
  if (status) status.textContent = notice;
}
function stopPipeline(p, { keepCamera }) {
  if (!p || p.stopped) return;
  p.stopped = true;
  stopVideo(p);
  p.worker.postMessage({ type: 'stop' });
  // The worker closes itself after releasing the model; make sure it is gone.
  setTimeout(() => p.worker.terminate(), 1000);
  p.generator?.stop();
  p.output?.getTracks().forEach((t) => t.stop());
  if (p.fallback) {
    p.fallback.srcObject = null;
    p.fallback.remove();
  }
  if (!keepCamera) p.raw.getTracks().forEach((t) => t.stop());
}
/** The camera's processed stream for the call. */
export async function process(raw) {
  release();
  pipeline = await start(raw, tools.prefs().background);
  return pipeline.output;
}
/** The camera stopped (or the call ended): the model and the camera go. */
export function release() {
  stopPipeline(pipeline, { keepCamera: false });
  pipeline = null;
  notice = '';
}
/** A different background for the camera that is on now. */
async function apply(choice) {
  tools.savePrefs({ background: choice });
  const camera = host?.publication('camera');
  if (!camera) return;
  if (choice === 'none') {
    if (!pipeline) return;
    const raw = pipeline.raw;
    stopPipeline(pipeline, { keepCamera: true });
    pipeline = null;
    await host.replaceCamera(raw);
  } else if (pipeline) {
    await setChoice(pipeline, choice);
  } else {
    pipeline = await start(camera.stream, choice);
    await host.replaceCamera(pipeline.output);
  }
}

// ---- custom backgrounds ------------------------------------------------------------
let customCache = null;
async function customList(refresh = false) {
  if (!customCache || refresh) customCache = await tools.api('/api/v1/users/@me/camera-backgrounds');
  return customCache;
}

// ---- the Video Effects dialog ------------------------------------------------------
export async function open() {
  if (document.querySelector('.backgrounds-dialog')) return;
  const d = tools.dialog('Video Effects', { className: 'backgrounds-dialog' });
  const preview = element('video', { className: 'backgrounds-preview', muted: true, playsInline: true, autoplay: true });
  preview.setAttribute('aria-label', 'Camera preview');
  const status = element('p', { className: 'muted-copy backgrounds-status', textContent: notice });
  status.setAttribute('role', 'status');
  const grid = element('div', { className: 'backgrounds-grid' });
  grid.setAttribute('role', 'radiogroup');
  grid.setAttribute('aria-label', 'Background');
  const upload = element('input', { type: 'file', accept: 'image/png,image/jpeg,image/webp,image/gif,video/mp4,video/webm', className: 'sr-only' });
  upload.setAttribute('aria-label', 'Upload a background');
  d.append(preview, status, grid, upload, tools.button('Done', 'button primary', () => d.close()));
  // Outside a call (or with the camera off), a preview camera of its own.
  let own = null, ownPipeline = null;
  const showPreview = async () => {
    const camera = host?.publication('camera');
    if (camera) {
      preview.srcObject = camera.stream;
      return;
    }
    try {
      own ??= await navigator.mediaDevices.getUserMedia({ video: { ...(await import('./callextras.js')).cameraConstraints(), ...(host?.cameraDevice ? { deviceId: { ideal: host.cameraDevice } } : {}) } });
      stopPipeline(ownPipeline, { keepCamera: true });
      ownPipeline = null;
      const choice = tools.prefs().background;
      if (choice !== 'none') ownPipeline = await start(own, choice);
      preview.srcObject = ownPipeline?.output ?? own;
    } catch (e) {
      status.textContent = e.name === 'NotAllowedError' ? 'Camera access was denied.' : e.message;
    }
  };
  d.addEventListener('close', () => {
    stopPipeline(ownPipeline, { keepCamera: true });
    own?.getTracks().forEach((t) => t.stop());
    preview.srcObject = null;
  });
  const choose = async (id) => {
    status.textContent = id === 'none' ? '' : 'Starting the effect…';
    try {
      if (host?.publication('camera')) await apply(id);
      else tools.savePrefs({ background: id });
      await showPreview();
      status.textContent = notice;
    } catch (e) {
      status.textContent = `That effect could not start: ${e.message}`;
      tools.savePrefs({ background: 'none' });
    }
    render();
  };
  const option = (id, label, paint) => {
    const b = element('button', { type: 'button', className: 'backgrounds-option' });
    b.setAttribute('role', 'radio');
    b.setAttribute('aria-checked', String(tools.prefs().background === id));
    b.setAttribute('aria-label', label);
    b.title = label;
    b.dataset.background = id;
    paint(b);
    b.onclick = () => choose(id);
    return b;
  };
  const label = (text) => (b) => b.append(element('span', { textContent: text }));
  const render = async () => {
    grid.replaceChildren(
      option('none', 'No background effect', label('None')),
      option('blur-light', 'Light blur', label('Blur')),
      option('blur-strong', 'Strong blur', label('Strong Blur')),
      ...Object.entries(SCENES).map(([id, [name]]) => option(`scene:${id}`, name, (b) => {
        const c = sceneCanvas(id, 160, 90);
        c.setAttribute('aria-hidden', 'true');
        b.append(c);
      })),
    );
    let custom = [];
    try { custom = await customList(); } catch {}
    for (const item of custom) {
      const b = option(`custom:${item.id}`, item.kind === 'video' ? 'Your video background' : 'Your background', (el) => {
        el.append(item.kind === 'video'
          ? element('span', { textContent: '▶ Video' })
          : element('img', { src: `/api/v1/users/@me/camera-backgrounds/${item.id}`, alt: '' }));
      });
      const remove = element('button', { type: 'button', className: 'backgrounds-remove icon-button', textContent: '×' });
      remove.setAttribute('aria-label', 'Delete this background');
      remove.onclick = async (e) => {
        e.stopPropagation();
        try {
          await tools.api(`/api/v1/users/@me/camera-backgrounds/${item.id}`, { method: 'DELETE' });
          if (tools.prefs().background === `custom:${item.id}`) await choose('none');
          await customList(true);
          render();
        } catch (err) { status.textContent = err.message; }
      };
      grid.append(element('div', { className: 'backgrounds-custom' }, b, remove));
    }
    if (custom.length < 10) {
      const add = element('button', { type: 'button', className: 'backgrounds-option backgrounds-add' }, element('span', { textContent: '+ Upload' }));
      add.setAttribute('aria-label', 'Upload a picture or short video');
      add.onclick = () => upload.click();
      grid.append(add);
    }
  };
  upload.onchange = async () => {
    const f = upload.files[0];
    upload.value = '';
    if (!f) return;
    if (f.size > 10 * 1024 * 1024) { status.textContent = 'Backgrounds can be at most 10 MB.'; return; }
    status.textContent = 'Uploading…';
    try {
      const item = await tools.api('/api/v1/users/@me/camera-backgrounds', { method: 'POST', body: f });
      await customList(true);
      await choose(`custom:${item.id}`);
    } catch (e) {
      status.textContent = e.message;
    }
  };
  await render();
  d.showModal();
  showPreview();
}
export function diagnostics() {
  return pipeline ? { active: true, choice: pipeline.choice, delegate: pipeline.delegate, stride: pipeline.stride, scale: pipeline.scale } : { active: false };
}
