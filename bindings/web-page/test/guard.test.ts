// Every call into the page interface goes through the driver's guard, as every call of the
// visual interface does: the version read included, so that a call failing there is the
// `VisualTrap` `PageModule.create` documents, and a call failing later kills the instance.
// The real module never fails so: its page interface is given, for this file only,
// exports whose function the test replaces.
import { readFileSync } from "node:fs";
import { assert, expect, test, vi } from "vitest";
import { PAGE_MODULE } from "../scripts/module.ts";
import type { Guard, ImageAt } from "../ts/interface.ts";
import { PageModule, VisualTrap } from "../ts/page.ts";

/** The functions of the page interface a test replaces in the real exports. */
const REPLACED = vi.hoisted(() => {
  const replaced: Record<string, unknown> = {};
  return replaced;
});

vi.mock(import("../ts/interface.ts"), async (original) => {
  const real = await original();
  return {
    ...real,
    /**
     * The real page interface, on the real exports with the test's replacements.
     * @param exports The exports of an instance of the page module.
     * @param guard The driver's guard.
     * @returns `page_image_at`, guarded.
     */
    pageInterface: (exports: Readonly<Record<string, unknown>>, guard: Guard): ImageAt => real.pageInterface({ ...exports, ...REPLACED }, guard),
  };
});

/** The page module, as `pnpm build:wasm` built it. */
function bytes(): Buffer<ArrayBuffer> {
  return readFileSync(new URL(`../dist/${PAGE_MODULE.file}`, import.meta.url));
}

/** A valid square. */
const SQUARE = { width: 17, height: 9, rate: [60, 1], side: 39_322, cycle: [1001, 1000] } as const;

test("turns a failed version read into the trap create documents, with its cause", async () => {
  const cause = new RangeError("Maximum call stack size exceeded");
  REPLACED["page_abi_version"] = () => {
    throw cause;
  };
  const failed: unknown = await PageModule.instantiate(bytes(), SQUARE).catch((error: unknown) => error);
  delete REPLACED["page_abi_version"];
  assert(failed instanceof VisualTrap, "a VisualTrap");
  expect([failed.message, failed.cause]).toStrictEqual(["the visual module trapped and cannot be used again", cause]);
});

test("kills the instance when page_image_at fails, never calling it again", async () => {
  const cause = new WebAssembly.RuntimeError("unreachable");
  const calls: bigint[] = [];
  REPLACED["page_image_at"] = (frame: bigint, _rate: number, _numerator: number, _denominator: number) => {
    calls.push(frame);
    throw cause;
  };
  const page = await PageModule.instantiate(bytes(), SQUARE);
  delete REPLACED["page_image_at"];
  let failed: unknown;
  try {
    page.imageAt(5n, 48_000, [60, 1]);
  } catch (error) {
    failed = error;
  }
  assert(failed instanceof VisualTrap, "a VisualTrap");
  expect(failed.cause, "the call's own error").toBe(cause);
  expect(() => page.imageAt(5n, 48_000, [60, 1]), "dead: refused before the call").toThrow(new VisualTrap());
  expect(() => page.render(0n), "the visual interface too").toThrow(new VisualTrap());
  expect(calls, "called once only").toStrictEqual([5n]);
});
