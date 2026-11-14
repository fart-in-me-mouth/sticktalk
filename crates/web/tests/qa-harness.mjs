// All traffic/data in this harness is synthetic and loopback-only.
import { chromium } from 'playwright';
import { spawn, execFileSync } from 'node:child_process';
import { createServer, connect } from 'node:net';
import { createServer as createHttpsServer } from 'node:https';
import { request as httpRequest } from 'node:http';
import { once } from 'node:events';
import { mkdtemp, mkdir, readFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const target = resolve(process.env.CARGO_TARGET_DIR ?? fileURLToPath(new URL('../../../target/', import.meta.url)));
export const output = process.env.STICKTALK_QA_DIR ?? 'screenshots/qa';
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
export async function harness(extra = [], {tls=false,dist}={}) {
  await mkdir(target, { recursive: true });
  await mkdir(output, { recursive: true });
  const data = await mkdtemp(join(target, 'qa-data-'));
  const port = await freePort(), publicPort=tls?await freePort():port;
  const origin = `${tls?'https':'http'}://127.0.0.1:${publicPort}`;
  const webRoot=dist??process.env.STICKTALK_WEB_DIST??join(target,'web-dist');
  const revision=JSON.parse(await readFile(join(webRoot,'sizes.json'),'utf8')).revision;
  let proxy;
  const sockets=new Set();
  if(tls) {
    // Ephemeral cert files only: never install a certificate or change an OS store.
    const key=join(data,'key.pem'),cert=join(data,'cert.pem');
    execFileSync(process.env.STICKTALK_QA_OPENSSL ?? 'openssl',[
      'req','-x509','-newkey','rsa:2048','-nodes','-keyout',key,'-out',cert,
      '-subj','/CN=127.0.0.1','-days','1','-addext','subjectAltName=IP:127.0.0.1',
    ],{windowsHide:true,stdio:'ignore'});
    proxy=createHttpsServer({key:await readFile(key),cert:await readFile(cert)},(req,res)=>{
      const upstream=httpRequest({hostname:'127.0.0.1',port,path:req.url,method:req.method,headers:req.headers},response=>{
        res.writeHead(response.statusCode,response.headers);response.pipe(res);
      });
      upstream.on('error',()=>{res.writeHead(502);res.end();});req.pipe(upstream);
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
  const server = spawn(process.env.STICKTALK_SERVER_EXE ?? join(target, `debug/sticktalk-server${process.platform === 'win32' ? '.exe' : ''}`), [
    '--bind', `127.0.0.1:${port}`, '--public-url', origin, '--data-dir', data,
    '--web-root', webRoot,
    '--min-free-disk-gb', '0', '--voice-bind-ip', '127.0.0.1',
    '--voice-advertised-ip', '127.0.0.1', ...extra,
  ], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  let log = '';
  try {
    await new Promise((done, fail) => {
      const timer = setTimeout(() => fail(Error(log)), 30000);
      const collect = chunk => {
        log = (log + chunk).slice(-20000);
        if (log.includes('StickTalk listening')) { clearTimeout(timer); done(); }
      };
      server.stdout.on('data', collect); server.stderr.on('data', collect);
      server.once('error', e => { clearTimeout(timer); fail(e); });
      server.once('exit', code => { clearTimeout(timer); fail(Error(`Server exit ${code}: ${log}`)); });
    });
    const browser = await chromium.launch({ headless: true, args: [
      '--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream',
      '--allow-loopback-in-peer-connection', '--disable-features=WebRtcHideLocalIpsWithMdns',
      '--mute-audio', '--disable-notifications',
      '--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1, EXCLUDE localhost',
      ...(tls?['--ignore-certificate-errors']:[]),
    ] });
    const contexts = [];
    return { origin, data, browser, revision, log: () => log,
      async phone(width = 390, height = 844) {
        const context = await browser.newContext({ viewport: { width, height },
          hasTouch: width < 1000, isMobile: width < 768, deviceScaleFactor: 1,
          permissions: ['microphone', 'camera'],ignoreHTTPSErrors:tls });
        // Never permit an external request or OS notification during QA.
        await context.route('**/*', route => {
          const url = new URL(route.request().url());
          return ['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname)
            ? route.continue() : route.abort('blockedbyclient');
        });
        contexts.push(context);
        const page = await context.newPage();
        await page.addInitScript(() => {
          window.__qaErrors = []; window.__qaCsp = [];
          addEventListener('error', e => window.__qaErrors.push(e.message));
          document.addEventListener('securitypolicyviolation', e => window.__qaCsp.push(e.effectiveDirective));
        });
        return page;
      },
      async close() {
        await browser.close();
        if(proxy){for(const socket of sockets)socket.destroy();await new Promise(done=>proxy.close(done));}
        if (server.exitCode === null) { const exit = once(server, 'exit'); server.kill(); await exit; }
        // Test DBs stay under the one task target and are removed with that dir.
      },
    };
  } catch (e) { server.kill();if(proxy){for(const socket of sockets)socket.destroy();proxy.close();}throw e; }
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
  await page.screenshot({ path });
  return path;
}
// Capture transient flows at the same three review sizes without repeating
// mutations/OAuth tickets. Restore the caller's viewport and scroll positions.
export async function shotAll(page, screen) {
  const viewport = page.viewportSize();
  const scroll = await page.evaluate(() => [...document.querySelectorAll('.settings-main,.settings-nav,.live-modal,#live-messages')]
    .map(e => [e.id ? `#${e.id}` : `.${e.classList[0]}`, e.scrollTop]));
  try {
    for (const [width,height] of [[390,844],[412,915],[1440,900]]) {
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
