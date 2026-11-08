import { defineConfig, devices } from "@playwright/test";
import { freePort, inTurn } from "@poyetik/config/playwright";
import { LOCALES, TIME_ZONES } from "./test/matrix.ts";

/** Port of the preview server for this run, shared by the main process and its workers. */
const PORT = await freePort("PLAYGROUND_E2E_PORT", "localhost");
/** Root URL of the preview server, the base of every page the tests open. */
const URL = `http://localhost:${String(PORT)}/`;

/**
 * The time zone and locale of the `index`-th project, taken in turn from the shared test
 * matrix, so that the projects spread over its values instead of running at the
 * machine's (see the end-to-end test that checks them).
 */
function environment(index: number): { timezoneId: string; locale: string } {
  return { timezoneId: inTurn(TIME_ZONES, index), locale: inTurn(LOCALES, index) };
}

// The playground runs in all three browser engines: Chromium, Firefox and Playwright's
// WebKit, at desktop and iPhone sizes; Playwright's WebKit is not Safari (roadmap #18 and #20). It is muted (`?muted`): the tests check what is played, not
// how loud.
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
  webServer: { command: `pnpm exec vite preview --port ${String(PORT)} --strictPort`, url: URL, reuseExistingServer: false },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"], launchOptions: { args: ["--disable-audio-output"] }, ...environment(0) } },
    { name: "firefox", use: { ...devices["Desktop Firefox"], ...environment(1) } },
    { name: "webkit", use: { ...devices["Desktop Safari"], ...environment(2) } },
    { name: "mobile-webkit", use: { ...devices["iPhone 15"], ...environment(3) } },
  ],
});
