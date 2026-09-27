import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  retries: 0,
  use: {
    baseURL: process.env.LIMITWISE_UI_URL ?? "http://127.0.0.1:43121",
    browserName: "chromium",
    trace: "retain-on-failure",
  },
  webServer: process.env.LIMITWISE_UI_URL || process.env.LIMITWISE_UI_BINARY
    ? undefined
    : {
        command: "npm run dev -- --port 43121",
        url: "http://127.0.0.1:43121",
        reuseExistingServer: false,
      },
});
