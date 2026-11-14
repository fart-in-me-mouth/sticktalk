import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import { mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';

// Keep the real-server suites runnable directly, while reporting every engine,
// feature and repetition independently. No retries or mock REST/WS transport.
for (const suite of ['polish', 'phone', 'session-revocation', 'admin', 'oauth', 'pwa', 'phone-messaging', 'phone-media', 'voice-negotiation']) {
  test(suite, async ({}, info) => {
    test.skip(process.platform === 'win32' && info.project.name === 'webkit' && ['phone-media','voice-negotiation'].includes(suite), 'Windows Playwright WebKit has no WebRTC/mediaDevices; run on macOS/iPhone for real H.264/voice.');
    const output = resolve('screenshots/engines', info.project.name, `run-${info.repeatEachIndex + 1}`, suite);
    await mkdir(output, { recursive: true });
    const child = spawn(process.execPath, [`tests/${suite === 'phone-messaging' ? 'phone-media' : suite}.mjs`], {
      windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
      env: { ...process.env, STICKTALK_QA_ENGINE: info.project.name, STICKTALK_QA_MEDIA_MODE: suite === 'phone-messaging' ? 'ui' : 'full', STICKTALK_QA_DIR: output },
    });
    let log = '';
    child.stdout.on('data', data => { log += data; });
    child.stderr.on('data', data => { log += data; });
    const timer = setTimeout(() => child.kill(), 9 * 60 * 1000);
    try {
      const code = await new Promise((done, fail) => { child.on('error', fail); child.on('exit', done); });
      await writeFile(`${output}/run.log`, log);
      await info.attach('suite-log', { body: log, contentType: 'text/plain' });
      expect(code, log).toBe(0);
    } finally { clearTimeout(timer); if (child.exitCode === null) child.kill(); }
  });
}
