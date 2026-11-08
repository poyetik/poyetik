// Helpers of the Playwright configurations of every package that runs end-to-end tests:
// measured and mutated here, once, where each configuration only calls them.
import { createServer } from "node:net";

/**
 * A free port on `host` for this run, so that two runs (two worktrees, two packages) never
 * share a server: the one in the environment `variable` when it is set (by an earlier call,
 * or by whoever runs the tests), otherwise one the system gives on `host`, written there. So
 * the main process of Playwright chooses it once, and its workers and the server it starts
 * read the same.
 * @throws {Error} what listening on `host` failed with (an address of another machine).
 */
export async function freePort(variable: string, host: string): Promise<number> {
  const chosen = process.env[variable];
  if (chosen !== undefined) {
    return Number(chosen);
  }
  const port = await new Promise<number>((resolve, reject) => {
    const server = createServer();
    server.on("error", reject);
    server.listen(0, host, () => {
      // A TCP server's address is an object with its port.
      const given = Number(Reflect.get(Object(server.address()), "port"));
      server.close(() => {
        resolve(given);
      });
    });
  });
  process.env[variable] = String(port);
  return port;
}

/**
 * The `index`-th of `values`, taken in turn (index modulo their count): so that the
 * projects of a configuration spread over the values of the test matrix (time zones,
 * locales) instead of running at the machine's.
 */
export function inTurn<T>(values: readonly [T, ...T[]], index: number): T {
  return values.reduce((chosen, value, at) => (at === index % values.length ? value : chosen), values[0]);
}
