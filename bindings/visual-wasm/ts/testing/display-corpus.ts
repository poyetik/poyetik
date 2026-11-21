// The one reader of the display corpus (`conformance/visual/display.json`, which
// `visual-cli bless` writes) for every TypeScript test that encodes it: this package's
// conformance tests and the browser's GPU in the playground's end-to-end tests
// (`@poyetik/visual-wasm/testing/display-corpus`), so that every target reads it alike, only
// in the form bless writes. Test code, never built into dist/: measured and mutated as the
// package's sources are, so that a reader that accepts what bless never writes fails a test.

/** The display corpus: each pixel's four values, and the four bytes the native encoding gives them. */
export interface DisplayCorpus {
  /** Four values a pixel (red, green, blue, alpha), in the corpus's order. */
  readonly values: Float32Array;
  /** Four bytes a pixel, as the native encoding gives them. */
  readonly bytes: Uint8Array;
}

/**
 * What a file of the visual corpus (`references.json`, `display.json` of
 * `conformance/visual`) that is not as `visual-cli bless` writes it asks: restored as last
 * committed, never from the index, by its path from the repository's root, which runs from
 * any directory of it; or rewritten only when the renders are meant to change, by the bless
 * run from the repository's root; the one spelling of every tool that reads a file only as
 * its bless writes it.
 * @param file The file's name.
 * @returns The remedy, its hints runnable as written.
 */
export function notAsBlessWrites(file: string): string {
  return `not as bless writes it: restore it (see: git checkout HEAD -- :/conformance/visual/${file}), or rewrite it only when the renders are meant to change (see: cargo run -p visual-cli -- bless conformance/visual)`;
}

/** The first two lines of the file, before its pixels. */
const HEAD = '{\n  "pixels": [';

/** The last lines of the file, after its pixels, its final newline included. */
const TAIL = "  ]\n}\n";

/** One pixel's line: its four values' bits then its four bytes, each lowercase hexadecimal. */
const PIXEL = /^ {4}\["([0-9a-f]{8})", "([0-9a-f]{8})", "([0-9a-f]{8})", "([0-9a-f]{8})", "([0-9a-f]{8})"\](,?)$/u;

/**
 * The pixels of the display corpus file `text`, read only exactly as `visual-cli bless`
 * writes it: its head and tail, one line per pixel ending in a comma but the last, `\n`
 * line endings and a final newline, the layout `spec/visual-design.md` (section 9, "Proven
 * across targets") writes out.
 * @param text The file's text.
 * @returns Its pixels' values and bytes.
 * @throws {Error} Naming the first line in another form.
 */
export function displayCorpus(text: string): DisplayCorpus {
  const lines = text.split("\n");
  const pixels = lines.slice(2, -3);
  if (lines.slice(0, 2).join("\n") !== HEAD || lines.slice(-3).join("\n") !== TAIL) {
    throw new Error(`display.json: ${notAsBlessWrites("display.json")}`);
  }
  const bits = new Uint32Array(pixels.length * 4);
  const bytes = new Uint8Array(pixels.length * 4);
  for (const [at, line] of pixels.entries()) {
    const words = PIXEL.exec(line);
    // A comma after every pixel but the last.
    if (words?.[6] !== (at === pixels.length - 1 ? "" : ",")) {
      throw new Error(`display.json, pixel ${String(at)}, ${JSON.stringify(line)}: ${notAsBlessWrites("display.json")}`);
    }
    // Its four values' bits, then its four bytes as one word.
    for (const [word, hex] of words.slice(1, 6).entries()) {
      const value = Number.parseInt(hex, 16);
      if (word < 4) {
        bits[at * 4 + word] = value;
      } else {
        new DataView(bytes.buffer).setUint32(at * 4, value);
      }
    }
  }
  return { values: new Float32Array(bits.buffer), bytes };
}
