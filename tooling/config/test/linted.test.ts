// The policy applies to all the TypeScript of the repository: every tracked file that holds
// TypeScript or JavaScript is linted by the ESLint configuration of its package, except the
// few listed below, each with the reason and what proves it instead.
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { ESLint } from "eslint";
import { expect, test } from "vitest";

/** The files that can hold TypeScript or JavaScript, by extension. */
const SOURCE = /\.(?:ts|tsx|mts|cts|js|jsx|mjs|cjs|astro)$/u;

/**
 * What no ESLint configuration lints, on purpose: a directory (ending with `/`) or a file,
 * relative to the repository root, and why. Each entry must still match a tracked file.
 */
const UNLINTED: ReadonlyMap<string, string> = new Map([
  ["tooling/config/test/policy/", "the policy's fixtures, each breaking a rule on purpose: the policy test lints them"],
  ["tooling/config/test/compiler/", "compiler fixtures, each breaking a setting on purpose: the policy test compiles them"],
  ["bindings/audio-wasm/test/types/", "compiler fixtures, some wrong on purpose: bindings/audio-wasm/test/types.test.ts compiles them"],
  // Their frontmatter only calls the linted code of src/lib (Board's `Props` is declared there).
  ["apps/roadmap/src/components/Board.astro", "an Astro component: `astro check` type-checks it; ESLint reads none until eslint-plugin-astro (#15 step 1)"],
  ["apps/roadmap/src/pages/index.astro", "an Astro page: `astro check` type-checks it; ESLint reads none until eslint-plugin-astro (#15 step 1)"],
]);

/** The root of the repository, found by walking up: the tests also run from a copy of the package, deeper in it. */
function repository(): string {
  for (let directory = import.meta.dirname; directory !== path.dirname(directory); directory = path.dirname(directory)) {
    if (existsSync(path.join(directory, "pnpm-workspace.yaml"))) {
      return directory;
    }
  }
  throw new Error("repository root not found");
}

/** Whether `file` is one of the {@link UNLINTED} entries. */
function exempt(file: string): boolean {
  return [...UNLINTED.keys()].some((entry) => (entry.endsWith("/") ? file.startsWith(entry) : file === entry));
}

test("every tracked TypeScript or JavaScript file is linted by its package's configuration", async () => {
  const root = repository();
  const tracked = execFileSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8" }).split("\0");
  const packages = tracked.filter((file) => path.basename(file) === "eslint.config.js").map((file) => path.dirname(file));
  expect(packages.length).toBeGreaterThan(1);
  const sources = tracked.filter((file) => SOURCE.test(file));
  // Every exception is current: it still matches a tracked file.
  for (const entry of UNLINTED.keys()) {
    expect(
      sources.some((file) => (entry.endsWith("/") ? file.startsWith(entry) : file === entry)),
      entry,
    ).toBe(true);
  }
  const linters = new Map(packages.map((directory) => [directory, new ESLint({ cwd: path.join(root, directory) })]));
  const unlinted: string[] = [];
  for (const file of sources.filter((source) => !exempt(source))) {
    // The package is the deepest directory above the file that has its own configuration.
    const owner = packages.filter((directory) => file.startsWith(`${directory}/`)).sort((left, right) => right.length - left.length)[0];
    const eslint = owner === undefined ? undefined : linters.get(owner);
    const absolute = path.join(root, file);
    if (eslint === undefined || (await eslint.isPathIgnored(absolute)) || (await eslint.calculateConfigForFile(absolute)) === undefined) {
      unlinted.push(file);
    }
  }
  expect(unlinted).toStrictEqual([]);
});
