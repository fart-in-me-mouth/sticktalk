// All traffic/data in this harness is synthetic and loopback-only.
import { engines, engine, device, launchOptions, contextOptions } from './engines.mjs';
import { spawn, execFileSync } from 'node:child_process';
import { createServer, connect } from 'node:net';
import { createServer as createHttpsServer } from 'node:https';
import { request as httpRequest } from 'node:http';
import { once } from 'node:events';
import { mkdtemp, mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const target = resolve(process.env.CARGO_TARGET_DIR ?? fileURLToPath(new URL('../../../target/', import.meta.url)));
export const output = process.env.STICKTALK_QA_DIR ?? 'screenshots/qa';
const screenshotPolicyEvents=[];
export const act = (page, action) => page.evaluate(action =>
  window.dispatchEvent(new CustomEvent('sticktalk-live', { detail: action })), action);
export const wait = ms => new Promise(done => setTimeout(done, ms));
// Playwright 1.65 waitForFunction treats an async predicate's Promise as truthy,
// even when it resolves false. Await diagnostic/API predicates in Node instead.
export async function until(predicate,timeout=30000) {
  const deadline=Date.now()+timeout;
  while(Date.now()<deadline){if(await predicate())return;await wait(100);}
  throw Error(`Async condition did not become true within ${timeout}ms`);
}
export async function freePort() {
  const socket = createServer();
  socket.listen(0, '127.0.0.1');
  await once(socket, 'listening');
  const port = socket.address().port;
  await new Promise(done => socket.close(done));
  return port;
}
export async function harness(extra = [], {tls=engine === 'webkit',dist,beforeRequest}={}) {
  await mkdir(target, { recursive: true });
  await mkdir(output, { recursive: true });
  const data = await mkdtemp(join(target, 'qa-data-'));
  const port = await freePort(), publicPort=tls?await freePort():port;
  const origin = `${tls?'https':'http'}://127.0.0.1:${publicPort}`;
  const webRoot=dist??process.env.STICKTALK_WEB_DIST??join(target,'web-dist');
  const revision=JSON.parse(await readFile(join(webRoot,'sizes.json'),'utf8')).revision;
  let proxy, denyProxy;
  const sockets=new Set();
  if(tls) {
    // Ephemeral cert files only: never install a certificate or change an OS store.
    const key=join(data,'key.pem'),cert=join(data,'cert.pem');
    execFileSync(process.env.STICKTALK_QA_OPENSSL ?? 'openssl',[
      'req','-x509','-newkey','rsa:2048','-nodes','-keyout',key,'-out',cert,
      '-subj','/CN=127.0.0.1','-days','1','-addext','subjectAltName=IP:127.0.0.1',
    ],{windowsHide:true,stdio:'ignore'});
    proxy=createHttpsServer({key:await readFile(key),cert:await readFile(cert)},async(req,res)=>{
      await beforeRequest?.(req);
      const upstream=httpRequest({hostname:'127.0.0.1',port,path:req.url,method:req.method,headers:req.headers},response=>{
        res.writeHead(response.statusCode,response.headers);response.pipe(res);
      });
      upstream.on('error',()=>{
        if(res.destroyed)return;
        if(res.headersSent){res.destroy();return;}
        res.writeHead(502);res.end();
      });
      res.on('close',()=>upstream.destroy());
      req.pipe(upstream);
    });
    proxy.on('connection',socket=>{sockets.add(socket);socket.on('close',()=>sockets.delete(socket));});
    proxy.on('upgrade',(req,socket,head)=>{
      const upstream=connect(port,'127.0.0.1',()=>{
        upstream.write(`${req.method} ${req.url} HTTP/${req.httpVersion}\r\n${req.rawHeaders.reduce((s,h,i)=>s+h+(i%2?'\r\n':': '),'')}\r\n`);
        if(head.length)upstream.write(head);socket.pipe(upstream);upstream.pipe(socket);
      });
      upstream.on('error',()=>socket.destroy());socket.on('error',()=>upstream.destroy());socket.on('close',()=>upstream.destroy());
    });
    proxy.listen(publicPort,'127.0.0.1');await once(proxy,'listening');
  }
  const exe=process.env.STICKTALK_SERVER_EXE ?? join(target, `debug/sticktalk-server${process.platform === 'win32' ? '.exe' : ''}`);
  const args=[
    '--bind', `127.0.0.1:${port}`, '--public-url', origin, '--data-dir', data,
    '--web-root', webRoot,
    '--min-free-disk-gb', '0', '--voice-bind-ip', '127.0.0.1',
    '--voice-advertised-ip', '127.0.0.1', ...extra,
  ];
  let server, log = '';
  // Also used to restart on the same port and data folder, as a deployment does.
  const start = async () => {
    const child = server = spawn(exe, args, { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    let seen = '';
    await new Promise((done, fail) => {
      const timer = setTimeout(() => fail(Error(log)), 30000);
      const collect = chunk => {
        log = (log + chunk).slice(-20000); seen = (seen + chunk).slice(-20000);
        if (seen.includes('StickTalk listening')) { clearTimeout(timer); done(); }
      };
      child.stdout.on('data', collect); child.stderr.on('data', collect);
      child.once('error', e => { clearTimeout(timer); fail(e); });
      child.once('exit', code => { clearTimeout(timer); fail(Error(`Server exit ${code}: ${log}`)); });
    });
  };
  const stop = async () => {
    if (server.exitCode === null && server.signalCode === null) { const exit = once(server, 'exit'); server.kill(); await exit; }
  };
  try {
    await start();
    // Firefox request interception defeats native service-worker offline reload.
    // Block non-loopback HTTP(S) through a loopback-only rejecting proxy instead.
    let browserOptions=launchOptions;
    if(engine==='firefox') {
      denyProxy=createServer(socket=>socket.destroy());
      denyProxy.listen(0,'127.0.0.1');await once(denyProxy,'listening');
      browserOptions={...launchOptions,proxy:{server:`http://127.0.0.1:${denyProxy.address().port}`,bypass:'127.0.0.1,localhost,[::1]'}};
    }
    const browser = await engines[engine].browser.launch(browserOptions);
    const contexts = [];
    return { origin, data, browser, revision, log: () => log, stop, start,
      async phone(width = device.viewport.width, height = device.viewport.height) {
        const context = await browser.newContext(contextOptions(width, height, tls));
        // Never permit an external request or OS notification during QA.
        if(engine!=='firefox')await context.route('**/*', route => {
          const url = new URL(route.request().url());
          return ['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname)
            ? route.continue() : route.abort('blockedbyclient');
        });
        contexts.push(context);
        const page = await context.newPage();
        await page.addInitScript(() => {
          window.__qaErrors = []; window.__qaCsp = []; window.__qaScreenshotCsp=[];
          let screenshotStyle=false;
          new MutationObserver(records=>{
            for(const record of records)for(const node of record.addedNodes)
              if(window.__qaScreenshotActive&&node.nodeName==='STYLE'&&node.textContent==='body {}')screenshotStyle=true;
          }).observe(document,{childList:true,subtree:true});
          addEventListener('error', e => window.__qaErrors.push(e.message));
          document.addEventListener('securitypolicyviolation', e => {
            const entry={directive:e.effectiveDirective,blocked:e.blockedURI,source:e.sourceFile,line:e.lineNumber,sample:e.sample};
            // Playwright WebKit screenshotter injects exactly `body {}` to sync
            // animations, even with animations allowed/caret initial. Our real
            // CSP correctly blocks it. Keep these separately, NEVER bypass CSP
            // or suppress any other policy violation.
            if(window.__qaScreenshotActive&&screenshotStyle&&entry.directive==='style-src-elem'&&entry.blocked==='inline'&&entry.source===''&&entry.line===5)
              window.__qaScreenshotCsp.push({...entry,stylesheet:'body {}',tool:'Playwright WebKit screenshot animation sync'});
            else window.__qaCsp.push(entry);
            screenshotStyle=false;
          });
        });
        return page;
      },
      async close() {
        await browser.close();
        if(denyProxy)await new Promise(done=>denyProxy.close(done));
        if(proxy){for(const socket of sockets)socket.destroy();await new Promise(done=>proxy.close(done));}
        await stop();
        // Test DBs stay under the one task target and are removed with that dir.
      },
    };
  } catch (e) { server.kill();denyProxy?.close();if(proxy){for(const socket of sockets)socket.destroy();proxy.close();}throw e; }
}
export async function register(page, origin, username) {
  await page.goto(origin);
  await page.locator('[data-live=register]').click();
  for(let attempt=0;attempt<8;attempt++) {
    await page.locator('#auth-username').fill(username);
    await page.locator('#auth-password').fill('synthetic-qa-password-1234');
    const [response]=await Promise.all([
      page.waitForResponse(r=>new URL(r.url()).pathname==='/api/v1/browser/session/register'),
      page.locator('[type=submit]').click(),
    ]);
    if(response.status()===429){await wait(Math.max(2,Number(response.headers()['retry-after'])||0)*1000);continue;}
    if(!response.ok())throw Error(`Register failed: ${response.status()} ${await response.text()}`);
    break;
  }
  await page.waitForSelector('.live-home');
  await page.waitForFunction(() => !document.querySelector('.connection-banner'));
}
export async function login(page,username,password) {
  for(let attempt=0;attempt<8;attempt++) {
    await page.locator('#auth-username').fill(username);
    await page.locator('#auth-password').fill(password);
    const [response]=await Promise.all([
      page.waitForResponse(r=>new URL(r.url()).pathname==='/api/v1/browser/session/login'),
      page.locator('[type=submit]').click(),
    ]);
    if(response.status()===429){await wait(Math.max(2,Number(response.headers()['retry-after'])||0)*1000);continue;}
    if(!response.ok())throw Error(`Login failed: ${response.status()} ${await response.text()}`);
    await page.waitForSelector('.live-home');return;
  }
  throw Error('Synthetic login still rate-limited after bounded Retry-After waits');
}
export async function form(page, kind, fields = {}) {
  await act(page, `dialog:${kind}`);
  await page.waitForSelector(`[data-live-form="${kind}"]`);
  for (const [id, value] of Object.entries(fields)) {
    const element = page.locator(`#${id}`);
    if (await element.evaluate(e => e.tagName === 'SELECT')) await element.selectOption(value);
    else await element.fill(value);
  }
  await page.locator(`[data-live-form="${kind}"] [type=submit]`).click();
}
export async function shot(page, screen) {
  await page.evaluate(() => document.fonts.ready);
  await wait(100);
  const { width, height } = page.viewportSize();
  const path = `${output}/${screen}-${width}x${height}.png`;
  await page.evaluate(()=>window.__qaScreenshotActive=true);
  try { await page.screenshot({ path, caret:'initial' }); }
  finally {
    const events=await page.evaluate(async()=>{
      await new Promise(done=>setTimeout(done,0));window.__qaScreenshotActive=false;
      return window.__qaScreenshotCsp?.splice(0)??[];
    }).catch(()=>[]);
    for(const event of events)screenshotPolicyEvents.push({screen,...event});
    if(events.length)await writeFile(`${output}/screenshot-tooling-csp.json`,JSON.stringify(screenshotPolicyEvents,null,2));
  }
  return path;
}
// Capture transient flows at the same three review sizes without repeating
// mutations/OAuth tickets. Restore the caller's viewport and scroll positions.
export async function shotAll(page, screen) {
  const viewport = page.viewportSize();
  const scroll = await page.evaluate(() => [...document.querySelectorAll('.settings-main,.settings-nav,.live-modal,#live-messages')]
    .map(e => [e.id ? `#${e.id}` : `.${e.classList[0]}`, e.scrollTop]));
  try {
    for (const [width,height] of process.env.STICKTALK_QA_ENGINE ? [[viewport.width,viewport.height]] : [[390,844],[412,915],[1440,900]]) {
      await page.setViewportSize({width,height});
      await shot(page, screen);
    }
  } finally {
    await page.setViewportSize(viewport);
    await page.evaluate(scroll => scroll.forEach(([selector,top]) => {
      const pane=document.querySelector(selector);if(pane)pane.scrollTop=top;
    }), scroll);
  }
  return `${output}/${screen}-${viewport.width}x${viewport.height}.png`;
}
