// The testing helpers other packages import (`@poyetik/wasm-build/testing/*`), proven here
// like any source: the hand-assembled modules on what WebAssembly itself accepts and exports,
// and the script runner on a real script of a package written in a temporary directory.
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { afterAll, describe, expect, test } from "vitest";
import { assemble, bump, charge, COUNTER_ENTRY, I32_GLOBAL, leb, name, SCRATCH_ENTRY, section } from "../src/testing/assemble.ts";
import { LIMIT_MS, scriptsOf } from "../src/testing/scripts.ts";

describe("assemble", () => {
  test("encodes LEB128 numbers, names and sections byte for byte", () => {
    expect([leb(0), leb(127), leb(128), leb(624_485)]).toStrictEqual([[0], [127], [0x80, 1], [0xe5, 0x8e, 0x26]]);
    expect(name("ab")).toStrictEqual([2, 0x61, 0x62]);
    expect(section(7, [1, 2])).toStrictEqual([7, 2, 1, 2]);
    expect([COUNTER_ENTRY, I32_GLOBAL, SCRATCH_ENTRY]).toStrictEqual([[0x7e, 1, 0x42, 0, 0x0b], [0x7f, 1, 0x41, 0, 0x0b], [0x7f, 1, 0x41, 0, 0x0b]]);
    expect(bump(3, 5)).toStrictEqual([0x23, 3, 0x42, 5, 0x7c, 0x24, 3]);
    expect(charge(1, 2)).toStrictEqual([0x24, 2, 0x23, 1, 0x23, 2, 0xad, 0x42, 7, 0x7c, 0x42, 3, 0x88, 0x7c, 0x24, 1, 0x23, 2]);
  });

  test("assembles a module WebAssembly accepts, exporting only the functions named and the exports added", () => {
    const module = new WebAssembly.Module(
      assemble({
        functions: [
          { params: 2, result: true, locals: [0], code: [0x20, 0, 0x20, 1, 0x6a, 0x0b], exported: "add" },
          { params: 0, result: false, locals: [0], code: [0x0b] },
        ],
        globals: [I32_GLOBAL],
        exports: [["g", 0x03, 0], ["memory", 0x02, 0]],
        custom: ["note", [1, 2, 3]],
      }),
    );
    expect(WebAssembly.Module.exports(module)).toStrictEqual([
      { name: "add", kind: "function" },
      { name: "g", kind: "global" },
      { name: "memory", kind: "memory" },
    ]);
    expect(WebAssembly.Module.customSections(module, "note").map((contents) => Array.from(new Uint8Array(contents)))).toStrictEqual([[1, 2, 3]]);
    const add: unknown = new WebAssembly.Instance(module).exports["add"];
    expect(typeof add === "function" ? Reflect.apply(add, undefined, [2, 3]) : undefined).toBe(5);
  });

  test("leaves out the export and code sections when asked, and writes the imports and a code trailer given", () => {
    const empty = { params: 0, result: false, locals: [0], code: [0x0b] } as const;
    expect(WebAssembly.validate(assemble({ functions: [empty], exportSection: false }))).toBe(true);
    expect(WebAssembly.validate(assemble({ functions: [empty], codeSection: false })), "functions declared, no bodies").toBe(false);
    expect(WebAssembly.validate(assemble({ functions: [empty], codeTrailer: [0] })), "bytes past the bodies").toBe(false);
    // (import "a" "b" (func (type 0))): the imported function takes index 0.
    const imported = new WebAssembly.Module(assemble({ functions: [empty], imports: [1, ...name("a"), ...name("b"), 0, 0] }));
    expect(WebAssembly.Module.imports(imported)).toStrictEqual([{ module: "a", name: "b", kind: "function" }]);
  });
});

describe("scriptsOf", () => {
  /** A package of the test's own, with one script: it prints its arguments and a variable once its stdin ends, says why its stdout failed, then exits with the code it is given. */
  const root = mkdtempSync(join(tmpdir(), "wasm-build-scripts-"));
  mkdirSync(join(root, "scripts"));
  writeFileSync(
    join(root, "scripts", "probe.mjs"),
    // It reads its stdin to the end first (no input given: an open stdin would hang it), says
    // on stderr why its stdout failed, and writes a character of four bytes in two writes,
    // split in its middle.
    'process.stdout.on("error", (error) => { process.stderr.write(`stdout: ${error.code}\\n`); }); process.stdin.resume(); process.stdin.on("end", () => { process.stdout.write(`${process.argv.slice(2).join(" ")} ${process.env["PROBE"] ?? "unset"}\\n`); const note = Buffer.from("said \u{1F3B5}\\n"); process.stderr.write(note.subarray(0, 7)); process.stderr.write(note.subarray(7)); process.exitCode = Number(process.argv[2] ?? 0); });\n',
  );
  const scripts = scriptsOf(pathToFileURL(`${root}/`));

  afterAll(() => {
    rmSync(root, { recursive: true });
  });

  test("runs a script of the package with its arguments and the environment given, added to the test's", () => {
    expect(scripts.entryPoint("probe.mjs")).toBe(join(root, "scripts", "probe.mjs"));
    expect(scripts.run("probe.mjs", ["3", "x"], { PROBE: "set" })).toStrictEqual({ status: 3, stdout: "3 x set\n", stderr: "said \u{1F3B5}\n" });
    expect(scripts.run("probe.mjs", [], {}), "the test's own environment").toStrictEqual({ status: 0, stdout: ` ${process.env["PROBE"] ?? "unset"}\n`, stderr: "said \u{1F3B5}\n" });
  });

  test("runs a script whose stdout is closed, with no input: its exit code and its stderr, whole", async () => {
    expect(await scripts.runClosed("probe.mjs", ["4"])).toStrictEqual({ status: 4, stderr: "said \u{1F3B5}\nstdout: EPIPE\n" });
  });

  test("bounds every process to half a minute", () => {
    expect(LIMIT_MS).toBe(30_000);
  });
});
