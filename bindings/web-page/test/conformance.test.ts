// The page module, the module a page loads, held to the visual corpus as the visual
// library's own module is (`@poyetik/visual-wasm/testing/conformance`): it is a module of
// its own, built from its own crate with its own dependencies and memory, so that the
// visual module's conformance proves nothing of its bits. And image for image, the same
// values, display bytes and hashes as the visual module.
import { readdirSync } from "node:fs";
import { type Exports, type SquareSetup, VisualDriver } from "@poyetik/visual-wasm/driver";
import { caseHashes, type Drive, encodedCorpus, firstDifference, largestImageIn, visualConformance } from "@poyetik/visual-wasm/testing/conformance";
import { VisualModule } from "@poyetik/visual-wasm/visual";
import { expect, test } from "vitest";
import { MAX_INDEX, PageModule, VisualSetupError } from "../ts/page.ts";
import { pageBytes, visualBytes } from "./modules.ts";

/** The visual conformance directory. */
const CONFORMANCE = visualConformance(new URL(import.meta.url));

/** A driver of the page module's visual interface the tests build on an instance's exports: `PageModule` creates the instance it drives. */
class PageTestDriver extends VisualDriver {
  /**
   * The driver of `exports`, with `square` set up.
   * @param exports The exports of an instance of the page module.
   * @param square The square to set up.
   */
  // A public constructor is the point: the base class's is protected.
  public constructor(exports: Exports, square: SquareSetup) {
    super(exports, square);
  }
}

/** Drives an instance of the page module through the tests' own driver. */
const drive: Drive = (exports, square) => new PageTestDriver(exports, square);

test("gives every case its reference hash, as the native renderer does", async () => {
  const { references, hashes } = await caseHashes(CONFORMANCE, pageBytes(), (module, square) => PageModule.instantiate(module, square));
  expect([...hashes.keys()], "every case of the corpus").toHaveLength(readdirSync(new URL("cases/", CONFORMANCE)).filter((file) => file.endsWith(".json")).length);
  expect(hashes).toStrictEqual(references);
  // As the visual module's: the image of 2^25 pixels most of its time; a limit, not a
  // tolerance on what the test checks.
}, 60_000);

test("encodes the display corpus to the bytes of the native encoding, every pixel", async () => {
  const read = await encodedCorpus(CONFORMANCE, pageBytes(), (module, square) => PageModule.instantiate(module, square));
  expect(read.corpus.values.length, "the corpus's pixels").toBe(3504 * 4);
  expect(firstDifference(read)).toBe("");
});

test("refuses an image its memory cannot hold, naming its size, never a trap, and keeps working", async () => {
  // Within 8000 pages (500 MiB) of the most a 32-bit module addresses: the largest image's
  // values, 512 MiB, cannot fit.
  expect(await largestImageIn(pageBytes(), drive, 8000)).toStrictEqual({
    refused: new VisualSetupError("width and height: image 16384x2048 does not fit in this machine's memory: 536870912 bytes are needed"),
    held: [3 * 2 * 4, 3 * 2 * 4, 4, 64],
    changed: [],
    again: [5 * 1 * 4, 5 * 1 * 4],
    left: 8000,
  });
  // Within 9000 pages (562.5 MiB): its values fit, its display bytes, 128 MiB more, do not.
  expect(await largestImageIn(pageBytes(), drive, 9000)).toStrictEqual({
    refused: new VisualSetupError("width and height: the display encoding of image 16384x2048 does not fit in this machine's memory: 134217728 bytes are needed"),
    held: [3 * 2 * 4, 3 * 2 * 4, 4, 64],
    changed: [],
    again: [5 * 1 * 4, 5 * 1 * 4],
    left: 9000,
  });
});

test("gives the same values, display bytes and hashes as the visual module, image for image", async () => {
  const square = { width: 17, height: 9, rate: [60, 1], side: 39_322, cycle: [1001, 1000] } as const;
  const [fromPage, fromVisual] = [await PageModule.instantiate(pageBytes(), square), await VisualModule.instantiate(visualBytes(), square)];
  for (const index of [0n, 1n, 1n << 32n, MAX_INDEX]) {
    const at = `image ${String(index)}`;
    const values = fromPage.render(index);
    expect(new Uint32Array(values.buffer), `${at}: every value's bits`).toStrictEqual(new Uint32Array(fromVisual.render(index).buffer));
    expect(Array.from(fromPage.display(index)), `${at}: its display bytes`).toStrictEqual(Array.from(fromVisual.display(index)));
    expect(fromPage.placement(index), `${at}: its placement`).toStrictEqual(fromVisual.placement(index));
    expect(fromPage.hash(index, 1), `${at}: its hash`).toBe(fromVisual.hash(index, 1));
    expect(values, at).toHaveLength(17 * 9 * 4);
  }
  expect(fromPage.hash(0n, 60), "a second of images").toBe(fromVisual.hash(0n, 60));
});
