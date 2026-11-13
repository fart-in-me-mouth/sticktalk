import {readFile} from 'node:fs/promises';
import {join} from 'node:path';
import vm from 'node:vm';
import assert from 'node:assert/strict';
const dist=process.env.STICKTALK_WEB_DIST ?? join(process.env.CARGO_TARGET_DIR,'web-dist');
const inventory=JSON.parse(await readFile(join(dist,'sizes.json'),'utf8'));
const core=inventory.assets.find(a=>a.file.endsWith('_bg.wasm'));
const chunks=inventory.assets.filter(a=>a.file.endsWith('.wasm')&&a!==core);
assert(core && chunks.length>=2, 'production must contain a core and actual lazy WASM chunks');
const handlers=new Map(), stores=new Map(), requests=[]; let activates=0;
const caches={
  open:async key=>{
    if(!stores.has(key))stores.set(key,new Map());
    const store=stores.get(key);
    return {addAll:async paths=>{for(const p of paths)store.set(p,new Response('cached '+p));},match:async p=>store.get(p)?.clone(),put:async(p,r)=>store.set(p,r.clone())};
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
