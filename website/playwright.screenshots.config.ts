import { defineConfig, devices } from '@playwright/test';

const BASE_URL = process.env.WEBSITE_BASE_URL || 'http://localhost:3100';

export default defineConfig({
  testDir: './tests',
  timeout: 60 * 1000,
  expect: { timeout: 10000 },
  workers: 1,
  use: {
    baseURL: BASE_URL,
    trace: 'off',
    viewport: { width: 1920, height: 1080 },
  },
  projects: [
    { name: 'Desktop Chrome', use: { ...devices['Desktop Chrome'] } },
  ],
  // No webServer - start manually
});
