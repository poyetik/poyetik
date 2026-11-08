import { EventEmitter } from "node:events";
import { runInNewContext } from "node:vm";
import { describe, expect, test } from "vitest";
import { USAGE, guardStreams, main, readJson, type Io } from "../src/cli.ts";
import { renderHash } from "../src/hash.ts";
import { parseWav } from "../src/wav.ts";
import { makeWav, sineSamples } from "./fixtures.ts";

/** A valid mono WAV file: 32 frames of a 1 kHz sine at 8 kHz, matching `CHANNEL`. */
const WAV = makeWav({ rate: 8000, channels: 1, samples: sineSamples(1, 32, 8000, 1000) });
/** The expected signal of the single channel of `WAV`. */
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

  test("explains its arguments on stdout when asked", () => {
    expect(USAGE).toBe("usage: oracle <conformance dir> <dir of exported WAV files>\n       oracle help");
    const io = memory({});
    expect(main(["help"], io)).toBe(0);
    expect(io.lines).toStrictEqual([USAGE]);
    expect(io.errors).toStrictEqual([]);
  });

  test("reads help followed by another argument as two directories", () => {
    const io = memory({});
    expect(main(["help", "wav"], io)).toBe(1);
    expect(io.errors).toStrictEqual(["error: ENOENT: help/references.json"]);
    expect(io.lines).toStrictEqual([]);
  });

  test.each([
    [[], 0],
    [["conf"], 1],
    [["conf", "wav", "extra"], 3],
  ])("rejects %o with one error line and the reason", (args, count) => {
    const io = memory({});
    expect(main(args, io)).toBe(2);
    expect(io.errors).toStrictEqual([`error: expected <conformance dir> <dir of exported WAV files>, got ${String(count)} argument(s) (see: oracle help)`]);
    expect(io.lines).toStrictEqual([]);
  });
});

describe("readJson", () => {
  test("parses a JSON file", () => {
    expect(readJson(memory({ "a.json": '{"x":[1]}' }), "a.json")).toStrictEqual({ x: [1] });
  });

  test("names the file and keeps the parser's error as the cause", () => {
    let thrown: unknown;
    try {
      readJson(memory({ "a.json": "{" }), "a.json");
    } catch (error) {
      thrown = error;
    }
    const failure = thrown instanceof Error ? thrown : undefined;
    expect(failure?.message).toMatch(/^a\.json: SyntaxError: /);
    expect(failure?.cause).toBeInstanceOf(SyntaxError);
  });

  test("refuses bytes that are not UTF-8, naming the file, instead of replacing them", () => {
    // "{}" with an invalid byte inside: a lenient decoder reads it as "{\uFFFD}".
    for (const bytes of [[0x7b, 0xff, 0x7d], [0x7b, 0x7d, 0xc3]]) {
      let thrown: unknown;
      try {
        readJson(memory({ "a.json": new Uint8Array(bytes) }), "a.json");
      } catch (error) {
        thrown = error;
      }
      const failure = thrown instanceof Error ? thrown : undefined;
      expect(failure?.message).toBe("a.json: TypeError: The encoded data was not valid for encoding utf-8");
      expect(failure?.cause).toBeInstanceOf(TypeError);
    }
  });
});

/** Streams that are real event emitters (an `error` nobody listens to throws), and what reached them. */
function streams(): { stdout: EventEmitter; stderr: EventEmitter & { write: (text: string) => boolean }; written: string[]; failures: number[] } {
  const written: string[] = [];
  const failures: number[] = [];
  return {
    stdout: new EventEmitter(),
    stderr: Object.assign(new EventEmitter(), {
      write: (text: string): boolean => {
        written.push(text);
        return true;
      },
    }),
    written,
    failures,
  };
}

/** Guards `target`'s streams, counting the failures reported. */
function guard(target: ReturnType<typeof streams>): void {
  guardStreams(target.stdout, target.stderr, () => {
    target.failures.push(1);
  });
}

describe("guardStreams", () => {
  test("drops a closed stdout silently", () => {
    const target = streams();
    guard(target);
    target.stdout.emit("error", Object.assign(new Error("write EPIPE"), { code: "EPIPE" }));
    expect(target.written).toStrictEqual([]);
    expect(target.failures).toStrictEqual([]);
  });

  test("turns any other stdout error into one error line and a failure", () => {
    const target = streams();
    guard(target);
    target.stdout.emit("error", Object.assign(new Error("write EIO"), { code: "EIO" }));
    expect(target.written).toStrictEqual(["error: cannot write to stdout: EIO\n"]);
    expect(target.failures).toStrictEqual([1]);
    const plain = streams();
    guard(plain);
    plain.stdout.emit("error", new Error("broken"));
    expect(plain.written).toStrictEqual(["error: cannot write to stdout: broken\n"]);
    expect(plain.failures).toStrictEqual([1]);
    const numbered = streams();
    guard(numbered);
    numbered.stdout.emit("error", Object.assign(new Error("odd"), { code: 5 }));
    expect(numbered.written).toStrictEqual(["error: cannot write to stdout: odd\n"]);
  });

  test("ignores a broken stderr: the exit code still tells", () => {
    const target = streams();
    guard(target);
    target.stderr.emit("error", Object.assign(new Error("write EPIPE"), { code: "EPIPE" }));
    target.stderr.emit("error", Object.assign(new Error("write EIO"), { code: "EIO" }));
    expect(target.written).toStrictEqual([]);
    expect(target.failures).toStrictEqual([]);
  });
});
