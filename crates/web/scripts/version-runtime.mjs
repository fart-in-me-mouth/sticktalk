// Keep a waiting-update tab's first lazy JS import on its own release, just as
// pkg/<digest>/ already does for split WASM. Stable aliases preserve diagnostics.
import {createHash} from 'node:crypto';
import {mkdir,readFile,readdir,writeFile} from 'node:fs/promises';
import {join} from 'node:path';

const modules=['calls.js','content.js','gifs.js','live.js','media.js','message-menu.js','nav.js','navigation.js','preferences.js','ptt.js','push.js','ui.js','viewer.js'];
async function readRuntime(directory) {
  const hash=createHash('sha256'), sources=new Map();
  for(const name of modules) {
    const source=await readFile(join(directory,name),'utf8');
    sources.set(name,source);hash.update(name).update('\0').update(source);
  }
  return {digest:hash.digest('hex').slice(0,20),sources};
}
export async function runtimeFingerprint(directory) {
  return (await readRuntime(directory)).digest;
}
export async function hostGraphFingerprint(wasm,directory) {
  return createHash('sha256').update(wasm).update(await runtimeFingerprint(directory))
    .update(await readFile(new URL('./version-runtime.mjs',import.meta.url)))
    .digest('hex').slice(0,20);
}
export async function versionRuntime(dist) {
  const {digest,sources}=await readRuntime(join(dist,'public'));
  const prefix=`/public/runtime/${digest}/`;
  await mkdir(join(dist,'public/runtime',digest),{recursive:true});
  const rewrite=source=>modules.reduce((text,name)=>text.replaceAll(`/public/${name}`,prefix+name),source);
  for(const [name,source] of sources) {
    await writeFile(join(dist,'public/runtime',digest,name),rewrite(source));
    await writeFile(join(dist,'public',name),`export * from './runtime/${digest}/${name}';\n`);
  }
  async function glue(dir) {
    for(const entry of await readdir(dir,{withFileTypes:true})) {
      const path=join(dir,entry.name);
      if(entry.isDirectory())await glue(path);
      else if(entry.name.endsWith('.js')) {
        const source=await readFile(path,'utf8');
        // Change JS module specifiers only. WASM import-object keys retain the
        // original raw_module names embedded in the wire-compatible binary.
        const next=source.replace(/(\bfrom\s*['"])\/public\/(live\.js)(['"])/g,
          (_,before,name,after)=>before+prefix+name+after);
        if(next!==source)await writeFile(path,next);
      }
    }
  }
  await glue(join(dist,'pkg'));
  return digest;
}
