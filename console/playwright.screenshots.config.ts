import { defineConfig, devices } from '@playwright/test';

// Hetzner infrastructure (App: 46.62.252.96, DB: 77.42.44.35)
const BASE_URL = process.env.CONSOLE_BASE_URL || 'https://console.apexmediation.ee';

export default defineConfig({
  testDir: './tests',
  timeout: 60 * 1000,
  expect: { timeout: 10000 },
  workers: 2,
  retries: 1,
  use: {
    baseURL: BASE_URL,
    trace: 'off',
    viewport: { width: 1920, height: 1080 },
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
  ],
  // No webServer - use existing deployed server
  // Only run desktop tests
  grep: /Desktop Views/,
});
