// The policy applies to all the TypeScript of the repository: every tracked file that holds
// TypeScript or JavaScript is linted by the ESLint configuration of its package, except the
// few listed below, each with the reason and what proves it instead.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { devNull } from "node:os";
import path from "node:path";
import { ESLint } from "eslint";
import { expect, test } from "vitest";
import { RUNTIME_MESSAGES } from "../eslint.ts";
import { repository } from "./repository.ts";

/**
 * The variables under which a test runs git without the machine's or the user's
 * configuration: a copy of xtask's `HERMETIC_GIT` (`crates/xtask/src/runner.rs`), which its
 * hermetic git gate holds this file to. `GIT_CONFIG_GLOBAL` needs git 2.32 or later.
 */
const HERMETIC_GIT: Readonly<Record<string, string>> = {
  "GIT_CONFIG_NOSYSTEM": "1",
  "GIT_CONFIG_GLOBAL": devNull,
  "GIT_ATTR_NOSYSTEM": "1",
  "XDG_CONFIG_HOME": devNull,
};

/**
 * The variables git sets for a hook that name one repository, never passed on, so that the
 * process running the tests leaks none in: a copy of xtask's `LOCAL_GIT_VARIABLES`, which its
 * hermetic git gate holds this file to.
 */
const LOCAL_GIT_VARIABLES: readonly string[] = [
  "GIT_ALTERNATE_OBJECT_DIRECTORIES",
  "GIT_CONFIG",
  "GIT_CONFIG_PARAMETERS",
  "GIT_CONFIG_COUNT",
  "GIT_OBJECT_DIRECTORY",
  "GIT_DIR",
  "GIT_WORK_TREE",
  "GIT_IMPLICIT_WORK_TREE",
  "GIT_GRAFT_FILE",
  "GIT_INDEX_FILE",
  "GIT_REPLACE_REF_BASE",
  "GIT_PREFIX",
  "GIT_SHALLOW_FILE",
  "GIT_COMMON_DIR",
];

/** The environment of a git a test starts: this process's, without git's local variables, under {@link HERMETIC_GIT}. */
function hermeticGit(): NodeJS.ProcessEnv {
  const kept = Object.entries(process.env).filter(([name]) => !LOCAL_GIT_VARIABLES.includes(name));
  return { ...Object.fromEntries(kept), ...HERMETIC_GIT };
}

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

/** Whether `file` is one of the {@link UNLINTED} entries. */
function exempt(file: string): boolean {
  return [...UNLINTED.keys()].some((entry) => (entry.endsWith("/") ? file.startsWith(entry) : file === entry));
}

test("every tracked TypeScript or JavaScript file is linted by its package's configuration", async () => {
  const root = repository();
  const tracked = execFileSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8", env: hermeticGit() }).split("\0");
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

test("every reviewed pattern of a runtime's message is matched by a test", () => {
  const root = repository();
  const tracked = execFileSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8", env: hermeticGit() }).split("\0");
  const tests = tracked.filter((file) => SOURCE.test(file) && /\.(?:test|spec)\./u.test(file) && !exempt(file)).map((file) => readFileSync(path.join(root, file), "utf8"));
  expect(tests.length).toBeGreaterThan(50);
  expect(RUNTIME_MESSAGES.filter(([pattern]) => !tests.some((text) => text.includes(`/${pattern}/`)))).toStrictEqual([]);
});

test("every rule a lint message cites is a title of CLAUDE.md", () => {
  const cited = [...readFileSync(path.join(repository(), "tooling/config/eslint.ts"), "utf8").matchAll(/\(CLAUDE\.md: ([^)]+)\)/gu)].map(([, title]) => String(title).toLowerCase());
  expect([...new Set(cited)].sort(), "the citations it reads").toStrictEqual(["a kept state is compared by its values", "an error is asserted whole", "never test only at the default", "tests are deterministic", "tests prove presence"]);
  const titles = [...readFileSync(path.join(repository(), "CLAUDE.md"), "utf8").matchAll(/^- \*\*([^*]+)\*\*/gmu)].map(([, title]) => String(title).toLowerCase().replace(/\.$/u, ""));
  expect(cited.filter((title) => !titles.includes(title))).toStrictEqual([]);
});
