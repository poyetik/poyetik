// Loading the page module: the real module as `@poyetik/web-page` builds it, fetched,
// instantiated with the square of the settings; and loading it once.
import { readFileSync } from "node:fs";
import { afterEach, describe, expect, test, vi } from "vitest";
import { COARSE_CLOCK_FROM, RENDER_QUANTUM } from "../src/audio-clock.ts";
import { SQUARE } from "../src/settings.ts";
import { SAMPLE_RATES } from "./matrix.ts";
import { SquareLoadError, loadSquare, once } from "../src/square-module.ts";

/** The page module, as its package builds it. */
const PAGE = readFileSync(new URL(import.meta.resolve("@poyetik/web-page/page.wasm")));

/** Fetch answering with the page module, or with `status`; records the URLs asked. */
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
  test("loads the page module with the square of the settings", async () => {
    const asked = stubFetch();
    const square = await loadSquare("page.wasm");
    expect(asked).toStrictEqual(["page.wasm"]);
    expect(square.display(0n)).toHaveLength(SQUARE.width * SQUARE.height * 4);
    expect(square.render(0n)).toHaveLength(SQUARE.width * SQUARE.height * 4);
    // 48 kHz at 60 images a second: 800 frames an image.
    expect(square.imageAt(1600n, 48_000, SQUARE.rate)).toBe(2n);
  });

  test("shows at most one image off for a clock a browser coarsens to 1 ms, from its boundary on", async () => {
    stubFetch();
    const square = await loadSquare("page.wasm");
    // The boundary: a render quantum lasts exactly 1 ms at 128 kHz.
    expect([COARSE_CLOCK_FROM, RENDER_QUANTUM]).toStrictEqual([128_000, 128]);
    // Firefox's clock, rounded to 1 ms from the boundary on, gives a frame at most rate × 1 ms
    // off (frameAt): 128 frames at the boundary (exactly an image at 1000 images a second),
    // rate / 1000 at every rate of the matrix above it, no more than an image lasts at every
    // rate up to 1000 images a second, so the image shown is at most one off.
    const coarse = [COARSE_CLOCK_FROM, ...SAMPLE_RATES.filter((rate) => rate > COARSE_CLOCK_FROM)];
    expect(coarse.length, "the boundary and the matrix's rates above it").toBeGreaterThan(1);
    for (const sampleRate of coarse) {
      const off = BigInt(sampleRate / 1000);
      for (const rate of [SQUARE.rate, [1000, 1], [30_000, 1001], [1, 1]] as const) {
        for (const frame of [off, 1_000_000n, 123_456_789n, 2n ** 40n + 7n]) {
          const shown = square.imageAt(frame, sampleRate, rate);
          for (const reported of [frame - off, frame + off]) {
            const image = square.imageAt(reported, sampleRate, rate);
            const apart = image > shown ? image - shown : shown - image;
            expect(apart <= 1n, `frame ${String(frame)} at ${String(sampleRate)} Hz, ${String(rate)}: image ${String(image)} for ${String(shown)}`).toBe(true);
          }
        }
      }
    }
  });

  test("says which stage failed: the download, caused by what refused it, or the module", async () => {
    stubFetch(404);
    const download = await loadSquare("page.wasm").catch((error: unknown) => error);
    expect(download).toBeInstanceOf(SquareLoadError);
    expect(download instanceof SquareLoadError ? [download.stage, download.message, download.cause] : undefined).toStrictEqual(["download", "the square's module did not download", new Error("cannot load the square from page.wasm (HTTP 404)")]);
    // Bytes that are no WebAssembly module: the module's own stage.
    vi.stubGlobal("fetch", () => Promise.resolve({ ok: true, status: 200, arrayBuffer: () => Promise.resolve(new Uint8Array([1, 2, 3]).buffer) }));
    const module = await loadSquare("page.wasm").catch((error: unknown) => error);
    expect(module instanceof SquareLoadError ? [module.stage, module.message, module.cause instanceof Error] : undefined).toStrictEqual(["module", "the square's module cannot run here", true]);
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

test("forgets a load that failed, so that the next call loads again", async () => {
  let loads = 0;
  const load = once(() => {
    loads += 1;
    return loads === 1 ? Promise.reject(new Error("offline")) : Promise.resolve(loads);
  });
  await expect(load()).rejects.toThrow("offline");
  await expect(load()).resolves.toBe(2);
  await expect(load(), "a success kept").resolves.toBe(2);
  expect(loads).toBe(2);
});
