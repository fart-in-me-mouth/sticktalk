// Deterministic CSR packager, using the same split/finalize pipeline as Trunk.
// Avoids Trunk's global tool-cache lock on concurrent Windows worktrees.
import {readFile,writeFile,mkdir,cp} from 'node:fs/promises';
import {join,resolve} from 'node:path';
import {createHash} from 'node:crypto';
import {hostGraphFingerprint} from './version-runtime.mjs';
import {buildStyles,copyUiSdk} from './styles.mjs';
import {copySegmentation} from './vendor.mjs';
const target=resolve(process.env.CARGO_TARGET_DIR), dist=resolve(process.env.TRUNK_STAGING_DIR);
await mkdir(dist,{recursive:true});
const wasm=await readFile(join(target,'wasm32-unknown-unknown/web-release/sticktalk-web.wasm'));
const hash=bytes=>createHash('sha256').update(bytes).digest('hex').slice(0,20);
const entry='sticktalk-web-'+hash(wasm), css=await buildStyles(target), style='app-'+hash(css)+'.css';
// The generated host glue changes when JS changes even if WASM does not. Give
// that graph a new immutable URL too, rather than reusing an HTTP-cached loader.
const graph=await hostGraphFingerprint(wasm,'public');
const pkg=`pkg/${graph}`;
await cp('public',join(dist,'public'),{recursive:true});
await copyUiSdk(join(dist,'public'));
await copySegmentation(join(dist,'public'));
await writeFile(join(dist,style),css);
let html=await readFile('index.html','utf8');
html=html.replace(/<link data-trunk rel="rust"[^>]*>/,`<script type="module">import init, * as bindings from './${pkg}/${entry}.js'; await init({module_or_path:'./${pkg}/${entry}_bg.wasm'});</script>`)
  .replace(/<link data-trunk rel="tailwind-css"[^>]*>/,`<link rel="stylesheet" href="/${style}">`)
  .replace(/<link data-trunk[^>]*>/g,'');
await writeFile(join(dist,'index.html'),html);
await import('./finalize.mjs');
