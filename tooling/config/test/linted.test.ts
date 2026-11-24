// The policy applies to all the TypeScript of the repository: every tracked file that holds
// TypeScript or JavaScript is linted by the ESLint configuration of its package, except the
// few listed below, each with the reason and what proves it instead.
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { devNull, tmpdir } from "node:os";
import path from "node:path";
import { ESLint } from "eslint";
import { expect, onTestFinished, test, vi } from "vitest";
import { RUNTIME_MESSAGES, strict } from "../eslint.ts";
import { citations, uncited, unmatched } from "../gates.ts";
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
function hermeticGit(): Record<string, string | undefined> {
  return { ...withoutLocalGit(), ...HERMETIC_GIT };
}

/** This process's environment without git's local variables. */
function withoutLocalGit(): Record<string, string | undefined> {
  return Object.fromEntries(Object.entries(process.env).filter(([name]) => !LOCAL_GIT_VARIABLES.includes(name)));
}

/**
 * The tracked files present in the repository at `root`. A read of the repository itself keeps
 * the user's configuration (`safe.directory`, without which git refuses a checkout of another
 * owner), which changes nothing `ls-files -z` writes; a tracked file deleted from the disk is
 * none of them, since nothing can read it.
 */
function listed(root: string): readonly string[] {
  const list = (...args: readonly string[]): readonly string[] =>
    execFileSync("git", ["ls-files", "-z", ...args], { cwd: root, encoding: "utf8", env: withoutLocalGit() })
      .split("\0")
      .filter((file) => file !== "");
  const deleted = list("--deleted");
  return list().filter((file) => !deleted.includes(file));
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

test("lists the tracked files present, in a checkout of another owner the user trusts", () => {
  const root = mkdtempSync(path.join(tmpdir(), "linted-"));
  onTestFinished(() => {
    vi.unstubAllEnvs();
    rmSync(root, { recursive: true });
  });
  const files = path.join(root, "a checkout");
  mkdirSync(files);
  for (const file of ["é.ts", "b c.ts", "gone.ts"]) {
    writeFileSync(path.join(files, file), "");
  }
  const git = (...args: readonly string[]): string => execFileSync("git", args, { cwd: files, encoding: "utf8", env: hermeticGit() });
  git("init", "--quiet");
  git("add", ".");
  rmSync(path.join(files, "gone.ts"));
  // Git takes the checkout for another owner's: it reads it only as the user's configuration says.
  // Git 2.37.1 or later (2.35.4 or 2.36.2 on their lines): older gits know neither the variable
  // nor `*`, or refuse the checkout in other words ("unsafe repository").
  for (const [name, value] of Object.entries({ ...HERMETIC_GIT, GIT_TEST_ASSUME_DIFFERENT_OWNER: "1", GIT_DIR: path.join(root, "a hook's") })) {
    vi.stubEnv(name, value);
  }
  // Refused for its owner, not for a stray GIT_DIR (both exit 128): finding ME-9 of the seventh
  // deep review of visual block 2 (#34); its words untranslated, whatever the locale (PC-1 of
  // the eighth).
  const refused = spawnSync("git", ["ls-files"], { cwd: files, env: { ...withoutLocalGit(), LC_ALL: "C" }, encoding: "utf8" });
  expect([refused.status, refused.stderr.includes("detected dubious ownership")]).toStrictEqual([128, true]);
  const config = path.join(root, "gitconfig");
  writeFileSync(config, "[safe]\n\tdirectory = *\n");
  vi.stubEnv("GIT_CONFIG_GLOBAL", config);
  expect(listed(files)).toStrictEqual(["b c.ts", "é.ts"]);
});

test("every tracked TypeScript or JavaScript file is linted by its package's configuration", async () => {
  const root = repository();
  const tracked = listed(root);
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
  const tests = listed(root).filter((file) => SOURCE.test(file) && /\.(?:test|spec)\./u.test(file) && !exempt(file));
  expect(tests.length).toBeGreaterThan(50);
  expect(unmatched(RUNTIME_MESSAGES.map(([pattern]) => pattern), tests.map((file) => [file, readFileSync(path.join(root, file), "utf8")] as const))).toStrictEqual([]);
});

test("every rule a lint message cites is a title of CLAUDE.md", () => {
  const cited = citations(JSON.stringify(strict("", []).map(({ rules }) => rules)));
  expect([...new Set(cited)].sort(), "the citations it reads").toStrictEqual([
    "(CLAUDE.md: a kept state is compared by its values)",
    "(CLAUDE.md: an error is asserted whole)",
    "(CLAUDE.md: never test only at the default)",
    "(CLAUDE.md: never work around a test)",
    "(CLAUDE.md: tests are deterministic)",
    "(CLAUDE.md: tests prove presence)",
  ]);
  expect(uncited(cited, readFileSync(path.join(repository(), "CLAUDE.md"), "utf8"))).toStrictEqual([]);
});
