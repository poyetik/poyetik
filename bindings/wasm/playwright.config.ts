import { defineConfig, devices } from "@playwright/test";
import { createServer } from "node:net";
import { LOCALES, TIME_ZONES } from "./test/fixtures.ts";

/**
 * A free local port for this run, so that two runs (two worktrees, two packages) never
 * share a server. Chosen once by the main process, then read from the environment by the
 * workers and the server, which it starts.
 */
async function freePort(variable: string): Promise<number> {
  const chosen = process.env[variable];
  if (chosen !== undefined) {
    return Number(chosen);
  }
  const port = await new Promise<number>((resolve, reject) => {
    const server = createServer();
    server.on("error", reject);
    server.listen(0, "localhost", () => {
      const address = server.address();
      server.close(() => {
        resolve(typeof address === "object" && address !== null ? address.port : 0);
      });
    });
  });
  process.env[variable] = String(port);
  return port;
}

/** The port of the static server for this run, shared with the workers through the environment. */
const PORT = await freePort("ENGINE_WASM_E2E_PORT");
/** The base URL of the static server, ending with a slash. */
const URL = `http://localhost:${String(PORT)}/`;


/**
 * The time zone and locale of the `index`-th project, taken in turn from the shared test
 * matrix, so that the projects spread over its values instead of running at the
 * machine's (see the end-to-end test that checks them).
 */
function environment(index: number): { timezoneId: string; locale: string } {
  return { timezoneId: TIME_ZONES[index % TIME_ZONES.length] ?? TIME_ZONES[0], locale: LOCALES[index % LOCALES.length] ?? LOCALES[0] };
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
