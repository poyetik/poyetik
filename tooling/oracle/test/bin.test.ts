import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { renderHash } from "../src/hash.ts";
import { parseWav } from "../src/wav.ts";
import { makeWav, sineSamples } from "./fixtures.ts";

// Runs the real entry point (disk, process) on a corpus in a temporary directory.
/** The temporary directory of the current test, created before and removed after each one. */
let dir = "";
/** The real `process.argv`, restored after each test since `runBin` overwrites it. */
const argv = process.argv;

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "oracle-"));
  vi.resetModules();
  process.exitCode = undefined;
});

afterEach(() => {
  // Each run installs its stream listeners again: keep none from one test to the next.
  process.stdout.removeAllListeners("error");
  process.stderr.removeAllListeners("error");
  rmSync(dir, { recursive: true, force: true });
  process.argv = argv;
  process.exitCode = undefined;
  vi.restoreAllMocks();
});

/** Runs `node src/bin.ts <args>` in this process, capturing its output. */
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
  await import("../src/bin.ts");
  return { stdout, stderr };
}

test("judges a corpus on disk", async () => {
  const wav = makeWav({ rate: 8000, channels: 1, samples: sineSamples(1, 32, 8000, 1000) });
  mkdirSync(join(dir, "conf", "cases"), { recursive: true });
  mkdirSync(join(dir, "wav"));
  writeFileSync(join(dir, "conf", "cases", "tone.json"), JSON.stringify({ sample_rate: 8000, frames: 32 }));
  writeFileSync(join(dir, "conf", "references.json"), JSON.stringify({ tone: renderHash(parseWav(wav)) }));
  writeFileSync(
    join(dir, "conf", "expectations.json"),
    JSON.stringify({ tone: { sample_rate: 8000, frames: 32, channels: [{ sine: { frequency: 1000, amplitude: 1, phase: 0 } }] } }),
  );
  writeFileSync(join(dir, "wav", "tone.wav"), wav);
  const { stdout, stderr } = await runBin([join(dir, "conf"), join(dir, "wav")]);
  expect(stdout).toMatch(/^ok {7}tone [0-9a-f]{64} \(1 channel\(s\), sine error .+\)\n$/);
  expect(stderr).toBe("");
  expect(process.exitCode).toBe(0);
});

test("fails with its message on stderr", async () => {
  const { stdout, stderr } = await runBin([join(dir, "missing"), dir]);
  expect(stdout).toBe("");
  expect(stderr).toMatch(/ENOENT.*\n$/);
  expect(process.exitCode).toBe(1);
});

test("refuses a case file that is not UTF-8, with one error line naming it", async () => {
  mkdirSync(join(dir, "conf", "cases"), { recursive: true });
  mkdirSync(join(dir, "wav"));
  const file = join(dir, "conf", "cases", "tone.json");
  const text = new TextEncoder().encode(JSON.stringify({ sample_rate: 8000, frames: 32 }));
  text[1] = 0xff;
  writeFileSync(file, text);
  const { stdout, stderr } = await runBin([join(dir, "conf"), join(dir, "wav")]);
  expect(stdout).toBe("");
  expect(stderr).toBe(`error: ${file}: TypeError: The encoded data was not valid for encoding utf-8\n`);
  expect(process.exitCode).toBe(1);
});

test("rejects a usage error with one error line and exit code 2", async () => {
  const { stdout, stderr } = await runBin(["conf"]);
  expect(stdout).toBe("");
  expect(stderr).toBe("error: expected <conformance dir> <dir of exported WAV files>, got 1 argument(s) (see: oracle help)\n");
  expect(process.exitCode).toBe(2);
});

test("prints its usage on stdout when asked", async () => {
  const { stdout, stderr } = await runBin(["help"]);
  expect(stdout).toBe("usage: oracle <conformance dir> <dir of exported WAV files>\n       oracle help\n");
  expect(stderr).toBe("");
  expect(process.exitCode).toBe(0);
});

test("ignores a closed stdout", async () => {
  await runBin([]);
  expect(() => process.stdout.emit("error", Object.assign(new Error("write EPIPE"), { code: "EPIPE" }))).not.toThrow();
  expect(process.exitCode).toBe(2);
});

test("ignores a broken stderr, keeping the exit code of a usage error", async () => {
  await runBin([]);
  expect(() => process.stderr.emit("error", Object.assign(new Error("write EPIPE"), { code: "EPIPE" }))).not.toThrow();
  expect(process.exitCode).toBe(2);
});

test("reports any other stdout error as one error line and exit code 1", async () => {
  let stderr = "";
  await runBin(["help"]);
  vi.spyOn(process.stderr, "write").mockImplementation((chunk) => {
    stderr += String(chunk);
    return true;
  });
  expect(() => process.stdout.emit("error", Object.assign(new Error("write EIO"), { code: "EIO" }))).not.toThrow();
  expect(stderr).toBe("error: cannot write to stdout: EIO\n");
  expect(process.exitCode).toBe(1);
});

test("never hides the exit code of a usage error behind a stdout error", async () => {
  await runBin([]);
  expect(() => process.stdout.emit("error", Object.assign(new Error("write EIO"), { code: "EIO" }))).not.toThrow();
  expect(process.exitCode).toBe(2);
});
