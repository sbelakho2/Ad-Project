import { defineConfig, devices } from '@playwright/test'

const PORT = Number(process.env.CONTROL_PLANE_E2E_PORT || 4400)
const LOCAL_BASE_URL = `http://localhost:${PORT}`
const REMOTE_BASE_URL = process.env.CONTROL_PLANE_BASE_URL
const BASE_URL = REMOTE_BASE_URL || LOCAL_BASE_URL
const USE_REMOTE_TARGET = Boolean(REMOTE_BASE_URL)

export default defineConfig({
  testDir: './tests',
  timeout: 60_000,
  expect: { timeout: 5_000 },
  use: {
    baseURL: BASE_URL,
    trace: 'on-first-retry',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: USE_REMOTE_TARGET
    ? undefined
    : {
        command: `npm run dev`,
        url: LOCAL_BASE_URL,
        reuseExistingServer: false,
        timeout: 120_000,
        env: {
          PORT: String(PORT),
          NODE_ENV: 'test',

          // Allow starting without an actual DB (we mock dashboard/devices in the browser).
          CONTROL_PLANE_SKIP_DB: 'true',

          // Auth config
          OPERATOR_JWT_SECRET: 'playwright-test-secret-32-chars-minimum-00000000',
          OPERATOR_SESSION_TTL_HOURS: '4',
          OPERATOR_CREDENTIALS: 'playwright@local.test:$2a$10$RgEzgFX1rJdD/RoyiGfEUOh.ippMHjBiYmZ77zEgx2hUe7b8Mgfq6',

          // Avoid device approval email flow for deterministic tests
          DEVICE_APPROVAL_ENABLED: 'false',
          PUBLIC_ORIGIN: LOCAL_BASE_URL,

          // Required by config schema (not used when CONTROL_PLANE_SKIP_DB=true)
          DB_HOST: '127.0.0.1',
          DB_NAME: 'apexmediation',
          DB_USER: 'apex_readonly',
          DB_PASSWORD: 'readonly-password',
          DB_SSL: 'false',

          // Keep CORS disabled for local same-origin UI
          CORS_ORIGIN: '',
        },
      },
})
