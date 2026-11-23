// Losing the connection: what the person sees, and how quickly it comes back.
// Synthetic loopback data only.
import assert from 'node:assert/strict';
import {writeFile} from 'node:fs/promises';
import {harness,register,form,until,wait,output} from './qa-harness.mjs';

const checks=[];
const h=await harness();
let page;
const errors=[];
try {
  page=await h.phone(1440,900);
  page.on('pageerror',e=>errors.push(String(e)));
  await page.addInitScript(()=>{
    const Socket=WebSocket;
    globalThis.__qaSockets=[];globalThis.__qaOpened=0;globalThis.__qaBanner=[];
    globalThis.WebSocket=class extends Socket {
      constructor(...args){super(...args);globalThis.__qaSockets.push(this);this.addEventListener('open',()=>globalThis.__qaOpened++);}
    };
    // Every moment the banner is on screen, however briefly.
    let shown=false;
    new MutationObserver(()=>{
      const now=!!document.querySelector('.connection-banner');
      if(now!==shown){shown=now;globalThis.__qaBanner.push([now?'shown':'hidden',Math.round(performance.now())]);}
    }).observe(document,{childList:true,subtree:true});
  });
  await register(page,h.origin,'qa_reconnect');
  await form(page,'create',{'f-name':'Reconnect regression'});
  await page.waitForSelector('#live-input');
  const say=async text=>{
    await page.locator('#live-input').fill(text);
    await page.locator('#live-input').press('Enter');
    await page.waitForFunction(text=>[...document.querySelectorAll('.markdown')].some(e=>e.textContent.includes(text)),text);
  };
  const opened=()=>page.evaluate(()=>globalThis.__qaOpened);
  const live=()=>page.evaluate(()=>globalThis.__qaSockets.filter(s=>s.readyState===1).length);
  const banner=()=>page.evaluate(()=>globalThis.__qaBanner.splice(0));
  await say('before the blip');
  await banner();

  // 1. A connection that drops and is back within a moment is not worth a banner.
  for(let round=0;round<3;round++) {
    const before=await opened();
    const started=Date.now();
    await page.evaluate(()=>globalThis.__qaSockets.at(-1).close());
    await until(async()=>await opened()>before&&await live()===1,10000);
    const took=Date.now()-started;
    await say(`after blip ${round}`);
    assert(took<3000,`a dropped connection is back within 3 s (took ${took} ms)`);
    assert.deepEqual(await banner(),[],`blip ${round}: the banner must not flash for a ${took} ms reconnect`);
  }
  checks.push('a connection that drops and resumes within a moment never shows the banner, and chat keeps working');

  // 2. A real outage does show it, and the app says so until the server is back.
  await h.stop();
  const down=Date.now();
  await page.locator('.connection-banner').filter({hasText:'Reconnecting…'}).waitFor({timeout:20000});
  const shownAfter=Date.now()-down;
  assert(shownAfter>=2000,`the banner waits out a short outage first (shown after ${shownAfter} ms)`);
  assert.equal(await page.locator('.skeleton-shell').count(),0,'the conversation stays on screen during an outage');
  checks.push(`an outage shows "Reconnecting…" after ${shownAfter} ms, with the conversation still on screen`);

  // 3. A restarted server is rejoined by the first connection that reaches it.
  await wait(3000);
  const beforeRestart=await opened();
  await h.start();
  const up=Date.now();
  await until(async()=>await page.locator('.connection-banner').count()===0&&await page.locator('#live-input').count()===1&&await live()===1,40000);
  const back=Date.now()-up;
  await say('after the restart');
  assert(await page.evaluate(()=>[...document.querySelectorAll('.markdown')].some(e=>e.textContent.includes('before the blip'))),'history is back after a restart');
  assert.equal(await opened()-beforeRestart,1,'a restarted server is rejoined on the first connection, not after a second wait');
  checks.push(`a restarted server is rejoined on the first connection (${back} ms after it was listening)`);

  assert.deepEqual(errors,[]);
} catch(error) {
  await writeFile(`${output}/reconnect-failure.json`,JSON.stringify({error:String(error),checks,log:h.log(),banner:await page?.evaluate(()=>globalThis.__qaBanner).catch(()=>null),body:await page?.locator('body').innerText().catch(()=>'')},null,2));
  throw error;
} finally {await h.close();}
await writeFile(`${output}/reconnect-results.json`,JSON.stringify({checks},null,2));
console.log(JSON.stringify({checks}));
