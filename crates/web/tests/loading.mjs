import {readFile,writeFile,mkdtemp,rm} from 'node:fs/promises';
import {join,resolve,dirname} from 'node:path';
import {hostGraphFingerprint,modules as runtimeModules} from '../scripts/version-runtime.mjs';
import vm from 'node:vm';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
process.env.CARGO_TARGET_DIR ??= fileURLToPath(new URL('../../../target/', import.meta.url));
const dist=process.env.STICKTALK_WEB_DIST ?? join(process.env.CARGO_TARGET_DIR,'web-dist');
const inventory=JSON.parse(await readFile(join(dist,'sizes.json'),'utf8'));
const core=inventory.assets.find(a=>a.file.endsWith('_bg.wasm'));
const chunks=inventory.assets.filter(a=>a.file.endsWith('.wasm')&&a!==core);
assert(core && chunks.length>=2, 'production must contain a core and actual lazy WASM chunks');
// 490 KB: the views carry their Tailwind utility classes and UI SDK elements,
// and the first screen now shows presence, who is in each voice channel,
// friend requests, day and "New" dividers and quoted/list/heading text.
assert(core.brotli<=490000, `core WASM exceeds 490 KB Brotli: ${core.brotli} B`);
const runtime=inventory.assets.filter(a=>/^public\/runtime\/[a-f0-9]{20}\/.*\.js$/.test(a.file));
assert.equal(runtime.length,runtimeModules.length,'all runtime JS modules are release-fingerprinted');
const alias=await readFile(join(dist,'public/media.js'),'utf8');
assert.match(alias,/export \* from '\.\/runtime\/[a-f0-9]{20}\/media\.js'/);
const liveRuntime=runtime.find(a=>a.file.endsWith('/live.js'));
const glue=await readFile(join(dist,core.file.replace(/_bg\.wasm$/,'.js')),'utf8');
assert(glue.includes('/'+liveRuntime.file),'WASM host bindings use the same fingerprinted JS release');
assert.match(glue,/"\/public\/live\.js": import/,'preserve raw WASM import-object keys while rewriting JS specifiers');
assert.notEqual(core.file.split('/')[1],core.file.match(/sticktalk-web-([a-f0-9]{20})_bg/)[1],
  'package loader URL fingerprints the host graph, not only the unchanged WASM');
const graphFixture=await mkdtemp(join(resolve(process.env.CARGO_TARGET_DIR),'graph-unit-'));
try {
  for(const file of runtimeModules)
    await writeFile(join(graphFixture,file),`export const fixture='${file}';`);
  const wasm=Buffer.from('unchanged core');
  const first=await hostGraphFingerprint(wasm,graphFixture);
  assert.equal(await hostGraphFingerprint(wasm,graphFixture),first,'stable module graph URL');
  await writeFile(join(graphFixture,'media.js'),'export const changed=true;');
  assert.notEqual(await hostGraphFingerprint(wasm,graphFixture),first,'JS-only release gets a new immutable loader URL');
} finally {
  assert.equal(dirname(graphFixture),resolve(process.env.CARGO_TARGET_DIR));
  await rm(graphFixture,{recursive:true,force:true});
}
const liveSource=await readFile(join(dist,liveRuntime.file),'utf8');
assert(!liveSource.includes('import("/public/media.js")'),'lazy imports cannot drift onto a newly deployed release');
const handlers=new Map(), stores=new Map(), requests=[]; let activates=0;
const caches={
  open:async key=>{
    if(!stores.has(key))stores.set(key,new Map());
    const store=stores.get(key);
    return {addAll:async paths=>{for(const p of paths)store.set(p,new Response('cached '+p));},match:async p=>store.get(p)?.clone(),put:async(p,r)=>store.set(p,r.clone()),
      keys:async()=>[...store.keys()].map(p=>({url:new URL(p,'https://sticktalk.test').href})),
      delete:async r=>store.delete(new URL(r.url).pathname)};
  }, keys:async()=>[...stores.keys()], delete:async key=>stores.delete(key),
};
const self={location:{origin:'https://sticktalk.test'},addEventListener:(type,fn)=>handlers.set(type,fn),skipWaiting:()=>activates++,clients:{claim:async()=>{}},registration:{}};
vm.runInNewContext(await readFile(join(dist,'sw.js'),'utf8'),{self,caches,URL,fetch:async request=>{requests.push(request.url);return new Response('fresh');}});
let pending;
handlers.get('install')({waitUntil:p=>pending=p}); await pending;
assert.equal(activates,0,'updates wait for a deliberate user reload');
const store=[...stores.values()][0];
assert(store.has('/'+core.file));
for(const c of chunks)assert(!store.has('/'+c.file),'do not precache lazy chunks');
assert(![...store.keys()].some(p=>p.startsWith('/api/')));
const request=async path=>{
  let response,work;
  handlers.get('fetch')({request:{method:'GET',url:'https://sticktalk.test'+path,mode:'cors'},respondWith:p=>response=p,waitUntil:p=>work=p});
  const result=response&&await response;
  await work;
  return result;
};
assert.equal(await request('/api/v1/browser/session'),undefined);
const cached=await request('/'+core.file);
assert((await cached.text()).startsWith('cached '));
assert(!requests.includes('https://sticktalk.test/'+core.file),'content-versioned core must not re-download on a warm visit');
const shell=await request('/index.html');
assert((await shell.text()).startsWith('cached '));
assert(requests.includes('https://sticktalk.test/index.html'),'shell uses stale-while-revalidate');
await request('/'+chunks[0].file);
assert(store.has('/'+chunks[0].file),'cache secondary screens only on demand');
handlers.get('message')({data:'activate-update'}); assert.equal(activates,1);
store.set('/api/v1/messages/private',new Response('private'));
store.set('/unlisted-private-data',new Response('private'));
await caches.open('sticktalk-private-test');
await caches.open('unrelated-app');
handlers.get('message')({data:'purge-private',waitUntil:p=>pending=p});await pending;
assert(!store.has('/api/v1/messages/private'));
assert(!store.has('/unlisted-private-data'));
assert(!stores.has('sticktalk-private-test'));
assert(stores.has('unrelated-app'));
assert(store.has('/'+core.file),'logout preserves the public offline shell');
console.log(`Loading: core ${core.brotli} B Brotli, ${chunks.length} real lazy chunks; shell/core precache, stale-while-revalidate, API exclusion, deliberate updates: passed`);

const {directSearch}=await import('../public/gifs.js');
const savedFetch=globalThis.fetch;
let media='https://static.klipy.com/small.webm';
globalThis.fetch=async(url,options)=>{
  assert.equal(new URL(url).hostname,'api.klipy.com'); assert.equal(options.credentials,'omit'); assert.equal(options.redirect,'error');
  return new Response(JSON.stringify({results:[{id:'123',title:'Hello',media_formats:{nanowebm:{url:media,dims:[150,100]},mediumgif:{url:'https://static2.klipy.com/send.gif',dims:[498,332]}}}]}));
};
try {
  const page=JSON.parse(await directSearch('public-test-key','hello'));
  assert.equal(page.items[0].id,'klipy:123');
  media='https://static.klipy.com.attacker.test/small.webm';
  await assert.rejects(()=>directSearch('public-test-key','hello'),/host/);
} finally {globalThis.fetch=savedFetch;}
console.log('Direct KLIPY adapter: public key only, omitted credentials, fixed API origin, exact media host validation: passed');
