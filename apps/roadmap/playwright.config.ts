import { defineConfig, devices } from "@playwright/test";
import { freePort, inTurn } from "@poyetik/config/playwright";
import { LOCALES, TIME_ZONES } from "./test/matrix.ts";

/** Port of the preview server for this run, shared by the main process and the workers through `ROADMAP_E2E_PORT`. */
const PORT = await freePort("ROADMAP_E2E_PORT", "localhost");
/** Root URL of the preview server: the base URL of every test and the address Playwright waits on before starting. */
const URL = `http://localhost:${String(PORT)}/`;

/**
 * The time zone and locale of the `index`-th project, taken in turn from the shared test matrix, so that
 * the projects spread over its values instead of running at the machine's (see `e2e/board.spec.ts`).
 */
function environment(index: number): { timezoneId: string; locale: string } {
  return { timezoneId: inTurn(TIME_ZONES, index), locale: inTurn(LOCALES, index) };
}

// The page is tested on all three engines, Chromium, Firefox and Playwright's WebKit, at desktop
// and iPhone sizes; Playwright's WebKit is not Safari (roadmap #18 and #20).
export default defineConfig({
  testDir: "e2e",
  forbidOnly: true,
  retries: 0,
  reporter: "list",
  use: { baseURL: URL },
  webServer: { command: `pnpm build && ASTRO_TELEMETRY_DISABLED=1 astro preview --port ${String(PORT)} --ignore-lock`, url: URL, reuseExistingServer: false },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"], ...environment(0) } },
    { name: "firefox", use: { ...devices["Desktop Firefox"], ...environment(1) } },
    { name: "webkit", use: { ...devices["Desktop Safari"], ...environment(2) } },
    { name: "mobile-webkit", use: { ...devices["iPhone 15"], ...environment(3) } },
  ],
});
