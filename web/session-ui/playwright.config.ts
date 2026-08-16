import { defineConfig } from 'playwright/test';

export default defineConfig({
  testDir: './tests',
  timeout: 20_000,
  webServer: {
    command: 'pnpm dev --host 127.0.0.1',
    url: 'http://127.0.0.1:5173',
    reuseExistingServer: true,
  },
  use: {
    baseURL: 'http://127.0.0.1:5173',
    viewport: { width: 1440, height: 900 },
    colorScheme: 'dark',
  },
});
