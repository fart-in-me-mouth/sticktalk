import { chromium, webkit, firefox, devices } from 'playwright';
import { resolve } from 'node:path';

export const engines = {
  webkit: { browser: webkit, device: devices['iPhone 15'] },
  chromium: { browser: chromium, device: devices['Pixel 7'] },
  // Firefox does not implement Playwright's isMobile emulation. This is a
  // touch-enabled narrow viewport, not an assertion about Firefox for Android.
  firefox: { browser: firefox, device: { viewport: { width: 412, height: 839 },
    screen: { width: 412, height: 915 }, deviceScaleFactor: 2.625, hasTouch: true } },
};
export const engine = process.env.STICKTALK_QA_ENGINE ?? 'chromium';
if (!engines[engine]) throw Error(`Unknown QA engine: ${engine}`);
export const device = engines[engine].device;
export const phoneViewports = process.env.STICKTALK_QA_ENGINE
  ? [[device.viewport.width, device.viewport.height]] : [[390,844],[412,915],[820,1180],[1440,900]];
export const launchOptions = engine === 'chromium' ? { headless: true, args: [
  '--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream',
  '--allow-loopback-in-peer-connection', '--disable-features=WebRtcHideLocalIpsWithMdns',
  '--mute-audio', '--disable-notifications',
  '--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1, EXCLUDE localhost',
] } : engine === 'firefox' ? { headless: true,
  env: {...process.env, MOZ_GMP_PATH:process.env.MOZ_GMP_PATH ?? resolve(process.env.CARGO_TARGET_DIR ?? '../../../target','tools/gmp-gmpopenh264/2.6.0')},
  firefoxUserPrefs: {
  'media.navigator.streams.fake': true, 'media.navigator.permission.disabled': true,
  // Fake capture alone does not count as an origin media grant for Firefox ICE
  // privacy. Grant only in this disposable fake-device browser so it can gather
  // loopback candidates instead of restricting capture to the default LAN route.
  'permissions.default.microphone': 1, 'permissions.default.camera': 1,
  'media.peerconnection.ice.loopback': true, 'media.peerconnection.ice.default_address_only': false,
  'media.peerconnection.ice.obfuscate_host_addresses': false,
  'media.volume_scale': '0.0', 'dom.webnotifications.enabled': false,
} } : { headless: true };

export function contextOptions(width = device.viewport.width, height = device.viewport.height, tls = false) {
  const { defaultBrowserType, ...options } = device;
  return { ...options, viewport: { width, height }, hasTouch: width < 1000,
    ...(engine === 'firefox' ? {} : { isMobile: width < 768 }),
    ...(engine === 'chromium' ? { permissions: ['microphone', 'camera'] } : {}),
    ignoreHTTPSErrors: tls };
}
