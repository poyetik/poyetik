import { EventEmitter } from "node:events";
import { runInNewContext } from "node:vm";
import { firstUnsafe } from "@poyetik/config/one-line";
import { describe, expect, test } from "vitest";
import { USAGE, guardStreams, main, readJson, type Io } from "../src/cli.ts";
import { RENDER_FORMATS } from "../src/format.ts";
import { makeWav, once, renders, sineSamples, type Renders } from "./fixtures.ts";

/** Valid mono WAV files of every render format: 32 frames of a 1 kHz sine at 8 kHz, matching `CHANNEL`, and their hashes. */
const tone = once(() => renders(1, 32, 8000, 1000));
/** The reference hashes of `tone`. */
const hashes = (): Renders["hashes"] => tone().hashes;
/** The expected signal of the single channel of `tone`. */
const CHANNEL = { sine: { frequency: 1000, amplitude: 1, phase: 0 } };

/** Files in memory, and what the command printed. */
function memory(files: Record<string, string | Uint8Array>): Io & { lines: string[]; errors: string[]; limits: Map<string, number> } {
  const lines: string[] = [];
  const errors: string[] = [];
  const limits = new Map<string, number>();
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
    limits,
    readBytes: (path, limit) => {
      limits.set(path, limit);
      const file = get(path);
      const bytes = typeof file === "string" ? new TextEncoder().encode(file) : file;
      if (bytes.length > limit) {
        throw new Error(`cannot read ${path}: larger than ${String(limit)} bytes`);
      }
      return bytes;
    },
    list: (dir) =>
      Object.keys(files)
        .filter((path) => path.startsWith(`${dir}/`))
        .map((path) => path.slice(dir.length + 1)),
    out: (line) => lines.push(line),
    err: (line) => errors.push(line),
  };
}

/** A conformance directory with one case, `tone`, and its exported WAV files, one per render format. */
function corpus(references: Record<string, unknown>): Record<string, string | Uint8Array> {
  return {
    "conf/cases/tone.json": JSON.stringify({ sample_rate: 8000, frames: 32 }),
    "conf/cases/README.md": "not a case",
    "conf/references.json": JSON.stringify(references),
    "conf/expectations.json": JSON.stringify({ tone: { sample_rate: 8000, frames: 32, channels: [CHANNEL] } }),
    ...Object.fromEntries([...tone().wavs].map(([format, bytes]) => [`wav/tone.${format}.wav`, bytes])),
    "wav/notes.txt": "not a WAV file",
  };
}

describe("main", () => {
  test("passes a matching corpus", () => {
    const io = memory(corpus({ tone: hashes() }));
    expect(main(["conf", "wav"], io)).toBe(0);
    expect(io.lines).toHaveLength(3);
    RENDER_FORMATS.forEach((format, index) => {
      expect(io.lines[index]).toMatch(new RegExp(`^ok {7}tone ${format} `));
    });
    expect(io.errors).toStrictEqual([]);
  });

  test("fails, with the number of problems, on a mismatch", () => {
    const io = memory(corpus({ tone: { ...hashes(), binary64: "0".repeat(64) } }));
    expect(main(["conf", "wav"], io)).toBe(1);
    expect(io.lines[0]).toMatch(/^FAIL {5}tone binary64: hash: /);
    expect(io.errors).toStrictEqual(["error: oracle failed on conf against wav: 1 problem(s)"]);
  });

  test("names both directories in its summary, escaped", () => {
    const moved = Object.fromEntries(
      Object.entries(corpus({ tone: { ...hashes(), binary32: "0".repeat(64) } })).map(([path, file]) => [path.replace(/^conf\//, "con\u001b[2Jf/").replace(/^wav\//, "wa\nv/"), file]),
    );
    const io = memory(moved);
    expect(main(["con\u001b[2Jf", "wa\nv"], io)).toBe(1);
    expect(io.errors).toStrictEqual(["error: oracle failed on con\\u{1b}[2Jf against wa\\u{a}v: 1 problem(s)"]);
  });

  test("reads each file with the limit of its kind, as engine-cli does", () => {
    const io = memory(corpus({ tone: hashes() }));
    expect(main(["conf", "wav"], io)).toBe(0);
    expect(io.limits).toStrictEqual(
      new Map([
        ["conf/cases/tone.json", 65_536],
        ["conf/references.json", 1_048_576],
        ["conf/expectations.json", 1_048_576],
        ...RENDER_FORMATS.map((format) => [`wav/tone.${format}.wav`, 2 ** 32 + 7] as const),
      ]),
    );
    const large = memory({ ...corpus({ tone: hashes() }), "conf/cases/tone.json": `{"sample_rate":8000,"frames":32}${" ".repeat(65_536)}` });
    expect(main(["conf", "wav"], large)).toBe(1);
    expect(large.errors).toStrictEqual(["error: cannot read conf/cases/tone.json: larger than 65536 bytes"]);
  });

  test("fails on unreadable or invalid inputs", () => {
    const io = memory({ ...corpus({ tone: hashes() }), "conf/cases/tone.json": "{" });
    expect(main(["conf", "wav"], io)).toBe(1);
    expect(io.lines).toHaveLength(3);
    expect(io.lines[0]).toMatch(/^FAIL {5}tone binary64: invalid conformance input: tone: not JSON \(SyntaxError: /);
    const unreadable = memory({ ...corpus({}), "conf/references.json": "{" });
    expect(main(["conf", "wav"], unreadable)).toBe(1);
    expect(unreadable.errors).toHaveLength(1);
    expect(unreadable.errors[0]).toMatch(/^error: conf\/references\.json: SyntaxError: /);
    const missing = memory({});
    expect(main(["conf", "wav"], missing)).toBe(1);
    expect(missing.errors).toStrictEqual(["error: ENOENT: conf/references.json"]);
  });

  test("reads only case names, as engine-cli does: lowercase ASCII letters, digits, dots and hyphens, starting with a letter or a digit", () => {
    for (const name of ["a", "0", "z9", "a.b-c", "sine-1.1754944e-38-at-8k", "9-.", "0.-"]) {
      const io = memory({ ...corpus({ tone: hashes() }), [`conf/cases/${name}.json`]: JSON.stringify({ sample_rate: 8000, frames: 32 }) });
      expect(main(["conf", "wav"], io)).toBe(1);
      expect(io.errors).toStrictEqual(["error: oracle failed on conf against wav: 1 problem(s)"]);
    }
    for (const name of ["A", "sinE", "my case", "-x", ".x", "a_b", "é", "a$(b)", "a\u0000", ".."]) {
      const io = memory({ ...corpus({ tone: hashes() }), [`conf/cases/${name}.json`]: "{}" });
      expect(main(["conf", "wav"], io)).toBe(1);
      expect(io.lines).toStrictEqual([]);
      expect(io.errors).toStrictEqual([
        `error: conf/cases/${name.replace("\u0000", "\\u{0}")}.json: not a case name (lowercase ASCII letters, digits, dots and hyphens, starting with a letter or a digit)`,
      ]);
    }
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
    expect(USAGE).toBe("usage: node src/bin.ts <conformance dir> <dir of exported WAV files>\n       node src/bin.ts help\n(in tooling/oracle)");
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
    expect(io.errors).toStrictEqual([`error: expected <conformance dir> <dir of exported WAV files>, got ${String(count)} argument(s) (see: node src/bin.ts help)`]);
    expect(io.lines).toStrictEqual([]);
  });
});

describe("untrusted text in its output", () => {
  /** Asserts that every line `io` printed holds no character that could drive a terminal or break the line, and is bounded. */
  function expectSafeLines(io: { lines: string[]; errors: string[] }): void {
    for (const line of [...io.lines, ...io.errors]) {
      expect(firstUnsafe(line)).toBeUndefined();
      expect(Array.from(line).length).toBeLessThanOrEqual("error: ".length + 1000);
    }
  }

  test("escapes a references key holding a terminal sequence or a line break, one line per problem", () => {
    const io = memory(corpus({ tone: hashes(), "\u001b[2J\nFAIL     forged": hashes() }));
    expect(main(["conf", "wav"], io)).toBe(1);
    expect(io.lines).toStrictEqual([
      "FAIL     \\u{1b}[2J\\u{a}FAIL     forged: missing case file, expectations",
      expect.stringMatching(/^ok {7}tone binary64 /),
      expect.stringMatching(/^ok {7}tone binary64-as-binary32 /),
      expect.stringMatching(/^ok {7}tone binary32 /),
    ]);
    expect(io.errors).toStrictEqual(["error: oracle failed on conf against wav: 1 problem(s)"]);
  });

  test("escapes and bounds an invalid references key of a million characters", () => {
    const key = `${"k".repeat(1_000_000)}\u202e`;
    const io = memory(corpus({ tone: hashes(), [key]: "x" }));
    expect(main(["conf", "wav"], io)).toBe(1);
    expect(io.lines).toStrictEqual([]);
    // "invalid conformance input: references.json: " and the key, cut at 1,000 characters.
    expect(io.errors).toStrictEqual([`error: invalid conformance input: references.json: ${"k".repeat(955)}…`]);
  });

  test("escapes the excerpt of a file the JSON parser quotes", () => {
    const io = memory({ ...corpus({}), "conf/references.json": "[1,\u001b[31m\u202e]" });
    // V8 quotes the text it could not read: raw bytes of the file.
    expect(() => {
      JSON.parse("[1,\u001b[31m\u202e]");
    }).toThrow("\u001b[31m\u202e");
    expect(main(["conf", "wav"], io)).toBe(1);
    expect(io.errors).toHaveLength(1);
    expect(io.errors[0]).toMatch(/^error: conf\/references\.json: SyntaxError: /);
    expectSafeLines(io);
  });

  test("escapes a chunk id read from a WAV file", () => {
    // A last chunk whose id is ESC [ 2 J and whose size runs past the end of the file.
    const wav = makeWav({ rate: 8000, channels: 1, samples: sineSamples(1, 32, 8000, 1000), trailer: [0x1b, 0x5b, 0x32, 0x4a, 0xff, 0xff, 0, 0] });
    const io = memory({ ...corpus({ tone: hashes() }), "wav/tone.binary32.wav": wav });
    expect(main(["conf", "wav"], io)).toBe(1);
    expect(io.lines[2]).toBe('FAIL     tone binary32: invalid WAV file: chunk "\\u{1b}[2J" extends past the end of the file');
  });
});

describe("readJson", () => {
  test("parses a JSON file", () => {
    expect(readJson(memory({ "a.json": '{"x":[1]}' }), "a.json", 9)).toStrictEqual({ x: [1] });
  });

  test("names the file and keeps the parser's error as the cause", () => {
    let thrown: unknown;
    try {
      readJson(memory({ "a.json": "{" }), "a.json", 9);
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
        readJson(memory({ "a.json": new Uint8Array(bytes) }), "a.json", 9);
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
