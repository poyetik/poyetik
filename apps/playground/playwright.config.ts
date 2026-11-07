import { defineConfig, devices } from "@playwright/test";

// The playground runs in all three browser engines: Chromium, Firefox, WebKit (Safari),
// on desktop and on a phone. It is muted (`?muted`): the tests check what is played, not
// how loud.
// Chromium renders to a fake audio output, which runs the AudioContext normally without
// opening the sound card of the machine. Firefox and WebKit have no such option that keeps
// live audio running: they open the sound card, muted by the pages under test.
export default defineConfig({
  testDir: "e2e",
  forbidOnly: true,
  retries: 0,
  reporter: "list",
  use: { baseURL: "http://localhost:4393/" },
  webServer: { command: "pnpm preview", url: "http://localhost:4393/", reuseExistingServer: false },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"], launchOptions: { args: ["--disable-audio-output"] } } },
    { name: "firefox", use: { ...devices["Desktop Firefox"] } },
    { name: "webkit", use: { ...devices["Desktop Safari"] } },
    { name: "mobile-webkit", use: { ...devices["iPhone 15"] } },
  ],
});
