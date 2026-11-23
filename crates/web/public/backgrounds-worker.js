// Camera background effects, off the page's main thread. Loaded only while a
// background effect is on; the model and its WebAssembly are released with it.
//
// Person segmentation: MediaPipe Image Segmenter with the selfie segmenter
// (landscape) model, both Apache-2.0, vendored and served by StickTalk itself
// (public/vendor/mediapipe/, see its NOTICE). Nothing is fetched from anywhere
// else and no frame leaves this device.
const BASE = '/public/vendor/mediapipe';
let segmenter = null, delegate = '';
let background = { kind: 'none' }, backgroundFrame = null;
let canvas, ctx, person, pctx, maskCanvas, mctx, maskData, previous;
let frames = 0, stride = 1, scale = 1, segCost = 0, totalCost = 0, lastTimestamp = 0, frameGap = 33;
const ewma = (old, v) => (old ? old * 0.9 + v * 0.1 : v);

async function init() {
  const { FilesetResolver, ImageSegmenter } = await import(`${BASE}/vision.js`);
  const fileset = await FilesetResolver.forVisionTasks(BASE, true);
  let failure;
  for (const d of ['GPU', 'CPU']) {
    try {
      segmenter = await ImageSegmenter.createFromOptions(fileset, {
        baseOptions: { modelAssetPath: `${BASE}/selfie_segmenter_landscape.tflite`, delegate: d },
        runningMode: 'VIDEO',
        outputCategoryMask: false,
        outputConfidenceMasks: true,
      });
      delegate = d;
      return;
    } catch (e) {
      failure = e;
    }
  }
  throw failure ?? new Error('Background effects are unavailable on this device.');
}

function surfaces(w, h) {
  if (canvas?.width === w && canvas?.height === h) return;
  canvas = new OffscreenCanvas(w, h);
  ctx = canvas.getContext('2d', { alpha: false, desynchronized: true });
  person = new OffscreenCanvas(w, h);
  pctx = person.getContext('2d');
}
// The confidence mask becomes the alpha of a small canvas, smoothed over time
// so the edge does not shimmer, then scaled up (bilinear) to soften it.
function updateMask(mask) {
  const w = mask.width, h = mask.height, values = mask.getAsFloat32Array();
  if (!maskCanvas || maskCanvas.width !== w || maskCanvas.height !== h) {
    maskCanvas = new OffscreenCanvas(w, h);
    mctx = maskCanvas.getContext('2d');
    maskData = new ImageData(w, h);
    previous = new Float32Array(w * h);
  }
  const data = maskData.data;
  for (let i = 0; i < values.length; i++) {
    const v = previous[i] * 0.35 + values[i] * 0.65;
    previous[i] = v;
    // A gentle curve: confident pixels solid, the edge soft.
    const a = v < 0.25 ? 0 : v > 0.75 ? 255 : ((v - 0.25) * 510) | 0;
    data[i * 4 + 3] = a;
  }
  mctx.putImageData(maskData, 0, 0);
}
function cover(source, sw, sh, w, h) {
  const s = Math.max(w / sw, h / sh), dw = sw * s, dh = sh * s;
  ctx.drawImage(source, (w - dw) / 2, (h - dh) / 2, dw, dh);
}
function compose(source, sw, sh, timestamp) {
  const w = Math.max(2, Math.round((sw * scale) / 2) * 2), h = Math.max(2, Math.round((sh * scale) / 2) * 2);
  surfaces(w, h);
  const t0 = performance.now();
  if (frames++ % stride === 0) {
    const result = segmenter.segmentForVideo(source, timestamp);
    const mask = result.confidenceMasks?.[0];
    if (mask) updateMask(mask);
    result.close?.();
    segCost = ewma(segCost, performance.now() - t0);
  }
  // The person, cut out by the mask.
  pctx.globalCompositeOperation = 'copy';
  pctx.filter = `blur(${Math.max(1, Math.round(h / 240))}px)`;
  if (maskCanvas) pctx.drawImage(maskCanvas, 0, 0, w, h);
  pctx.filter = 'none';
  pctx.globalCompositeOperation = 'source-in';
  pctx.drawImage(source, 0, 0, w, h);
  pctx.globalCompositeOperation = 'source-over';
  // What is behind them.
  if (background.kind === 'blur') {
    const r = Math.round(h / (background.strength === 'strong' ? 30 : 80));
    ctx.filter = `blur(${r}px)`;
    ctx.drawImage(source, -r * 2, -r * 2, w + r * 4, h + r * 4);
    ctx.filter = 'none';
  } else if (backgroundFrame) {
    cover(backgroundFrame, backgroundFrame.width, backgroundFrame.height, w, h);
  } else {
    ctx.drawImage(source, 0, 0, w, h);
  }
  ctx.drawImage(person, 0, 0);
  totalCost = ewma(totalCost, performance.now() - t0);
  adapt(timestamp);
}
// Hold the camera's frame rate: segment every second or third frame (reusing
// the mask) and, last, compose at a lower resolution. Recover when there is room.
let lastAdapt = 0;
function adapt(timestamp) {
  if (lastTimestamp) frameGap = ewma(frameGap, Math.min(200, Math.max(5, timestamp - lastTimestamp)));
  lastTimestamp = timestamp;
  if (frames - lastAdapt < 30) return;
  const budget = frameGap * 0.8, before = `${stride}:${scale}`;
  if (totalCost > budget) {
    if (stride < 3) stride++;
    else if (scale > 0.5) scale = Math.max(0.5, scale - 0.25);
  } else if (totalCost < budget * 0.45) {
    if (scale < 1) scale = Math.min(1, scale + 0.25);
    else if (stride > 1) stride--;
  }
  if (`${stride}:${scale}` !== before) {
    lastAdapt = frames;
    self.postMessage({ type: 'load', stride, scale, ms: Math.round(totalCost * 10) / 10 });
  }
}
function safeTimestamp(ms) {
  // MediaPipe requires strictly increasing timestamps.
  const t = Math.max(ms, (safeTimestamp.last ?? 0) + 1);
  safeTimestamp.last = t;
  return t;
}

async function pipe(readable, writable) {
  const transform = new TransformStream({
    transform(frame, controller) {
      try {
        compose(frame, frame.displayWidth, frame.displayHeight, safeTimestamp(frame.timestamp / 1000));
        controller.enqueue(new VideoFrame(canvas, { timestamp: frame.timestamp, duration: frame.duration ?? undefined }));
      } catch (e) {
        self.postMessage({ type: 'error', message: String(e?.message ?? e) });
      } finally {
        frame.close();
      }
    },
  });
  await readable.pipeThrough(transform).pipeTo(writable).catch(() => {});
}

self.onmessage = async (e) => {
  const m = e.data;
  try {
    switch (m.type) {
      case 'init':
        await init();
        self.postMessage({ type: 'ready', delegate });
        break;
      case 'background':
        background = m.background;
        backgroundFrame?.close?.();
        backgroundFrame = m.bitmap ?? null;
        break;
      case 'background-frame':
        backgroundFrame?.close?.();
        backgroundFrame = m.bitmap;
        break;
      case 'stream':
        pipe(m.readable, m.writable);
        break;
      case 'frame': {
        compose(m.bitmap, m.bitmap.width, m.bitmap.height, safeTimestamp(m.timestamp));
        m.bitmap.close();
        const out = canvas.transferToImageBitmap();
        self.postMessage({ type: 'frame', bitmap: out }, [out]);
        break;
      }
      case 'stop':
        segmenter?.close();
        segmenter = null;
        backgroundFrame?.close?.();
        self.close();
        break;
    }
  } catch (err) {
    self.postMessage({ type: 'error', message: String(err?.message ?? err) });
  }
};
