# Vendored camera-background segmentation

Everything here is served by StickTalk itself from `/public/vendor/mediapipe/`.
Nothing is fetched from a CDN or any third party at runtime.

| File | Source | License |
|---|---|---|
| `selfie_segmenter_landscape.tflite` (144x256, float16) | MediaPipe Selfie Segmenter (landscape), `storage.googleapis.com/mediapipe-models/image_segmenter/selfie_segmenter_landscape/float16/latest/`, SHA-256 `490e9ea734313e0de10fa0cd9e3c6133e36ea4db2b7a49bde9ef019f72796b8e` | Apache License 2.0 (MediaPipe model card) |
| `vision.js`, `vision_wasm_module_internal.js`, `vision_wasm_module_internal.wasm` | npm `@mediapipe/tasks-vision@1.2.3` (pinned in `package.json`; copied at build time by `scripts/vendor.mjs`) | Apache License 2.0 |

Apache License 2.0: https://www.apache.org/licenses/LICENSE-2.0 (the full text
is copied next to the files as `LICENSE` at build time).

The built-in background scenes are drawn by `public/backgrounds.js` at runtime;
they are original to StickTalk and use no image files.
