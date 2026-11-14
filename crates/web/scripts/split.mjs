// Trunk post-build: split the raw linked module BEFORE wasm-bindgen and -Oz.
// Retain Trunk's content-addressed entry names; chunk names are content hashes.
import { readdir, readFile, writeFile, rm, mkdir, cp } from 'node:fs/promises';
import { join, resolve, basename, dirname } from 'node:path';
import { execFileSync } from 'node:child_process';
import {hostGraphFingerprint} from './version-runtime.mjs';
const target = resolve(process.env.CARGO_TARGET_DIR);
const dist = resolve(process.env.TRUNK_STAGING_DIR);
const run = (tool, args) => execFileSync(tool, args, { stdio: 'inherit', windowsHide: true, env: {...process.env, BINARYEN_CORES: '2'} });
async function find(dir, name) {
  for (const e of await readdir(dir, {withFileTypes:true}).catch(()=>[])) {
    const path = join(dir,e.name);
    if (e.isFile() && e.name === name) return path;
    if (e.isDirectory()) { const found=await find(path,name); if(found)return found; }
  }
}
const exe = process.platform === 'win32' ? '.exe' : '';
const split = process.env.WASM_SPLIT ?? join(target,'tools','bin',`wasm-split-cli${exe}`);
const cache = join(process.env.LOCALAPPDATA ?? '', 'trunkrs', 'trunk', 'cache');
const bindgen = process.env.WASM_BINDGEN ?? await find(cache, `wasm-bindgen${exe}`);
const opt = process.env.WASM_OPT ?? await find(cache, `wasm-opt${exe}`);
if (!bindgen || !opt) throw Error('Set WASM_BINDGEN and WASM_OPT to the matching Trunk tools.');
let html = await readFile(join(dist,'index.html'),'utf8');
const entryPath = html.match(/import init, \* as bindings from ['"]\.\/([^'"]+)\.js['"]/)[1];
const entry = basename(entryPath);
const folder = dirname(entryPath)==='.'
  ? `pkg/${await hostGraphFingerprint(await readFile(join(target,'wasm32-unknown-unknown','web-release','sticktalk-web.wasm')),'public')}`
  : dirname(entryPath);
if (dirname(entryPath)==='.') {
  html=html.replaceAll(`./${entry}.js`,`./${folder}/${entry}.js`).replaceAll(`./${entry}_bg.wasm`,`./${folder}/${entry}_bg.wasm`);
  await writeFile(join(dist,'index.html'),html);
  await rm(join(dist,`${entry}.js`),{force:true});
  await rm(join(dist,`${entry}_bg.wasm`),{force:true});
}
const output = join(dist,folder);
await mkdir(output,{recursive:true});
const scratch = join(target,'web-split',entry);
await mkdir(scratch,{recursive:true});
run(split, [join(target,'wasm32-unknown-unknown','web-release','sticktalk-web.wasm'), scratch, '--out-name', entry, '--target','web','--cost-model','none']);
run(bindgen, [join(scratch,`${entry}.wasm`),'--out-dir',scratch,'--out-name',entry,'--target','web','--keep-lld-exports','--no-demangle']);
await rm(join(scratch,`${entry}.wasm`));
for (const name of await readdir(scratch)) {
  if (name.endsWith('.wasm')) run(opt, [join(scratch,name),'-Oz','--strip-debug','--enable-bulk-memory','--enable-nontrapping-float-to-int','-o',join(scratch,name)]);
  await cp(join(scratch,name),join(output,name),{recursive:true});
}
