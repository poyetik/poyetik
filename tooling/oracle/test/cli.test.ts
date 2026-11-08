import { runInNewContext } from "node:vm";
import { describe, expect, test } from "vitest";
import { USAGE, ignoreClosedReader, main, type Io } from "../src/cli.ts";
import { renderHash } from "../src/hash.ts";
import { parseWav } from "../src/wav.ts";
import { makeWav, sineSamples } from "./fixtures.ts";

const WAV = makeWav({ rate: 8000, channels: 1, samples: sineSamples(1, 32, 8000, 1000) });
const CHANNEL = { sine: { frequency: 1000, amplitude: 1, phase: 0 } };

/** Files in memory, and what the command printed. */
function memory(files: Record<string, string | Uint8Array>): Io & { lines: string[]; errors: string[] } {
  const lines: string[] = [];
  const errors: string[] = [];
  const get = (path: string): string | Uint8Array => {
    const file = files[path];
    if (file === undefined) {
      throw new Error(`ENOENT: ${path}`);
    }
    return file;
  };
  return {
    lines,
    errors,
    readText: (path) => {
      const file = get(path);
      return typeof file === "string" ? file : new TextDecoder().decode(file);
    },
    readBytes: (path) => {
      const file = get(path);
      return typeof file === "string" ? new TextEncoder().encode(file) : file;
    },
    list: (dir) =>
      Object.keys(files)
        .filter((path) => path.startsWith(`${dir}/`))
        .map((path) => path.slice(dir.length + 1)),
    out: (line) => lines.push(line),
    err: (line) => errors.push(line),
  };
}

/** A conformance directory with one case, `tone`, and its exported WAV file. */
function corpus(references: Record<string, string>): Record<string, string | Uint8Array> {
  return {
    "conf/cases/tone.json": JSON.stringify({ sample_rate: 8000, frames: 32 }),
    "conf/cases/README.md": "not a case",
    "conf/references.json": JSON.stringify(references),
    "conf/expectations.json": JSON.stringify({ tone: { sample_rate: 8000, frames: 32, channels: [CHANNEL] } }),
    "wav/tone.wav": WAV,
    "wav/notes.txt": "not a WAV file",
  };
}

describe("main", () => {
  test("passes a matching corpus", () => {
    const io = memory(corpus({ tone: renderHash(parseWav(WAV)) }));
    expect(main(["conf", "wav"], io)).toBe(0);
    expect(io.lines).toHaveLength(1);
    expect(io.lines[0]).toMatch(/^ok {7}tone /);
    expect(io.errors).toStrictEqual([]);
  });

  test("fails, with the number of problems, on a mismatch", () => {
    const io = memory(corpus({ tone: "0".repeat(64) }));
    expect(main(["conf", "wav"], io)).toBe(1);
    expect(io.lines[0]).toMatch(/^FAIL {5}tone: hash: /);
    expect(io.errors).toStrictEqual(["error: oracle failed: 1 problem(s)"]);
  });

  test("fails on unreadable or invalid inputs", () => {
    const io = memory({ "conf/cases/tone.json": "{" });
    expect(main(["conf", "wav"], io)).toBe(1);
    expect(io.errors).toHaveLength(1);
    expect(io.errors[0]).toMatch(/^error: conf\/cases\/tone\.json: SyntaxError: /);
    expect(io.lines).toStrictEqual([]);
    const missing = memory({});
    expect(main(["conf", "wav"], missing)).toBe(1);
    expect(missing.errors).toStrictEqual(["error: ENOENT: conf/references.json"]);
  });

  test("reports what is thrown even when it is not an Error of this realm", () => {
    // An Error of another realm is not an `instanceof Error` here.
    const io = { ...memory(corpus({})), list: (): readonly string[] => {
      const thrown: unknown = runInNewContext("throw new Error('no listing')");
      return [String(thrown)];
    } };
    const errors: string[] = [];
    expect(main(["conf", "wav"], { ...io, err: (line) => errors.push(line) })).toBe(1);
    expect(errors).toStrictEqual(["error: Error: no listing"]);
  });

  test("explains its arguments", () => {
    expect(USAGE).toBe("usage: oracle <conformance dir> <dir of exported WAV files>");
  });

  test.each([[[]], [["conf"]], [["conf", "wav", "extra"]]])("prints the usage for %o", (args) => {
    const io = memory({});
    expect(main(args, io)).toBe(2);
    expect(io.errors).toStrictEqual([USAGE]);
  });
});

describe("ignoreClosedReader", () => {
  test("drops a closed reader and lets any other stream error through", () => {
    const listeners: ((error: Error) => void)[] = [];
    ignoreClosedReader({ on: (event: string, listener: (error: Error) => void) => (event === "error" ? listeners.push(listener) : 0) });
    const emit = (error: Error): void => {
      listeners.forEach((listener) => {
        listener(error);
      });
    };
    expect(() => {
      emit(Object.assign(new Error("write EPIPE"), { code: "EPIPE" }));
    }).not.toThrow();
    const full = Object.assign(new Error("no space"), { code: "ENOSPC" });
    expect(() => {
      emit(full);
    }).toThrow(full);
    const plain = new Error("broken");
    expect(() => {
      emit(plain);
    }).toThrow(plain);
  });
});
