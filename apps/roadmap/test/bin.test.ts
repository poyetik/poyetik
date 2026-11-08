import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emptyBoard, parseBoard, serializeBoard } from "../src/lib/board.ts";

// Tests the real entry point (disk, clock, process) in a temporary directory.
describe("bin", () => {
  let dir: string;
  const argv = process.argv;
  const env = process.env["ROADMAP_FILE"];

  beforeEach(() => {
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
    process.exitCode = undefined;
    vi.restoreAllMocks();
  });

  /** Imports the real entry point afresh with `args` as the command line and returns what it wrote to stdout and stderr, captured instead of printed. */
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
    return { stdout, stderr };
  }

  it("reads and writes the ROADMAP_FILE file with today's date", async () => {
    const file = join(dir, "board.json");
    writeFileSync(file, serializeBoard(emptyBoard("Project")));
    process.env["ROADMAP_FILE"] = file;
    // A fixed clock, a date unlike any default: the test cannot depend on the day it runs.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(2031, 6, 9, 12));
    const { stdout } = await runBin(["add", "Engine"]);
    vi.useRealTimers();
    expect(stdout).toBe("added: #1  later  Engine\n");
    expect(process.exitCode).toBe(0);
    const saved = parseBoard(JSON.parse(readFileSync(file, "utf8")));
    expect(saved.cards[0]?.createdAt).toBe("2031-07-09");
  });

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
    chmodSync(file, 0o444);
    process.env["ROADMAP_FILE"] = file;
    const { stderr } = await runBin(["add", "Engine"]);
    chmodSync(file, 0o644);
    expect(stderr).toBe(`error: cannot write ${file}: EACCES\n`);
    expect(process.exitCode).toBe(1);
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
  it("ignores a closed stdout", async () => {
    await runBin(["help"]);
    expect(() => process.stdout.emit("error", Object.assign(new Error("write EPIPE"), { code: "EPIPE" }))).not.toThrow();
  });
});
