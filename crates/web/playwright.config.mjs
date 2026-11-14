import { defineConfig } from '@playwright/test';
import { engines } from './tests/engines.mjs';
const evidence=process.env.STICKTALK_ENGINE_EVIDENCE_ROOT ?? 'screenshots/engines/latest';

export default defineConfig({
  testDir: './tests/cross-engine',
  timeout: 10 * 60 * 1000,
  workers: 1,
  retries: 0,
  repeatEach: 3,
  globalSetup: './tests/cross-engine/setup.mjs',
  outputDir: './test-results/engines',
  reporter: [['list'], ['json', { outputFile: `${evidence}/results.json` }]],
  projects: Object.entries(engines).map(([name, { device }]) => {
    const { defaultBrowserType, ...options } = device;
    return { name, use: { ...options, browserName: name, headless: true } };
  }),
});
