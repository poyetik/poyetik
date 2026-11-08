// The limits the oracle shares with the engine, tied to their source: the Rust constants
// and the corpus README, read as text. A change on either side fails here until the other
// follows.
import { existsSync, readFileSync } from "node:fs";
import { expect, test } from "vitest";
import { MAX_CASE_BYTES, MAX_EXPECTATIONS_BYTES, MAX_REFERENCES_BYTES, MAX_WAV_BYTES } from "../src/cli.ts";
import { MAX_RATE, MIN_RATE } from "../src/inputs.ts";

/** A file of the repository, found by walking up: the tests also run from a copy of the package, deeper in the repository. */
function repositoryText(path: string): string {
  for (let directory = new URL("./", import.meta.url); directory.pathname !== "/"; directory = new URL("../", directory)) {
    const candidate = new URL(path, directory);
    if (existsSync(candidate)) {
      return readFileSync(candidate, "utf8");
    }
  }
  throw new Error(`${path} not found`);
}

/** The value of the Rust constant `name` in `source`, written as a number with `_` separators or a product of such numbers. */
function rustConstant(source: string, name: string): number {
  const match = new RegExp(`^ *pub const ${name}: (?:u32|usize) = ([0-9_]+(?: \\* [0-9_]+)*);$`, "m").exec(source);
  if (match?.[1] === undefined) {
    throw new Error(`no constant ${name}`);
  }
  return match[1].split(" * ").reduce((product, factor) => product * Number(factor.replaceAll("_", "")), 1);
}

test("reads case and references files with engine-cli's limits", () => {
  const cli = repositoryText("crates/engine-cli/src/cli.rs");
  expect(MAX_CASE_BYTES).toBe(rustConstant(cli, "MAX_CASE_BYTES"));
  expect(MAX_REFERENCES_BYTES).toBe(rustConstant(cli, "MAX_REFERENCES_BYTES"));
  expect([MAX_CASE_BYTES, MAX_REFERENCES_BYTES]).toStrictEqual([65_536, 1_048_576]);
});

test("states every limit of its reading in the corpus README", () => {
  expect(repositoryText("conformance/README.md")).toContain(
    "a case file holds at most 64 KiB, a patch file 16 MiB (the limit of the patch format), `references.json` 1 MiB, `expectations.json` 1 MiB, an exported WAV file 4 GiB + 7 bytes (what its 32-bit RIFF size can declare)",
  );
  expect([MAX_EXPECTATIONS_BYTES, MAX_WAV_BYTES]).toStrictEqual([1_048_576, 4_294_967_303]);
});

test("accepts the engine's range of sample rates", () => {
  const rate = repositoryText("crates/engine-core/src/rate.rs");
  expect([MIN_RATE, MAX_RATE]).toStrictEqual([rustConstant(rate, "MIN"), rustConstant(rate, "MAX")]);
});

test("reads the constants it compares", () => {
  expect(rustConstant("    pub const X: usize = 1_000 * 3 * 2;", "X")).toBe(6000);
  expect(() => rustConstant("pub const X: usize = 2 + 2;", "X")).toThrow(new Error("no constant X"));
});
