// Loading the page's module: the real module as `@poyetik/web-page` builds it, fetched,
// instantiated with the square of the settings; and loading it once.
import { readFileSync } from "node:fs";
import { afterEach, describe, expect, test, vi } from "vitest";
import { SQUARE } from "../src/settings.ts";
import { loadSquare, once } from "../src/square-module.ts";

/** The page's module, as its package builds it. */
const PAGE = readFileSync(new URL(import.meta.resolve("@poyetik/web-page/page.wasm")));

/** Fetch answering with the page's module, or with `status`; records the URLs asked. */
function stubFetch(status = 200): string[] {
  const asked: string[] = [];
  vi.stubGlobal("fetch", (url: string) => {
    asked.push(url);
    return Promise.resolve({ ok: status === 200, status, arrayBuffer: () => Promise.resolve(PAGE.buffer.slice(PAGE.byteOffset, PAGE.byteOffset + PAGE.byteLength)) });
  });
  return asked;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("loadSquare", () => {
  test("loads the page's module with the square of the settings", async () => {
    const asked = stubFetch();
    const square = await loadSquare("page.wasm");
    expect(asked).toStrictEqual(["page.wasm"]);
    expect(square.display(0n)).toHaveLength(SQUARE.width * SQUARE.height * 4);
    expect(square.render(0n)).toHaveLength(SQUARE.width * SQUARE.height * 4);
    // 48 kHz at 60 images a second: 800 frames an image.
    expect(square.imageAt(1600n, 48_000, SQUARE.rate)).toBe(2n);
  });

  test("says which download failed", async () => {
    stubFetch(404);
    await expect(loadSquare("page.wasm")).rejects.toThrow(new Error("cannot load the square from page.wasm (HTTP 404)"));
  });
});

describe("once", () => {
  test("loads once, every call answering the first's promise", async () => {
    let loads = 0;
    const load = once(() => {
      loads += 1;
      return Promise.resolve(loads);
    });
    const first = load();
    expect(load()).toBe(first);
    expect([await first, loads]).toStrictEqual([1, 1]);
  });
});
