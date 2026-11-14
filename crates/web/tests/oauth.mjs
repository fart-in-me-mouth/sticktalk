// Mock authorization, token and JWKS endpoints. No Google request leaves Chromium.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { generateKeyPairSync, createHash, sign, randomUUID } from 'node:crypto';
import { once } from 'node:events';
import { writeFile } from 'node:fs/promises';
import { harness, register, act, shotAll as shot, output } from './qa-harness.mjs';
const {privateKey,publicKey}=generateKeyPairSync('rsa',{modulusLength:2048});
const jwk={...publicKey.export({format:'jwk'}),kid:'qa-key',alg:'RS256',use:'sig'};
const codes=new Map(), checks=[],screenshots=[],errors=[],external=[];
let mockOrigin,h,page;
const mock=createServer(async(req,res)=>{
  try {
    const u=new URL(req.url,mockOrigin);
    if(u.pathname==='/jwks'){res.setHeader('Cache-Control','max-age=3600');return res.end(JSON.stringify({keys:[jwk]}));}
    if(u.pathname==='/authorize') {
      const q=u.searchParams;
      assert.equal(q.get('code_challenge_method'),'S256');assert.equal(q.get('nonce'),q.get('state'));
      assert.equal(new URL(q.get('redirect_uri')).hostname,'127.0.0.1');
      const code=randomUUID();codes.set(code,Object.fromEntries(q));
      const callback=new URL(q.get('redirect_uri'));callback.searchParams.set('state',q.get('state'));callback.searchParams.set('code',code);
      res.writeHead(302,{location:callback.href});return res.end();
    }
    if(u.pathname==='/token') {
      let raw='';for await(const chunk of req)raw+=chunk;
      const body=new URLSearchParams(raw),code=body.get('code'),authorization=codes.get(code);codes.delete(code);
      assert(authorization,'one-time authorization code');
      assert.equal(createHash('sha256').update(body.get('code_verifier')).digest('base64url'),authorization.code_challenge);
      assert.equal(body.get('redirect_uri'),authorization.redirect_uri);
      const now=Math.floor(Date.now()/1000);
      const enc=value=>Buffer.from(JSON.stringify(value)).toString('base64url');
      const payload=`${enc({alg:'RS256',kid:'qa-key'})}.${enc({iss:mockOrigin,aud:'qa-google-client',sub:'synthetic-google-user',iat:now,exp:now+3600,email_verified:true,nonce:authorization.state})}`;
      res.setHeader('Content-Type','application/json');return res.end(JSON.stringify({id_token:`${payload}.${sign('RSA-SHA256',Buffer.from(payload),privateKey).toString('base64url')}`}));
    }
    res.writeHead(404);res.end();
  } catch(e) {errors.push(String(e));res.writeHead(500);res.end();}
});
mock.listen(0,'127.0.0.1');await once(mock,'listening');mockOrigin=`http://127.0.0.1:${mock.address().port}`;
try {
  h=await harness(['--google-web-client-id','qa-google-client','--google-web-client-secret','qa-mock-secret',
    '--mock-google-token-url',`${mockOrigin}/token`,'--mock-google-jwks',`${mockOrigin}/jwks`,'--mock-google-issuer',mockOrigin],{tls:true});
  const seed=await h.phone();await register(seed,h.origin,'qa_taken');await seed.context().close();
  page=await h.phone();page.on('pageerror',e=>errors.push(String(e)));
  page.on('request',request=>{if(!['127.0.0.1','localhost','[::1]'].includes(new URL(request.url()).hostname))external.push(request.url());});
  await page.context().route('**/api/v1/browser/google/start*',async route=>{
    const response=await route.fetch({maxRedirects:0});
    const authorization=new URL(response.headers().location);
    assert.equal(authorization.hostname,'accounts.google.com');
    return route.fulfill({response,headers:{...response.headers(),location:`${mockOrigin}/authorize${authorization.search}`}});
  });
  await page.goto(h.origin);await page.getByRole('link',{name:'Continue with Google'}).waitFor();
  screenshots.push(await shot(page,'login-google'));
  await page.locator('[data-live=register]').click();screenshots.push(await shot(page,'register-google'));
  await page.locator('[data-live=register]').click();await page.getByRole('link',{name:'Continue with Google'}).click();
  await page.waitForSelector('#auth-username');
  await page.waitForFunction(()=>document.querySelector('h1')?.textContent==='Choose a username');
  screenshots.push(await shot(page,'google-username'));
  await page.locator('#auth-username').fill('qa_taken');await page.locator('[type=submit]').click();
  await page.waitForSelector('.live-error');
  assert.equal(await page.locator('#auth-username').inputValue(),'qa_taken');
  await page.locator('#auth-username').fill('qa_google');await page.locator('[type=submit]').click();
  await page.waitForSelector('.live-home');
  checks.push('mock Google authorization + PKCE/token + RSA JWKS, signup and username collision');
  await act(page,'dialog:settings');await page.getByRole('button',{name:'Log Out',exact:true}).click();
  await page.waitForSelector('#auth-username');await page.getByRole('link',{name:'Continue with Google'}).click();
  await page.waitForSelector('.live-home');checks.push('existing Google identity login after logout');
  await act(page,'dialog:delete-account');await page.getByRole('button',{name:'Verify with Google',exact:true}).click();
  await page.waitForURL(/reauth=delete_account/);
  await page.waitForSelector('.live-modal[data-dialog="delete-account"]');
  screenshots.push(await shot(page,'google-delete-reauth'));
  await page.locator('#f-value').fill('DELETE');await page.locator('[type=submit]').click();
  await page.waitForSelector('#auth-username');
  checks.push('operation-bound Google reauthentication returns to Delete Account and erases account');
  assert.deepEqual(errors,[]);
  assert.deepEqual(external,[]);
  await writeFile(`${output}/oauth-results.json`,JSON.stringify({revision:h.revision,checks,screenshots,errors,externalRequests:external.length},null,2));
  console.log(JSON.stringify({checks,errors}));
} catch(e) {
  await writeFile(`${output}/oauth-failure.json`,JSON.stringify({error:String(e),checks,errors,body:await page?.locator('body').innerText().catch(()=>'' )},null,2));throw e;
} finally {await h?.close();await new Promise(done=>mock.close(done));}
