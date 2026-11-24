import { afterEach, assert, expect, test, vi } from "vitest";
import { fetchBytes } from "../src/fetch.ts";

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

test.each([200, 204, 299])("returns the body of a %i, asked with the options given", async (status) => {
  const asked = answering(status);
  const init = { signal: new AbortController().signal };
  expect(new TextDecoder().decode(await fetchBytes("the engine", "engine.wasm", init))).toBe(status === 204 ? "" : "body");
  expect(asked).toStrictEqual([["engine.wasm", init]]);
});

test.each([404, 500, 300, 199])("names what, the URL and the status of a %i", async (status) => {
  // The Response constructor refuses statuses below 200: a plain object stands in for them.
  vi.stubGlobal("fetch", () => Promise.resolve({ ok: false, status }));
  await expect(fetchBytes("the engine", "engine.wasm", { signal: new AbortController().signal })).rejects.toThrow(new Error(`cannot load the engine from engine.wasm (HTTP ${String(status)})`));
});

test("names what and the URL of a request that fails, its error the cause, worded once", async () => {
  const refused = new TypeError("Failed to fetch");
  vi.stubGlobal("fetch", () => Promise.reject(refused));
  const [message, cause] = await refusal();
  expect(message).toBe("cannot load the engine from engine.wasm");
  expect(cause, "its own error").toBe(refused);
});

test("names what and the URL of a body that cannot be read, its error the cause (sixth deep review of visual block 2)", async () => {
  const cut = new TypeError("network error");
  vi.stubGlobal("fetch", () => Promise.resolve({ ok: true, status: 200, arrayBuffer: () => Promise.reject(cut) }));
  const [message, cause] = await refusal();
  expect(message).toBe("cannot load the engine from engine.wasm");
  expect(cause, "its own error").toBe(cut);
});

/** What `fetchBytes` rejected with, which must be an `Error`: its message and its cause. */
async function refusal(): Promise<readonly [string, unknown]> {
  const error: unknown = await fetchBytes("the engine", "engine.wasm", { signal: new AbortController().signal }).then(
    () => undefined,
    (reason: unknown) => reason,
  );
  assert(error instanceof Error, "an Error");
  return [error.message, error.cause];
}
