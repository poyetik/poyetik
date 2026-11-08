import { defineConfig, devices } from "@playwright/test";
import { createServer } from "node:net";
import { LOCALES, TIME_ZONES } from "./test/matrix.ts";

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


/** Port of the preview server for this run, shared by the main process and the workers through `ROADMAP_E2E_PORT`. */
const PORT = await freePort("ROADMAP_E2E_PORT");
/** Root URL of the preview server: the base URL of every test and the address Playwright waits on before starting. */
const URL = `http://localhost:${String(PORT)}/`;


/**
 * The time zone and locale of the `index`-th project, taken in turn from the shared test matrix, so that
 * the projects spread over its values instead of running at the machine's (see `e2e/board.spec.ts`).
 */
function environment(index: number): { timezoneId: string; locale: string } {
  return { timezoneId: TIME_ZONES[index % TIME_ZONES.length] ?? TIME_ZONES[0], locale: LOCALES[index % LOCALES.length] ?? LOCALES[0] };
}

// The page is tested on all three engines: Chromium, Firefox, WebKit (Safari).
export default defineConfig({
  testDir: "e2e",
  forbidOnly: true,
  retries: 0,
  reporter: "list",
  use: { baseURL: URL },
  webServer: { command: `pnpm build && astro preview --port ${String(PORT)} --ignore-lock`, url: URL, reuseExistingServer: false },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"], ...environment(0) } },
    { name: "firefox", use: { ...devices["Desktop Firefox"], ...environment(1) } },
    { name: "webkit", use: { ...devices["Desktop Safari"], ...environment(2) } },
    { name: "mobile-webkit", use: { ...devices["iPhone 15"], ...environment(3) } },
  ],
});
