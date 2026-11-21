// The page module as `pnpm build:wasm` built it into dist/: both its interfaces, driven
// through `PageModule`, and the page interface's checks on fake exports.
import { readFileSync } from "node:fs";
import { assert, describe, expect, test } from "vitest";
import { PAGE_MODULE } from "../scripts/module.ts";
import { type Guard, imageOf, PAGE_ABI_VERSION, pageInterface, PageModuleError, REFUSED } from "../ts/interface.ts";
import { MAX_FRAME, PageModule, type Placement, type ShaderKind, VisualModuleError, VisualSetupError } from "../ts/page.ts";

/**
 * The module, as `pnpm build:wasm` built it: read by each test, so that a wrong file name
 * fails the tests rather than the loading of this file.
 */
function bytes(): Buffer<ArrayBuffer> {
  return readFileSync(new URL(`../dist/${PAGE_MODULE.file}`, import.meta.url));
}

/** A valid square. */
const SQUARE = { width: 17, height: 9, rate: [60, 1], side: 39_322, cycle: [1001, 1000] } as const;

describe("PageModule", () => {
  test("drives the visual interface and answers which image the page shows at a frame", async () => {
    const page = await PageModule.create(await WebAssembly.compile(bytes()), SQUARE);
    expect(page.render(7n)).toHaveLength(17 * 9 * 4);
    expect(page.display(7n)).toHaveLength(17 * 9 * 4);
    expect(page.hash(0n, 3)).toMatch(/^[0-9a-f]{64}$/u);
    // 48 000 Hz at 60 images a second: 800 frames an image.
    expect([799n, 800n, 1599n, 1600n].map((frame) => page.imageAt(frame, 48_000, [60, 1]))).toStrictEqual([0n, 1n, 1n, 2n]);
    // Frames from 2^63 on cross the interface as negative i64: read back whole.
    expect(page.imageAt(1n << 63n, 8000, [1000, 1])).toBe(1n << 60n);
    // A sample rate of 0 crosses the interface (the module refuses it), as does frame 0.
    expect(() => page.imageAt(0n, 0, [60, 1])).toThrow(new PageModuleError("sample rate 0 Hz or image rate 60/1 refused"));
    expect(page.imageAt(MAX_FRAME, 8000, [1000, 1])).toBe(MAX_FRAME / 8n);
  });

  test("refuses a rate the module refuses, and values outside its interface", async () => {
    const page = await PageModule.instantiate(bytes(), SQUARE);
    expect(() => page.imageAt(0n, 7999, [60, 1])).toThrow(new PageModuleError("sample rate 7999 Hz or image rate 60/1 refused"));
    expect(() => page.imageAt(0n, 48_000, [2, 4])).toThrow(new PageModuleError("sample rate 48000 Hz or image rate 2/4 refused"));
    expect(() => page.imageAt(-1n, 48_000, [60, 1])).toThrow(new PageModuleError("frame -1 is not from 0 to 2^64 - 1"));
    expect(() => page.imageAt(MAX_FRAME + 1n, 48_000, [60, 1])).toThrow(new PageModuleError("frame 18446744073709551616 is not from 0 to 2^64 - 1"));
    for (const [terms, message] of [
      [[48_000.5, 60, 1], "sample rate is not an unsigned 32-bit integer: 48000.5"],
      [[-1, 60, 1], "sample rate is not an unsigned 32-bit integer: -1"],
      [[2 ** 32, 60, 1], "sample rate is not an unsigned 32-bit integer: 4294967296"],
      [[48_000, 60.5, 1], "image rate numerator is not an unsigned 32-bit integer: 60.5"],
      [[48_000, 60, -1], "image rate denominator is not an unsigned 32-bit integer: -1"],
    ] as const) {
      expect(() => page.imageAt(0n, terms[0], [terms[1], terms[2]]), message).toThrow(new PageModuleError(message));
    }
    // The largest 32-bit terms cross the interface, and the module refuses them.
    expect(() => page.imageAt(0n, 0xffff_ffff, [0xffff_ffff, 1])).toThrow(new PageModuleError("sample rate 4294967295 Hz or image rate 4294967295/1 refused"));
  });

  test("refuses a square the module refuses, and a value outside 32 bits, each with its own error", async () => {
    await expect(PageModule.instantiate(bytes(), { ...SQUARE, width: 0 })).rejects.toThrow(new VisualSetupError("width and height: invalid image size 0x9: each side must be from 1 to 16384 pixels"));
    await expect(PageModule.instantiate(bytes(), { ...SQUARE, width: 2 ** 32 })).rejects.toThrow(new VisualModuleError("width is not an unsigned 32-bit integer: 4294967296"));
    const page = await PageModule.instantiate(bytes(), SQUARE);
    const before = [page.hash(0n, 3), page.placement(1n)];
    // Every term differs from the square's, so that a setup refused after taking any shows.
    expect(() => { page.setup({ width: 18, height: 10, rate: [30, 1], side: 65_537, cycle: [1, 3] }); }).toThrow(new VisualSetupError("side: invalid square side 65537: at most 65536, a square as tall as the image (a side is in 65536ths of the image height)"));
    expect([page.hash(0n, 3), page.placement(1n)], "the square it had, kept").toStrictEqual(before);
    expect(page.imageAt(1600n, 48_000, [60, 1]), "the page interface still answers").toBe(2n);
  });

  test("refuses bytes that are not a module, and a module that needs imports", async () => {
    const garbage: unknown = await PageModule.instantiate(Uint8Array.of(1, 2, 3), SQUARE).catch((error: unknown) => error);
    assert(garbage instanceof PageModuleError, "a PageModuleError");
    expect(garbage.message).toMatch(/^page module: cannot be compiled: CompileError: /u);
    expect(garbage.cause, "the reason kept").toBeInstanceOf(WebAssembly.CompileError);
    // (module (import "a" "b" (func))): a valid module that needs an import.
    const needsImports = Uint8Array.of(0x00, 0x61, 0x73, 0x6d, 1, 0, 0, 0, 1, 4, 1, 0x60, 0, 0, 2, 7, 1, 1, 0x61, 1, 0x62, 0, 0);
    const unlinked: unknown = await PageModule.instantiate(needsImports, SQUARE).catch((error: unknown) => error);
    assert(unlinked instanceof PageModuleError, "a PageModuleError");
    expect(unlinked.message).toMatch(/^page module: cannot be instantiated with no imports: /u);
    expect(unlinked.cause, "the reason kept: no import object for module a").toBeInstanceOf(TypeError);
    expect(new PageModuleError("x")).toMatchObject({ name: "PageModuleError", message: "page module: x" });
  });
});

describe("pageInterface", () => {
  /** The arguments each fake `page_image_at` received. */
  const calls: unknown[][] = [];

  /** Fake page exports: the version, and `page_image_at` answering `answer`. */
  const fake = (answer: unknown, version: unknown = PAGE_ABI_VERSION): Record<string, unknown> => ({
    page_abi_version: () => version,
    page_image_at: (frame: bigint, rate: number, numerator: number, denominator: number) => {
      calls.push([frame, rate, numerator, denominator]);
      return answer;
    },
  });

  /** What the guard of each test ran: one entry a call. */
  const guarded: string[] = [];

  /** A guard as the driver's: it runs the call, and turns any failure of it into another error, so that what is checked outside it stays itself. */
  const guard: Guard = (call) => {
    guarded.push("call");
    try {
      return call();
    } catch {
      throw new Error("guarded");
    }
  };

  test("checks the version, then page_image_at, naming what is wrong", () => {
    for (const [exports, reason] of [
      [{}, "no function page_abi_version"],
      [{ ...fake(0n), page_abi_version: (extra: number) => PAGE_ABI_VERSION + extra }, "page_abi_version takes 1 argument(s), expected 0"],
      [fake(0n, 1), "interface 0x1, expected 0x50500001: build the module again from this source (pnpm build:wasm)"],
      [fake(0n, 0x5050_0002), "interface 0x50500002, expected 0x50500001: build the module again from this source (pnpm build:wasm)"],
      [{ ...fake(0n), page_abi_version: () => "1" }, "interface 1, expected 0x50500001: build the module again from this source (pnpm build:wasm)"],
      [{ page_abi_version: () => PAGE_ABI_VERSION }, "no function page_image_at"],
      [{ ...fake(0n), page_image_at: (frame: bigint) => frame }, "page_image_at takes 1 argument(s), expected 4"],
    ] as const) {
      expect(() => pageInterface(exports, guard), reason).toThrow(new PageModuleError(reason));
    }
  });

  test("passes the answer on as it is, every argument in its place", () => {
    expect(pageInterface(fake(5), guard)(9n, 1, 2, 3)).toBe(5);
    expect(calls.at(-1), "every argument in its place").toStrictEqual([9n, 1, 2, 3]);
  });

  test("runs every call into the module through the guard, the version read included, and nothing it checks", () => {
    guarded.length = 0;
    expect(() => pageInterface({}, guard), "a missing function, outside the guard").toThrow(new PageModuleError("no function page_abi_version"));
    expect(() => pageInterface(fake(0n, 1), guard), "another version, outside the guard").toThrow(PageModuleError);
    expect(guarded, "the version read alone").toStrictEqual(["call"]);
    const imageAt = pageInterface(fake(7n), guard);
    expect(imageAt(1n, 2, 3, 4)).toBe(7n);
    expect(guarded, "the version read, then the call").toStrictEqual(["call", "call", "call"]);
    const failing = { ...fake(0n), page_abi_version: () => { throw new RangeError("deep"); } };
    expect(() => pageInterface(failing, guard), "a failed version read, the guard's").toThrow(new Error("guarded"));
    const trapping = pageInterface({ ...fake(0n), page_image_at: (_frame: bigint, _rate: number, _numerator: number, _denominator: number) => { throw new RangeError("deep"); } }, guard);
    expect(() => trapping(0n, 1, 2, 3), "a failed call, the guard's").toThrow(new Error("guarded"));
  });
});

describe("imageOf", () => {
  test("reads an answer as an unsigned 64-bit integer, and refuses any other", () => {
    expect(imageOf(-1n)).toBe(REFUSED);
    expect(imageOf(5n)).toBe(5n);
    for (const answer of [5, "5", undefined]) {
      expect(() => imageOf(answer), String(answer)).toThrow(new PageModuleError("page_image_at did not answer a 64-bit integer"));
    }
  });
});

test("names the page module: its crate, what messages call it and its file in dist/", () => {
  expect(PAGE_MODULE).toStrictEqual({ package: "web-page", label: "page module", file: "page.wasm" });
});

describe("the package built to JavaScript", () => {
  test("is what a page imports: the same module driven through the built code, with its types", async () => {
    const { PageModule: Built, MAX_FRAME: builtMaxFrame } = await import("@poyetik/web-page/page");
    const page = await Built.instantiate(bytes(), SQUARE);
    expect(page.imageAt(1600n, 48_000, [60, 1])).toBe(2n);
    expect(page.display(0n)).toHaveLength(17 * 9 * 4);
    expect(builtMaxFrame).toBe(MAX_FRAME);
    expect(import.meta.resolve("@poyetik/web-page/page")).toMatch(/\/dist\/page\.js$/u);
  });
});

describe("what a page names of the visual interface", () => {
  test("is every value the visual library's entry point exports but its module, the same values", async () => {
    const page: Readonly<Record<string, unknown>> = await import("../ts/page.ts");
    const visual: Readonly<Record<string, unknown>> = await import("@poyetik/visual-wasm/visual");
    const pageOwn = new Set(["PageModule", "MAX_FRAME", "PAGE_ABI_VERSION", "PageModuleError"]);
    const shared = Object.entries(page).filter(([name]) => !pageOwn.has(name));
    expect(Object.fromEntries(shared)).toStrictEqual(Object.fromEntries(Object.entries(visual).filter(([name]) => name !== "VisualModule")));
  });

  test("comes from the page's own entry point: the square, its shaders and errors", async () => {
    const page = await import("../ts/page.ts");
    const driver = await import("@poyetik/visual-wasm/driver");
    expect([page.MAX_INDEX, page.SHADERS, page.THRESHOLDS, page.VisualModuleError, page.VisualSetupError, page.VisualTrap]).toStrictEqual([driver.MAX_INDEX, driver.SHADERS, driver.THRESHOLDS, driver.VisualModuleError, driver.VisualSetupError, driver.VisualTrap]);
    const placed: Placement = [0, 1, 2, 3];
    const kind: ShaderKind = "display";
    expect([placed.length, kind]).toStrictEqual([4, "display"]);
  });
});
