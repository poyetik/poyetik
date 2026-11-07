import { defineConfig, devices } from "@playwright/test";

// The page is tested on all three engines: Chromium, Firefox, WebKit (Safari).
export default defineConfig({
  testDir: "e2e",
  forbidOnly: true,
  retries: 0,
  reporter: "list",
  use: { baseURL: "http://localhost:4391/" },
  webServer: { command: "pnpm build && astro preview --port 4391 --ignore-lock", url: "http://localhost:4391/", reuseExistingServer: false },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "firefox", use: { ...devices["Desktop Firefox"] } },
    { name: "webkit", use: { ...devices["Desktop Safari"] } },
    { name: "mobile-webkit", use: { ...devices["iPhone 15"] } },
  ],
});
