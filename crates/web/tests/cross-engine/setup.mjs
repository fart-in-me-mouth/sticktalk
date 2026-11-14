import { access, mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { devices } from 'playwright';
import { execFileSync } from 'node:child_process';

export default async function setup() {
  const target=resolve(process.env.CARGO_TARGET_DIR ?? '../../target');
  const evidence=resolve(process.env.STICKTALK_ENGINE_EVIDENCE_ROOT ?? 'screenshots/engines/latest');
  const sizes=JSON.parse(await readFile(join(target,'web-dist/sizes.json'),'utf8'));
  await access(process.env.STICKTALK_SERVER_EXE ?? join(target,`debug/sticktalk-server${process.platform==='win32'?'.exe':''}`));
  await mkdir(evidence,{recursive:true});
  await writeFile(join(evidence,'environment.json'),JSON.stringify({startedAt:new Date().toISOString(),
    platform:process.platform,node:process.version,revision:sizes.revision,headless:true,retries:0,
    codeHead:execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8',windowsHide:true}).trim(),
    devices:{webkit:devices['iPhone 15'],chromium:devices['Pixel 7'],firefox:'412x839 touch viewport (no isMobile)'},
    webkitWindowsMedia:'Blocked: mediaDevices/RTCPeerConnection/AudioContext absent; MP4 decoder unavailable',
    firefoxMedia:'Real Cisco OpenH264 GMP; fake capture + test-profile media grants; real SFU/RTP',
    firefoxOffline:'Native offline + SW reload; inject only the missing Playwright online event after navigator.onLine returns true',
    gestures:'Chromium native CDP multi-touch; WebKit/Firefox public touchscreen taps plus DOM pointer/touch sequences for multi-touch and hold',
    bundle:sizes},null,2));
}
