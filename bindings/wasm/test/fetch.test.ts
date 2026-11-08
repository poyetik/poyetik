import { afterEach, expect, test, vi } from "vitest";
import { fetchOk } from "../e2e/site/fetch.ts";

afterEach(() => {
  vi.unstubAllGlobals();
});

/** Stubs `fetch` to answer `status`, recording the URLs asked. */
function answering(status: number): unknown[] {
  const asked: unknown[] = [];
  vi.stubGlobal("fetch", (url: unknown) => {
    asked.push(url);
    return Promise.resolve(new Response(status === 204 ? null : "body", { status }));
  });
  return asked;
}

test.each([200, 204, 299])("returns the response of a %i", async (status) => {
  const asked = answering(status);
  const url = new URL("http://localhost/dist/engine.wasm");
  expect((await fetchOk(url)).status).toBe(status);
  expect(asked).toStrictEqual([url]);
});

test.each([404, 500, 300])("names the URL and the status of a %i", async (status) => {
  answering(status);
  await expect(fetchOk("/conformance/cases/missing.json")).rejects.toThrow(new Error(`GET /conformance/cases/missing.json: ${String(status)}`));
});
