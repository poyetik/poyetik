// Loading the page module: the real module as `@poyetik/web-page` builds it, fetched,
// instantiated with the square of the settings; and loading it once.
import { readFileSync } from "node:fs";
import { PageModule, PageModuleError, VisualSetupError } from "@poyetik/web-page/page";
import { afterEach, assert, describe, expect, test, vi } from "vitest";
import { COARSE_CLOCK_FROM, RENDER_QUANTUM } from "../src/audio-clock.ts";
import { SQUARE } from "../src/settings.ts";
import { SAMPLE_RATES } from "./matrix.ts";
import { SQUARE_DOWNLOAD_MS, SquareLoadError, loadSquare, once } from "../src/square-module.ts";

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
  vi.restoreAllMocks();
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

  /** What `loadSquare` rejected with: its class, stage, message and cause, which must be a `SquareLoadError`. */
  async function refusal(url = "page.wasm", environment?: { readonly WebAssembly?: unknown }): Promise<readonly [string, string, string, unknown]> {
    const error = await loadSquare(url, environment).then(
      () => undefined,
      (reason: unknown) => reason,
    );
    assert(error instanceof SquareLoadError, "a SquareLoadError");
    return [error.name, error.stage, error.message, error.cause];
  }

  test("says the download failed, caused by the request, the server's answer or the body, in fetchBytes's words", async () => {
    vi.stubGlobal("fetch", () => Promise.reject(new TypeError("Failed to fetch")));
    expect(await refusal(), "a rejected request").toStrictEqual(["SquareLoadError", "download", "the square's module did not download", new Error("cannot load the square from page.wasm")]);
    stubFetch(404);
    expect(await refusal(), "the server's error").toStrictEqual(["SquareLoadError", "download", "the square's module did not download", new Error("cannot load the square from page.wasm (HTTP 404)")]);
    const cut = new TypeError("network error");
    vi.stubGlobal("fetch", () => Promise.resolve({ ok: true, status: 200, arrayBuffer: () => Promise.reject(cut) }));
    const body = await refusal();
    expect(body, "a rejected body").toStrictEqual(["SquareLoadError", "download", "the square's module did not download", new Error("cannot load the square from page.wasm")]);
    const [, , , error] = body;
    assert(error instanceof Error, "an Error");
    expect(error.cause, "the body's own error, kept").toBe(cut);
  });

  // Finding ME-2 of the seventh deep review of visual block 2 (#34): a body that stalls after
  // its headers left "Loading the square…" up for good.
  test("says the download failed when it outlasts its bound, the request aborted, and leaves no timer once loaded", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      // A body that never comes, but rejects with the signal's reason once it aborts, as a browser's does.
      vi.stubGlobal("fetch", (_url: string, init: { readonly signal: AbortSignal }) => {
        const arrayBuffer = (): Promise<never> =>
          new Promise((_resolve, reject) => {
            init.signal.addEventListener("abort", () => {
              // The bound's reason, an Error: any other leaves the body stalled, and the test with it.
              const reason: unknown = init.signal.reason;
              if (reason instanceof Error) {
                reject(reason);
              }
            });
          });
        return Promise.resolve({ ok: true, status: 200, arrayBuffer });
      });
      const stalled = refusal();
      // Finding PC-4 of the eighth deep review of visual block 2 (#34): not a millisecond early.
      let settled = false;
      void stalled.finally(() => (settled = true));
      await vi.advanceTimersByTimeAsync(SQUARE_DOWNLOAD_MS - 1);
      expect(settled, "still loading a millisecond before the bound").toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      const [name, stage, message, cause] = await stalled;
      expect([name, stage, message, cause]).toStrictEqual(["SquareLoadError", "download", "the square's module did not download", new Error("cannot load the square from page.wasm")]);
      assert(cause instanceof Error, "an Error");
      expect(cause.cause, "the bound's reason").toStrictEqual(new Error("no answer within 30 s"));
      // Finding PC-5: the bound's derivation, the page module twice over a link of 50 kbit/s.
      expect((PAGE.byteLength * 8 / 50_000) * 2, "seconds, twice the download at 50 kbit/s").toBeLessThan(SQUARE_DOWNLOAD_MS / 1000);
      stubFetch();
      await loadSquare("page.wasm");
      expect(vi.getTimerCount(), "the bound cleared").toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  /** Fetch answering 200 with `bytes`. */
  function serving(bytes: Uint8Array): void {
    vi.stubGlobal("fetch", () => Promise.resolve({ ok: true, status: 200, arrayBuffer: () => Promise.resolve(bytes.slice().buffer) }));
  }

  test("says the download failed for bytes that are no module, whatever the server answered: never this browser's fault", async () => {
    for (const [what, bytes] of [
      ["three bytes", new Uint8Array([1, 2, 3])],
      ["a fallback page served 200", new TextEncoder().encode("<!doctype html><title>Not found</title>")],
    ] as const) {
      serving(bytes);
      expect(await refusal(), what).toStrictEqual(["SquareLoadError", "download", "the square's module did not download", new Error("cannot load the square from page.wasm (not a WebAssembly module)")]);
    }
  });

  test("says this browser cannot run the module: no WebAssembly, before any request, or a module it cannot compile", async () => {
    const asked = stubFetch();
    expect(await refusal("page.wasm", {}), "no WebAssembly").toStrictEqual(["SquareLoadError", "module", "the square's module cannot run here", new Error("this browser has no WebAssembly")]);
    expect(asked, "nothing downloaded").toStrictEqual([]);
    // A module's magic and version, then a body this browser cannot compile: its own CompileError, as its cause's cause.
    serving(new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0xff]));
    const [name, stage, message, cause] = await refusal();
    expect([name, stage, message]).toStrictEqual(["SquareLoadError", "module", "the square's module cannot run here"]);
    assert(cause instanceof PageModuleError, "the page module's own error");
    expect(cause.message, "the bytes do not compile").toMatch(/^page module: cannot be compiled: CompileError: /u);
    expect(cause.cause, "the browser's error").toBeInstanceOf(WebAssembly.CompileError);
  });

  test("says the page module failed to set up, a defect, when it refuses the square or is another interface, caused by its error itself", async () => {
    // The real module never refuses the settings' square: only its memory full could, which a
    // test cannot reach through the download, so its refusal is given.
    stubFetch();
    const refused = new VisualSetupError("width and height: image 320x180 does not fit in this machine's memory: 921600 bytes are needed");
    // An interface of another version: a PageModuleError caused by no CompileError.
    const another = new PageModuleError("interface 0x5050_0001, expected 0x5050_0002: build the module again from this source (see: pnpm --filter @poyetik/web-page build:wasm)");
    const instantiate = vi.spyOn(PageModule, "instantiate").mockRejectedValueOnce(refused).mockRejectedValueOnce(another);
    for (const error of [refused, another]) {
      expect(await refusal(), error.message).toStrictEqual(["SquareLoadError", "defect", "the square's module failed to set up", error]);
    }
    expect(instantiate.mock.calls.map(([, square]) => square), "the settings' square").toStrictEqual([SQUARE, SQUARE]);
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
  await expect(load()).rejects.toThrow(new Error("offline"));
  await expect(load()).resolves.toBe(2);
  await expect(load(), "a success kept").resolves.toBe(2);
  expect(loads).toBe(2);
});
