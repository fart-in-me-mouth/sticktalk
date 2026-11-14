import { defineConfig } from '@playwright/test';
import { engines } from './tests/engines.mjs';

export default defineConfig({
  testDir: './tests/cross-engine',
  timeout: 10 * 60 * 1000,
  workers: 1,
  retries: 0,
  repeatEach: 3,
  outputDir: './test-results/engines',
  reporter: [['list'], ['json', { outputFile: 'screenshots/engines/results.json' }]],
  projects: Object.entries(engines).map(([name, { device }]) => {
    const { defaultBrowserType, ...options } = device;
    return { name, use: { ...options, browserName: name, headless: true } };
  }),
});
