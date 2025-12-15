import { defineConfig, devices } from '@playwright/test';

const PLAYWRIGHT_PORT = Number(process.env.PLAYWRIGHT_PORT || 3100);
const PLAYWRIGHT_BASE_URL = process.env.WEBSITE_BASE_URL || `http://localhost:${PLAYWRIGHT_PORT}`;

export default defineConfig({
  testDir: './tests',
  timeout: 60 * 1000,
  expect: { timeout: 10000 },
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: process.env.CI ? 'github' : 'list',
  use: {
    baseURL: PLAYWRIGHT_BASE_URL,
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
  },
  projects: [
    // Desktop browsers
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
    { name: 'firefox', use: { ...devices['Desktop Firefox'] } },
    { name: 'webkit', use: { ...devices['Desktop Safari'] } },
    // Mobile phones
    { name: 'mobile-chrome', use: { ...devices['Pixel 5'] } },
    { name: 'mobile-safari', use: { ...devices['iPhone 12'] } },
    { name: 'iphone-se', use: { ...devices['iPhone SE'] } },
    { name: 'iphone-14', use: { ...devices['iPhone 14'] } },
    { name: 'galaxy-s8', use: { ...devices['Galaxy S8'] } },
    // Tablets
    { name: 'ipad', use: { ...devices['iPad (gen 7)'] } },
    { name: 'ipad-pro', use: { ...devices['iPad Pro 11'] } },
  ],
  webServer: {
    command: `npm run dev -- -p ${PLAYWRIGHT_PORT}`,
    url: PLAYWRIGHT_BASE_URL,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
