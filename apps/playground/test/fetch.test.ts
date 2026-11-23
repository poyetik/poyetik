import { afterEach, assert, expect, test, vi } from "vitest";
import { fetchOk } from "../src/fetch.ts";

afterEach(() => {
  vi.unstubAllGlobals();
});

/** Stubs `fetch` to answer `status`, recording what was asked. */
function answering(status: number): unknown[][] {
  const asked: unknown[][] = [];
  vi.stubGlobal("fetch", (url: unknown, init: unknown) => {
    asked.push([url, init]);
    return Promise.resolve(new Response(status === 204 ? null : "body", { status }));
  });
  return asked;
}

test.each([200, 204, 299])("returns the response of a %i, asked with the options given", async (status) => {
  const asked = answering(status);
  const init = { signal: new AbortController().signal };
  expect((await fetchOk("the engine", "engine.wasm", init)).status).toBe(status);
  expect(asked).toStrictEqual([["engine.wasm", init]]);
});

test.each([404, 500, 300, 199])("names what, the URL and the status of a %i", async (status) => {
  // The Response constructor refuses statuses below 200: a plain object stands in for them.
  vi.stubGlobal("fetch", () => Promise.resolve({ ok: false, status }));
  await expect(fetchOk("the engine", "engine.wasm", {})).rejects.toThrow(new Error(`cannot load the engine from engine.wasm (HTTP ${String(status)})`));
});

test("names what and the URL of a request that fails, its error the cause, worded once", async () => {
  const refused = new TypeError("Failed to fetch");
  vi.stubGlobal("fetch", () => Promise.reject(refused));
  const error: unknown = await fetchOk("the engine", "engine.wasm", {}).then(
    () => undefined,
    (reason: unknown) => reason,
  );
  assert(error instanceof Error, "an Error");
  expect([error.message, error.cause]).toStrictEqual(["cannot load the engine from engine.wasm", refused]);
  expect(error.cause, "the request's own error").toBe(refused);
});
