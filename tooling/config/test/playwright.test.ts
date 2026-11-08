import { createServer } from "node:net";
import { afterEach, expect, test, vi } from "vitest";
import { freePort, inTurn } from "../playwright.ts";

/** The variable these tests choose a port in: one no real run uses. */
const VARIABLE = "POYETIK_CONFIG_TEST_PORT";

afterEach(() => {
  vi.unstubAllEnvs();
});

test("keeps the port already chosen", async () => {
  vi.stubEnv(VARIABLE, "4321");
  expect(await freePort(VARIABLE, "localhost")).toBe(4321);
  expect(process.env[VARIABLE]).toBe("4321");
});

test("chooses a free port and writes it for the workers", async () => {
  vi.stubEnv(VARIABLE, undefined);
  const port = await freePort(VARIABLE, "localhost");
  expect(Number.isInteger(port) && port > 0 && port <= 65_535).toBe(true);
  expect(process.env[VARIABLE]).toBe(String(port));
  // The same again, from the environment; and the port is free: it can be listened on.
  expect(await freePort(VARIABLE, "localhost")).toBe(port);
  await new Promise<void>((resolve, reject) => {
    const server = createServer();
    server.on("error", reject);
    server.listen(port, "localhost", () => {
      server.close(() => {
        resolve();
      });
    });
  });
});

test("fails on an address of another machine, choosing nothing", async () => {
  vi.stubEnv(VARIABLE, undefined);
  // 192.0.2.1 is reserved for documentation (RFC 5737): never an address of this machine.
  await expect(freePort(VARIABLE, "192.0.2.1")).rejects.toThrow(/EADDRNOTAVAIL/);
  expect(process.env[VARIABLE]).toBeUndefined();
});

test("takes the values in turn", () => {
  const values = ["a", "b", "c"] as const;
  expect([0, 1, 2, 3, 4, 5, 7].map((index) => inTurn(values, index))).toStrictEqual(["a", "b", "c", "a", "b", "c", "b"]);
  expect(inTurn(["only"], 5)).toBe("only");
});
