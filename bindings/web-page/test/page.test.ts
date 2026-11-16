// The page's module as `pnpm build:wasm` built it into dist/: both its interfaces, driven
// through `PageModule`, and the page interface's checks on fake exports.
import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import { PAGE_MODULE } from "../scripts/module.ts";
import { PAGE_ABI_VERSION, pageInterface, PageModuleError, REFUSED } from "../ts/interface.ts";
import { MAX_FRAME, PageModule } from "../ts/page.ts";

/** The module, as `pnpm build:wasm` built it. */
const BYTES = readFileSync(new URL(`../dist/${PAGE_MODULE.file}`, import.meta.url));

/** A valid square. */
const SQUARE = { width: 17, height: 9, rate: [60, 1], side: 39_322, cycle: [1001, 1000] } as const;

describe("PageModule", () => {
  test("drives the visual interface and answers which image the page shows at a frame", async () => {
    const page = await PageModule.create(await WebAssembly.compile(BYTES), SQUARE);
    expect(page.render(7n)).toHaveLength(17 * 9 * 4);
    expect(page.hash(0n, 3)).toMatch(/^[0-9a-f]{64}$/u);
    // 48 000 Hz at 60 images a second: 800 frames an image.
    expect([799n, 800n, 1599n, 1600n].map((frame) => page.imageAt(frame, 48_000, [60, 1]))).toStrictEqual([0n, 1n, 1n, 2n]);
    // Frames from 2^63 on cross the interface as negative i64: read back whole.
    expect(page.imageAt(1n << 63n, 8000, [1000, 1])).toBe(1n << 60n);
    expect(page.imageAt(MAX_FRAME, 8000, [1000, 1])).toBe(MAX_FRAME / 8n);
  });

  test("refuses a rate the module refuses, and values outside its interface", async () => {
    const page = await PageModule.instantiate(BYTES, SQUARE);
    expect(() => page.imageAt(0n, 7999, [60, 1])).toThrow(new PageModuleError("sample rate 7999 Hz or image rate 60/1 refused"));
    expect(() => page.imageAt(0n, 48_000, [2, 4])).toThrow(new PageModuleError("sample rate 48000 Hz or image rate 2/4 refused"));
    expect(() => page.imageAt(-1n, 48_000, [60, 1])).toThrow(new PageModuleError("frame -1 is not from 0 to 2^64 - 1"));
    expect(() => page.imageAt(MAX_FRAME + 1n, 48_000, [60, 1])).toThrow(new PageModuleError("frame 18446744073709551616 is not from 0 to 2^64 - 1"));
    for (const terms of [[48_000.5, 60, 1], [-1, 60, 1], [2 ** 32, 60, 1], [48_000, 60.5, 1], [48_000, 60, -1]] as const) {
      expect(() => page.imageAt(0n, terms[0], [terms[1], terms[2]]), terms.join()).toThrow(new PageModuleError(`${terms.join(", ")}: not unsigned 32-bit integers`));
    }
    // The largest 32-bit terms cross the interface, and the module refuses them.
    expect(() => page.imageAt(0n, 0xffff_ffff, [0xffff_ffff, 1])).toThrow(new PageModuleError("sample rate 4294967295 Hz or image rate 4294967295/1 refused"));
  });

  test("refuses bytes that are not a module, and a module that needs imports", async () => {
    await expect(PageModule.instantiate(Uint8Array.of(1, 2, 3), SQUARE)).rejects.toThrow(/^page module: cannot be compiled: CompileError: /u);
    // (module (import "a" "b" (func))): a valid module that needs an import.
    const needsImports = Uint8Array.of(0x00, 0x61, 0x73, 0x6d, 1, 0, 0, 0, 1, 4, 1, 0x60, 0, 0, 2, 7, 1, 1, 0x61, 1, 0x62, 0, 0);
    await expect(PageModule.instantiate(needsImports, SQUARE)).rejects.toThrow(/^page module: cannot be instantiated with no imports: /u);
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

  test("checks the version, then page_image_at, naming what is wrong", () => {
    for (const [exports, reason] of [
      [{}, "no function page_abi_version"],
      [{ ...fake(0n), page_abi_version: (extra: number) => PAGE_ABI_VERSION + extra }, "page_abi_version takes 1 argument(s), expected 0"],
      [fake(0n, 1), "interface 1, expected 0x50500001: build the module again from this source (pnpm build:wasm)"],
      [{ page_abi_version: () => PAGE_ABI_VERSION }, "no function page_image_at"],
      [{ ...fake(0n), page_image_at: (frame: bigint) => frame }, "page_image_at takes 1 argument(s), expected 4"],
    ] as const) {
      expect(() => pageInterface(exports), reason).toThrow(new PageModuleError(reason));
    }
  });

  test("reads the answer as an unsigned 64-bit integer, and refuses any other", () => {
    expect(pageInterface(fake(-1n))(0n, 0, 0, 0)).toBe(REFUSED);
    expect(pageInterface(fake(5n))(9n, 1, 2, 3)).toBe(5n);
    expect(calls.at(-1), "every argument in its place").toStrictEqual([9n, 1, 2, 3]);
    expect(() => pageInterface(fake(5))(0n, 0, 0, 0)).toThrow(new PageModuleError("page_image_at did not answer a 64-bit integer"));
  });
});

test("names the page module: its crate, what messages call it and its file in dist/", () => {
  expect(PAGE_MODULE).toStrictEqual({ package: "web-page", label: "page module", file: "page.wasm" });
});
