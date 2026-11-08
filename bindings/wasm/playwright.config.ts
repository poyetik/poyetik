import { defineConfig, devices } from "@playwright/test";
import { freePort, inTurn } from "@poyetik/config/playwright";
import { LOCALES, TIME_ZONES } from "./test/fixtures.ts";

/** The port of the static server for this run, shared with the workers through the environment. */
const PORT = await freePort("ENGINE_WASM_E2E_PORT", "localhost");
/** The base URL of the static server, ending with a slash. */
const URL = `http://localhost:${String(PORT)}/`;

/**
 * The time zone and locale of the `index`-th project, taken in turn from the shared test
 * matrix, so that the projects spread over its values instead of running at the
 * machine's (see the end-to-end test that checks them).
 */
function environment(index: number): { timezoneId: string; locale: string } {
  return { timezoneId: inTurn(TIME_ZONES, index), locale: inTurn(LOCALES, index) };
}

// The engine runs in all three browser engines: Chromium, Firefox, WebKit (Safari),
// on desktop and on a phone.
// Chromium renders to a fake audio output, which runs the AudioContext normally without
// opening the sound card of the machine. Firefox and WebKit have no such option that keeps
// live audio running: they open the sound card, muted by the pages under test.
export default defineConfig({
  testDir: "e2e",
  forbidOnly: true,
  // One browser at a time: on an idle machine, macOS slows down background processes, and
  // browsers run in parallel then timed out at random (live captures, clicks). Serial runs
  // passed every time, idle or not, and the whole suite still takes seconds.
  workers: 1,
  retries: 0,
  reporter: "list",
  use: { baseURL: URL },
  // The test bench, started exactly as the README tells a person to start it.
  webServer: { command: "pnpm -s bench", url: `${URL}e2e/site/index.html`, reuseExistingServer: false },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"], launchOptions: { args: ["--disable-audio-output"] }, ...environment(0) } },
    { name: "firefox", use: { ...devices["Desktop Firefox"], ...environment(1) } },
    { name: "webkit", use: { ...devices["Desktop Safari"], ...environment(2) } },
    { name: "mobile-webkit", use: { ...devices["iPhone 15"], ...environment(3) } },
  ],
});
