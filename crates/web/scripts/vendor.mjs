// Third-party runtime files StickTalk serves itself (never from a CDN):
// MediaPipe Tasks Vision for camera-background segmentation, loaded only when a
// background effect is turned on. See assets/models/NOTICE.md for licenses.
import {readFile,writeFile,mkdir,copyFile} from 'node:fs/promises';
import {join,resolve} from 'node:path';

const APACHE = `Apache License, Version 2.0 — https://www.apache.org/licenses/LICENSE-2.0

MediaPipe Tasks Vision (@mediapipe/tasks-vision) and the MediaPipe Selfie
Segmenter model are Copyright Google LLC and licensed under the Apache License,
Version 2.0. You may not use these files except in compliance with the License.
Unless required by applicable law or agreed to in writing, software distributed
under the License is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR
CONDITIONS OF ANY KIND, either express or implied.
`;

/** Copies the segmentation runtime and model into <public>/vendor/mediapipe/. */
export async function copySegmentation(publicDir) {
  const pkg=resolve('node_modules/@mediapipe/tasks-vision');
  const {version,license}=JSON.parse(await readFile(join(pkg,'package.json'),'utf8').catch(()=>{
    throw new Error('@mediapipe/tasks-vision is not installed. Run "npm ci --ignore-scripts" in crates/web first.');
  }));
  if(license!=='Apache-2.0')throw new Error(`Unexpected MediaPipe license: ${license}`);
  const to=join(publicDir,'vendor','mediapipe');
  await mkdir(to,{recursive:true});
  // The ES module build: it loads the module variant of the WASM glue.
  await copyFile(join(pkg,'vision_bundle.mjs'),join(to,'vision.js'));
  for(const file of ['vision_wasm_module_internal.js','vision_wasm_module_internal.wasm'])
    await copyFile(join(pkg,'wasm',file),join(to,file));
  await copyFile(resolve('assets/models/selfie_segmenter_landscape.tflite'),join(to,'selfie_segmenter_landscape.tflite'));
  await copyFile(resolve('assets/models/NOTICE.md'),join(to,'NOTICE.md'));
  await writeFile(join(to,'LICENSE'),APACHE);
  // The source map reference points at a file that is not shipped.
  const bundle=join(to,'vision.js');
  await writeFile(bundle,(await readFile(bundle,'utf8')).replace(/\n\/\/# sourceMappingURL=.*\s*$/,'\n'));
  return {version};
}
