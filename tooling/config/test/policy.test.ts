import { readdirSync } from "node:fs";
import path from "node:path";
import { ESLint, type Linter } from "eslint";
import jsdoc from "eslint-plugin-jsdoc";
import ts from "typescript";
import tseslint from "typescript-eslint";
import { expect, test } from "vitest";
import { AUDIO_CLOCK_READERS, JSDOC_CONTEXTS, RULES, strict } from "../eslint.ts";

/** The fixtures of the linting policy: each file but `clean.ts` breaks one rule. */
const POLICY = path.join(import.meta.dirname, "policy");

/** The fixtures of the compiler settings: each file breaks one setting of the base tsconfig. */
const COMPILER = path.join(import.meta.dirname, "compiler");

/** Errors, not warnings: a warning would let the check pass. */
const ERROR = 2;

/** Whether a rule entry of the policy turns its rule off. */
function isOff(entry: Linter.RuleEntry | undefined): boolean {
  const severity = Array.isArray(entry) ? entry[0] : entry;
  return severity === "off" || severity === 0;
}

test("each rule of the policy rejects what it forbids, and only that", async () => {
  const eslint = new ESLint({ cwd: POLICY, overrideConfigFile: true, overrideConfig: strict(POLICY, ["ignored.ts"]) });
  const results = await eslint.lintFiles(["*.ts", "e2e/site/*.ts", "src/cli/*.ts"]);
  const reported = Object.fromEntries(
    results.map((result) => [
      path.basename(result.filePath, ".ts"),
      result.messages.map((message) => message.ruleId ?? "inline").sort(),
    ]),
  );
  const assertions = "@typescript-eslint/consistent-type-assertions";
  const boolean = "@typescript-eslint/strict-boolean-expressions";
  const exhaustive = "@typescript-eslint/switch-exhaustiveness-check";
  const tsComment = "@typescript-eslint/ban-ts-comment";
  const boundary = "@typescript-eslint/explicit-module-boundary-types";
  const documented = "jsdoc/require-jsdoc";
  // Every rule the policy sets, unless it turns it off, is broken by some fixture: a rule
  // added without one, or removed from the policy, fails here or below.
  const broken = new Set(Object.values(reported).flat());
  expect(Object.keys(RULES).filter((rule) => !isOff(RULES[rule]) && !broken.has(rule))).toStrictEqual([]);
  expect(reported).toStrictEqual({
    clean: [],
    types: [],
    assertion: [assertions, "@typescript-eslint/no-unsafe-type-assertion"],
    "non-null": ["@typescript-eslint/no-non-null-assertion"],
    "explicit-any": [boundary, "@typescript-eslint/no-explicit-any"],
    "rest-any": ["@typescript-eslint/no-explicit-any"],
    "ts-ignore": [tsComment],
    "ts-expect-error": [tsComment],
    "ts-nocheck": [tsComment],
    "string-condition": [boolean],
    "number-condition": [boolean],
    "nullable-object-condition": [boolean],
    "switch-union": [exhaustive],
    "switch-non-union": [exhaustive],
    "boundary-types": [boundary],
    enum: ["no-restricted-syntax"],
    global: ["no-restricted-syntax"],
    "type-import": ["@typescript-eslint/consistent-type-imports"],
    // Only inline type specifiers: the import itself stays, as a side effect.
    "inline-type-import": ["@typescript-eslint/no-import-type-side-effects"],
    "mutable-field": ["@typescript-eslint/prefer-readonly"],
    "undocumented-function": [documented],
    "undocumented-exported-const": [documented],
    "undocumented-module-const": [documented],
    "undocumented-type": [documented],
    "undocumented-interface": [documented],
    "undocumented-class": [documented],
    // The property, the getter, the setter and the method; not the constructor (see clean.ts).
    "undocumented-class-members": [documented, documented, documented, documented],
    // A property and a method signature, as members of a class; the members of an inline
    // object type (clean.ts) are not items.
    "undocumented-interface-member": [documented, documented],
    // A constant declared inside a module declaration, as at the top level.
    "undocumented-module-member.d": [documented],
    "empty-description": ["jsdoc/require-description"],
    // The inline comment does not silence the rule, and is reported itself.
    "loose-equality": ["eqeqeq", "inline"],
    "param-mutation": ["no-param-reassign"],
    "param-reassign": ["no-param-reassign"],
    "let-never-reassigned": ["prefer-const"],
    "short-name": ["id-length"],
    shadow: ["@typescript-eslint/no-shadow"],
    var: ["no-var"],
    "with-resolvers": ["no-restricted-properties"],
    // The global, then through globalThis, window and self; the checked helper (fetch.ts) may call it.
    "bare-fetch": ["no-restricted-globals", ...Array<string>(3).fill("no-restricted-properties")],
    fetch: [],
    // A read of an audio clock, plain, optional and destructured; not a write, nor a class's
    // own field; the checked reader (audio-clock.ts) may read it.
    "bare-audio-clock": Array<string>(3).fill("no-restricted-syntax"),
    "audio-clock": [],
    // performance.now, Date.now, process.hrtime and process.uptime, then new Date() and
    // Date(), then the twelve reads through the global object (performance.now, Date.now,
    // new Date() and Date() through each of globalThis, window and self); not new Date(0),
    // directly or through the global object.
    clock: [...Array<string>(4).fill("no-restricted-properties"), ...Array<string>(14).fill("no-restricted-syntax")].sort(),
    // The clock readers (CLOCK_READERS) read a clock, directly or through the global object;
    // the bench page's timing module still may not use what the other restrictions refuse
    // (Promise.withResolvers, an enum, a global augmentation, a skipped test).
    timing: ["no-restricted-properties", ...Array<string>(3).fill("no-restricted-syntax")],
    bin: [],
    // Each formatter given no locale, `undefined` or the empty list, called and constructed:
    // the five methods, localeCompare in each form, the formatters of Intl; then, in a
    // fixture of their own, so that one of them refused cannot pass for one of the refused
    // forms missed, none given a locale, nor a read of the locale (resolvedOptions).
    locale: Array<string>(24).fill("no-restricted-syntax"),
    "locale-named": [],
    // Vitest and Playwright: every focus or skip, at every level.
    "focused-test": ["no-restricted-syntax", "no-restricted-syntax"],
    "skipped-test": Array<string>(5).fill("no-restricted-syntax"),
    "skipped-browser-test": ["no-restricted-syntax", "no-restricted-syntax"],
    // A rejection's reason dropped by a callback that takes none; a leading parameter unused.
    "catch-without-reason": ["no-restricted-syntax"],
    "unused-leading-param": ["@typescript-eslint/no-unused-vars"],
  });
  const severities = results.flatMap((result) => result.messages.filter((message) => message.ruleId !== null).map((message) => message.severity));
  expect(new Set(severities)).toStrictEqual(new Set([ERROR]));
  const messages = results.flatMap((result) => result.messages.map((message) => message.message));
  expect(messages).toContain("Use a union of string literals or an `as const` object.");
  expect(messages).toContain("No global augmentation.");
  expect(messages).toContain("Never focus or skip a test: run it, fix it, or discuss it (CLAUDE.md, testing policy).");
  expect(messages).toContain("Name the locale (CLAUDE.md: never test only at the default): a formatter given none follows the machine's.");
  expect(messages).toContain("Name the reason a rejection carries: say where it goes (a log, a typed result, an error's cause), or drop it by a name starting with `_` and a comment saying why.");
  expect(messages).toContain(
    "'Promise.withResolvers' is restricted from being used. It is missing from browsers the friendly checks still explain (Safari before 17.4, Firefox before 121): use `new Promise`.",
  );
  const fetchMessage = "Fetch through the package's checked helper (its fetch.ts), which refuses a response that is not 2xx and names the URL.";
  expect(messages).toContain(`Unexpected use of 'fetch'. ${fetchMessage}`);
  expect(messages).toContain(`'globalThis.fetch' is restricted from being used. ${fetchMessage}`);
  const clockMessage = "Read no clock (CLAUDE.md: tests are deterministic): only the files of CLOCK_READERS read one.";
  expect(messages).toContain(`'performance.now' is restricted from being used. ${clockMessage}`);
  expect(messages).toContain(`'process.hrtime' is restricted from being used. ${clockMessage}`);
  expect(messages).toContain(`'process.uptime' is restricted from being used. ${clockMessage}`);
  expect(messages).toContain(clockMessage);
  expect(messages).toContain(
    "Read an audio context's clock through the package's checked reader (its audio-clock.ts): a browser may report one that is no time, which every AudioParam refuses.",
  );
  // The package's own ignores, then the outputs every package ignores.
  for (const ignored of ["ignored.ts", "coverage/x.ts", "reports/x.ts", ".stryker-tmp/x.ts", "playwright-report/x.ts", "test-results/x.ts"]) {
    expect(await eslint.isPathIgnored(path.join(POLICY, ignored)), ignored).toBe(true);
  }
  expect(await eslint.isPathIgnored(path.join(POLICY, "clean.ts"))).toBe(false);
});

test("a clock reader keeps exactly the general restrictions but the clock reads", () => {
  const clockMessage = "Read no clock (CLAUDE.md: tests are deterministic): only the files of CLOCK_READERS read one.";
  const readers = strict(POLICY, []).find((config) => config.files?.includes("**/e2e/site/timing.ts") === true);
  for (const rule of ["no-restricted-syntax", "no-restricted-properties"]) {
    const general = RULES[rule];
    const kept = readers?.rules?.[rule];
    expect(Array.isArray(general) && Array.isArray(kept)).toBe(true);
    const unclocked = Array.isArray(general) ? general.filter((entry: unknown) => !JSON.stringify(entry).includes(clockMessage)) : [];
    expect(kept).toStrictEqual(unclocked);
    expect(Array.isArray(general) ? general.length - unclocked.length : 0).toBeGreaterThan(0);
  }
});

test("an audio clock reader keeps exactly the general restrictions but the reads of an audio clock", () => {
  const audioClockMessage = "Read an audio context's clock through the package's checked reader (its audio-clock.ts)";
  expect(AUDIO_CLOCK_READERS).toStrictEqual(["**/audio-clock.ts"]);
  const readers = strict(POLICY, []).find((config) => config.files?.includes("**/audio-clock.ts") === true);
  for (const rule of ["no-restricted-syntax", "no-restricted-properties"]) {
    const general = RULES[rule];
    const kept = readers?.rules?.[rule];
    expect(Array.isArray(general) && Array.isArray(kept)).toBe(true);
    const unclocked = Array.isArray(general) ? general.filter((entry: unknown) => !JSON.stringify(entry).includes(audioClockMessage)) : [];
    expect(kept).toStrictEqual(unclocked);
  }
  const syntax = RULES["no-restricted-syntax"];
  expect(Array.isArray(syntax) ? syntax.filter((entry: unknown) => JSON.stringify(entry).includes(audioClockMessage)).length : 0).toBe(2);
});

test("each JSDoc context of the policy is triggered by a fixture", async () => {
  expect(JSDOC_CONTEXTS.length).toBeGreaterThan(0);
  for (const context of JSDOC_CONTEXTS) {
    // That context alone, without the functions, classes and methods required besides.
    const alone: Linter.Config = {
      files: ["**/*.ts"],
      languageOptions: { parser: tseslint.parser },
      plugins: { jsdoc },
      rules: {
        "jsdoc/require-jsdoc": [
          "error",
          {
            publicOnly: false,
            require: { FunctionDeclaration: false, ClassDeclaration: false, MethodDefinition: false },
            checkConstructors: false,
            checkGetters: false,
            checkSetters: false,
            contexts: [context],
          },
        ],
      },
    };
    const eslint = new ESLint({ cwd: POLICY, overrideConfigFile: true, overrideConfig: alone });
    const results = await eslint.lintFiles(["*.ts"]);
    const triggered = results.flatMap((result) => result.messages.filter((message) => message.ruleId === "jsdoc/require-jsdoc"));
    expect(triggered.length, context).toBeGreaterThan(0);
  }
});

test("extra configuration comes last and applies", async () => {
  const eslint = new ESLint({
    cwd: POLICY,
    overrideConfigFile: true,
    overrideConfig: strict(POLICY, [], { files: ["var.ts"], rules: { "no-var": "off" } }),
  });
  const [result] = await eslint.lintFiles(["var.ts"]);
  expect(result?.messages).toStrictEqual([]);
});

test("the base tsconfig rejects what each strict setting forbids", () => {
  const parsed = ts.getParsedCommandLineOfConfigFile(path.join(COMPILER, "tsconfig.json"), {}, {
    ...ts.sys,
    onUnRecoverableConfigFileDiagnostic: () => undefined,
  });
  const files = readdirSync(COMPILER).filter((name) => name.endsWith(".ts"));
  const program = ts.createProgram(
    files.map((name) => path.join(COMPILER, name)),
    parsed?.options ?? {},
  );
  const reported = Object.fromEntries(
    files.map((name) => [
      path.basename(name, ".ts"),
      ts
        .getPreEmitDiagnostics(program, program.getSourceFile(path.join(COMPILER, name)))
        .map((diagnostic) => diagnostic.code),
    ]),
  );
  expect(reported).toStrictEqual({
    erasable: [1294], // erasableSyntaxOnly: no enum
    "exact-optional": [2375], // exactOptionalPropertyTypes
    "implicit-any": [7006], // strict: noImplicitAny
    "implicit-return": [2366], // noImplicitReturns
    "index-signature": [4111], // noPropertyAccessFromIndexSignature
    "unchecked-index": [2322], // noUncheckedIndexedAccess
    "unknown-catch": [18046], // useUnknownInCatchVariables
    "unused-local": [6133], // noUnusedLocals
  });
});
