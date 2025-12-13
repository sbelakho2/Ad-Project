import { defineConfig, devices } from '@playwright/test';

// Hetzner infrastructure (App: 46.62.252.96, DB: 77.42.44.35)
export default defineConfig({
  testDir: './tests',
  timeout: 60 * 1000,
  expect: { timeout: 10000 },
  fullyParallel: false,
  workers: 1,
  use: {
    baseURL: 'https://console.apexmediation.ee',
    trace: 'on-first-retry',
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
  ],
});
