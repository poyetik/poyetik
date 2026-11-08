import { spawn } from "node:child_process";
import type * as Fs from "node:fs";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { addCard, emptyBoard, parseBoard, serializeBoard } from "../src/lib/board.ts";
import { TIME_ZONES, offsetOf } from "./matrix.ts";

/** The output streams the entry point guards, which have no `error` listener outside it. */
const STREAMS = [process.stdout, process.stderr];

// Tests the real entry point (disk, clock, process) in a temporary directory.
describe("bin", () => {
  let dir: string;
  const argv = process.argv;
  const env = process.env["ROADMAP_FILE"];
  const zone = process.env.TZ;

  beforeEach(() => {
    for (const stream of STREAMS) {
      expect(stream.listenerCount("error")).toBe(0);
    }
    dir = mkdtempSync(join(tmpdir(), "roadmap-"));
    vi.resetModules();
    process.exitCode = undefined;
  });

  afterEach(() => {
    rmSync(dir, { recursive: true, force: true });
    process.argv = argv;
    if (env === undefined) {
      delete process.env["ROADMAP_FILE"];
    } else {
      process.env["ROADMAP_FILE"] = env;
    }
    if (zone === undefined) {
      delete process.env.TZ;
    } else {
      process.env.TZ = zone;
    }
    process.exitCode = undefined;
    vi.restoreAllMocks();
    // Each import of the entry point guards the real streams again: drop its listeners.
    for (const stream of STREAMS) {
      stream.removeAllListeners("error");
    }
  });

  /** Imports the real entry point afresh with `args` as the command line and returns what it writes to stdout and stderr, captured instead of printed, then and afterwards. */
  async function runBin(args: string[]): Promise<{ stdout: string; stderr: string }> {
    let stdout = "";
    let stderr = "";
    vi.spyOn(process.stdout, "write").mockImplementation((chunk) => {
      stdout += String(chunk);
      return true;
    });
    vi.spyOn(process.stderr, "write").mockImplementation((chunk) => {
      stderr += String(chunk);
      return true;
    });
    process.argv = ["node", "bin.ts", ...args];
    await import("../src/cli/bin.ts");
    // Getters: the streams can still be written to after the import (stream failures come later).
    return {
      get stdout() {
        return stdout;
      },
      get stderr() {
        return stderr;
      },
    };
  }

  it.each(TIME_ZONES.flatMap((timeZone) => [[timeZone, 0, 30], [timeZone, 23, 30]] as const))(
    "reads and writes the ROADMAP_FILE file with today's local date in %s at %i:%i",
    async (timeZone, hours, minutes) => {
      const file = join(dir, "board.json");
      writeFileSync(file, serializeBoard(emptyBoard("Project")));
      process.env["ROADMAP_FILE"] = file;
      process.env.TZ = timeZone;
      // A fixed clock, a date unlike any default, near midnight where a UTC date would be another day.
      vi.useFakeTimers({ toFake: ["Date"] });
      vi.setSystemTime(new Date(2031, 6, 9, hours, minutes));
      const { stdout } = await runBin(["add", "Engine"]);
      vi.useRealTimers();
      expect(new Date(2031, 6, 9).getTimezoneOffset()).toBe(offsetOf(timeZone, Date.UTC(2031, 6, 9)));
      expect(stdout).toBe("added: #1  later  Engine\n");
      expect(process.exitCode).toBe(0);
      const saved = parseBoard(JSON.parse(readFileSync(file, "utf8")));
      expect(saved.cards[0]?.createdAt).toBe("2031-07-09");
    },
  );

  it("writes errors to stderr with exit code 1", async () => {
    const file = join(dir, "board.json");
    writeFileSync(file, "{}");
    process.env["ROADMAP_FILE"] = file;
    const { stderr } = await runBin(["validate"]);
    expect(stderr).toBe(`error: invalid ${file}: board: expected fields cards, nextId, title, version, got \n`);
    expect(process.exitCode).toBe(1);
  });

  it("uses the package board.json by default", async () => {
    delete process.env["ROADMAP_FILE"];
    const { stdout } = await runBin(["validate"]);
    expect(stdout).toMatch(/^valid board: \d+ card\(s\)\n$/);
    expect(process.exitCode).toBe(0);
  });
  it("refuses an empty ROADMAP_FILE, naming it, and leaves the default board alone", async () => {
    process.env["ROADMAP_FILE"] = "";
    const board = readFileSync(new URL("../board.json", import.meta.url), "utf8");
    const { stdout, stderr } = await runBin(["add", "Never added"]);
    expect(stdout).toBe("");
    expect(stderr).toBe("error: ROADMAP_FILE is set but empty: unset it or name a board file\n");
    expect(process.exitCode).toBe(1);
    expect(readFileSync(new URL("../board.json", import.meta.url), "utf8")).toBe(board);
  });

  it("names the board file it cannot read, without a stack trace", async () => {
    const file = join(dir, "missing.json");
    process.env["ROADMAP_FILE"] = file;
    const { stderr } = await runBin(["list"]);
    expect(stderr).toBe(`error: cannot read ${file}: ENOENT\n`);
    expect(process.exitCode).toBe(1);
  });

  it("names the board file it cannot write", async () => {
    const file = join(dir, "board.json");
    writeFileSync(file, serializeBoard(emptyBoard("Project")));
    process.env["ROADMAP_FILE"] = file;
    // A write the system refuses, whatever the user: a file's permissions do not stop root,
    // so the failure is the system's own error for a path through a file (ENOTDIR), given
    // by the real writeFileSync, for the write of the board only.
    vi.doMock("node:fs", async (original) => {
      const fs = await original<typeof Fs>();
      return {
        ...fs,
        writeFileSync: (_path: string, text: string): void => {
          fs.writeFileSync(join(file, "x"), text);
        },
      };
    });
    const { stderr } = await runBin(["add", "Engine"]);
    vi.doUnmock("node:fs");
    expect(stderr).toBe(`error: cannot write ${file}: ENOTDIR\n`);
    expect(process.exitCode).toBe(1);
    expect(parseBoard(JSON.parse(readFileSync(file, "utf8"))).cards).toEqual([]);
  });

  it("refuses a board that is not valid UTF-8 instead of replacing the bytes", async () => {
    const file = join(dir, "board.json");
    const text = Buffer.from(serializeBoard(emptyBoard("Project")));
    // One byte of the title replaced by 0xFF, which no UTF-8 text contains.
    text[text.indexOf("Project")] = 0xff;
    writeFileSync(file, text);
    process.env["ROADMAP_FILE"] = file;
    const { stdout, stderr } = await runBin(["validate"]);
    expect(stdout).toBe("");
    expect(stderr).toBe(`error: cannot read ${file}: ERR_ENCODING_INVALID_ENCODED_DATA\n`);
    expect(process.exitCode).toBe(1);
  });

  it("exits with 2 on a usage error", async () => {
    const { stderr } = await runBin(["frobnicate"]);
    expect(stderr).toBe("error: unknown command: frobnicate (see: roadmap help)\n");
    expect(process.exitCode).toBe(2);
  });
  /** A stream error as Node reports it, with its system code. */
  function streamError(code: string): Error {
    return Object.assign(new Error(`write ${code}`), { code });
  }

  it("guards both output streams once per run", async () => {
    await runBin(["help"]);
    expect(process.stdout.listenerCount("error")).toBe(1);
    expect(process.stderr.listenerCount("error")).toBe(1);
  });

  it("ignores a closed stdout: the exit code stays the command's", async () => {
    const { stderr } = await runBin(["help"]);
    expect(() => process.stdout.emit("error", streamError("EPIPE"))).not.toThrow();
    expect(process.exitCode).toBe(0);
    expect(stderr).toBe("");
  });

  it("reports another stdout failure as one error line, exit code 1", async () => {
    const output = await runBin(["help"]);
    expect(output.stderr).toBe("");
    expect(() => process.stdout.emit("error", streamError("EIO"))).not.toThrow();
    expect(process.exitCode).toBe(1);
    expect(output.stderr).toBe("error: cannot write to stdout: EIO\n");
  });

  it("never hides a usage error behind a stream failure", async () => {
    await runBin(["frobnicate"]);
    expect(() => process.stderr.emit("error", streamError("EPIPE"))).not.toThrow();
    expect(() => process.stderr.emit("error", streamError("EIO"))).not.toThrow();
    expect(() => process.stdout.emit("error", streamError("EIO"))).not.toThrow();
    expect(process.exitCode).toBe(2);
  });

  it("exits with 0 and says nothing when the real process's reader is gone", async () => {
    // A listing far larger than a pipe's buffer (64 KiB): whenever the read end closes, before or
    // after the first write, a later write meets the closed pipe, with no dependence on timing.
    let board = emptyBoard("Project");
    for (let i = 0; i < 3000; i += 1) {
      board = addCard(board, { title: `A card title long enough to fill the pipe quickly ${String(i)}` }, "2026-11-07").board;
    }
    const file = join(dir, "board.json");
    writeFileSync(file, serializeBoard(board));
    const child = spawn(process.execPath, [fileURLToPath(new URL("../src/cli/bin.ts", import.meta.url)), "list"], {
      env: { ...process.env, ROADMAP_FILE: file },
      stdio: ["ignore", "pipe", "pipe"],
    });
    child.stdout.destroy();
    let stderr = "";
    child.stderr.on("data", (chunk) => {
      stderr += String(chunk);
    });
    const code = await new Promise((resolve) => child.on("close", resolve));
    expect(stderr).toBe("");
    expect(code).toBe(0);
  });
});
