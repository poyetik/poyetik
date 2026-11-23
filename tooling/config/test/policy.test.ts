import { readdirSync } from "node:fs";
import path from "node:path";
import { ESLint, type Linter } from "eslint";
import jsdoc from "eslint-plugin-jsdoc";
import ts from "typescript";
import tseslint from "typescript-eslint";
import { expect, test } from "vitest";
import { AUDIO_CLOCK_READERS, JSDOC_CONTEXTS, MODULE_DRIVERS, RULES, RUNTIME_MESSAGES, TEST_FILES, TIMED_WAITS, strict } from "../eslint.ts";

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
  const results = await eslint.lintFiles(["*.ts", "e2e/*.ts", "e2e/site/*.ts", "src/cli/*.ts", "test/*.ts", "testing/*.ts", "ts/*.ts"]);
  const reported = Object.fromEntries(
    results.map((result) => [
      // Its path from the fixtures, without `.ts`: files of the same name apart.
      path.relative(POLICY, result.filePath).replace(/\.ts$/u, ""),
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
    "bare-fetch": Array<string>(4).fill("no-restricted-globals"),
    fetch: [],
    // In test code (TEST_FILES: a test or end-to-end folder, a test or spec file), the timers
    // of a sleep, bare and through each global object, but a test's own timeout and a timer
    // named in a string; code that is no test may run a timer; a reviewed timed wait
    // (TIMED_WAITS) may, never `fetch`; a fetch helper in test code may fetch, never sleep.
    "e2e/sleeps": Array<string>(5).fill("no-restricted-globals"),
    "test/sleeps": ["no-restricted-globals", "no-restricted-globals"],
    // Node's timer modules, whatever the name they are imported by, a testing folder's
    // sleep, and the sleeps no timer names: a signal that times out, a blocked thread.
    "test/timer-modules": Array<string>(4).fill("no-restricted-imports"),
    "testing/sleeps": ["no-restricted-globals"],
    "timer-modules": [],
    "signal-sleeps": ["no-restricted-properties", "no-restricted-properties"],
    "sleeps.test": ["no-restricted-globals"],
    "sleeps.spec": ["no-restricted-globals"],
    timer: [],
    "e2e/audio-clock": ["no-restricted-globals"],
    "e2e/fetch": ["no-restricted-globals"],
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
    "e2e/site/timing": ["no-restricted-properties", ...Array<string>(3).fill("no-restricted-syntax")],
    "src/cli/bin": [],
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
    // An error class alone, in each matcher of what a call throws, a rejection's included, by
    // its name or a namespace's; `every` asserted true, with a message and without, on an
    // optional chain, two joined (each), and by each matcher of true; none for
    // the error itself, a value's name or a constant's, the failing elements asserted empty,
    // or `every` compared otherwise.
    "class-only-throw": Array<string>(4).fill("no-restricted-syntax"),
    "every-true": Array<string>(8).fill("no-restricted-syntax"),
    // A part of an error's text, in each form; none for a pattern of RUNTIME_MESSAGES.
    "partial-throw": Array<string>(6).fill("no-restricted-syntax"),
    "exact-assertions": [],
    // A sleep on a page or a frame; an alternation in a matcher's regular expression, in
    // each matcher of text, and none where it is kept apart, escaped or a plain string.
    sleep: ["no-restricted-properties", "no-restricted-properties"],
    // (the two of what a call throws are parts of an error's text too).
    alternation: Array<string>(13).fill("no-restricted-syntax"),
    "alternation-free": [],
    "unused-leading-param": ["@typescript-eslint/no-unused-vars"],
    // In a module driver's code (MODULE_DRIVERS), a call by Reflect.apply outside the
    // boundary helper; inside it, or outside a driver's code, none.
    "ts/export-call": ["no-restricted-syntax"],
    "ts/exported": [],
    "reflect-apply": [],
  });
  const severities = results.flatMap((result) => result.messages.filter((message) => message.ruleId !== null).map((message) => message.severity));
  expect(new Set(severities)).toStrictEqual(new Set([ERROR]));
  const messages = results.flatMap((result) => result.messages.map((message) => message.message));
  expect(messages).toContain("Use a union of string literals or an `as const` object.");
  expect(messages).toContain("No global augmentation.");
  expect(messages).toContain(
    "Assert one text, not either of several (CLAUDE.md: tests prove presence): a matcher given a regular expression with an alternation passes on any branch; key what each case gives in a table and assert it exactly.",
  );
  expect(messages).toContain("'waitForTimeout' is restricted from being used. Wait for what the test needs (a poll, an event, the next recorded repaint), never for a time (CLAUDE.md: tests are deterministic).");
  expect(messages).toContain("Never focus or skip a test: run it, fix it, or discuss it (CLAUDE.md, testing policy).");
  expect(messages).toContain("Name the locale (CLAUDE.md: never test only at the default): a formatter given none follows the machine's.");
  expect(messages).toContain("Assert the error itself, `new X(message)`, not its class alone: a class passes on any message, another refusal of it included (CLAUDE.md: an error is asserted whole).");
  expect(messages).toContain("Assert the elements that fail, `expect(xs.filter((x) => !ok(x))).toStrictEqual([])`, not `every` as true: its failure names none of them (CLAUDE.md: a kept state is compared by its values).");
  expect(messages).toContain("Assert the error itself, `new X(message)`, not a part of its text (CLAUDE.md: an error is asserted whole): a text another program decides is a pattern of RUNTIME_MESSAGES, with why.");
  expect(messages).toContain("Name the reason a rejection carries: say where it goes (a log, a typed result, an error's cause), or drop it by a name starting with `_` and a comment saying why.");
  expect(messages).toContain(
    "'Promise.withResolvers' is restricted from being used. It is missing from browsers the friendly checks still explain (Safari before 17.4, Firefox before 121): use `new Promise`.",
  );
  const fetchMessage = "Fetch through the package's checked helper (its fetch.ts), which refuses a response that is not 2xx and names the URL.";
  expect(messages).toContain(`Unexpected use of 'fetch'. ${fetchMessage}`);
  const sleepMessage = "Wait for what the test needs (a poll, an event, the next recorded repaint), never for a time (CLAUDE.md: tests are deterministic).";
  for (const timer of ["setTimeout", "setInterval"]) {
    expect(messages).toContain(`Unexpected use of '${timer}'. ${sleepMessage}`);
  }
  const clockMessage = "Read no clock (CLAUDE.md: tests are deterministic): only the files of CLOCK_READERS read one.";
  expect(messages).toContain(`'performance.now' is restricted from being used. ${clockMessage}`);
  expect(messages).toContain(`'process.hrtime' is restricted from being used. ${clockMessage}`);
  expect(messages).toContain(`'process.uptime' is restricted from being used. ${clockMessage}`);
  expect(messages).toContain(clockMessage);
  expect(messages).toContain(
    "Call a module's export only through the driver's boundary helper (a function named `exported…`), which checks it and runs its call through the driver's failure handling: a call made otherwise escapes the guard the driver documents.",
  );
  expect(messages).toContain(
    "Read an audio context's clock through the package's checked reader (its audio-clock.ts): a browser may report one that is no time, which every AudioParam refuses.",
  );
  // The package's own ignores, then the outputs every package ignores.
  for (const ignored of ["ignored.ts", "coverage/x.ts", "reports/x.ts", ".stryker-tmp/x.ts", "playwright-report/x.ts", "test-results/x.ts"]) {
    expect(await eslint.isPathIgnored(path.join(POLICY, ignored)), ignored).toBe(true);
  }
  expect(await eslint.isPathIgnored(path.join(POLICY, "clean.ts"))).toBe(false);
});

test("lets a test match part of an error's text only by a pattern reviewed once, with why", () => {
  const patterns = RUNTIME_MESSAGES.map(([pattern]) => pattern);
  expect(new Set(patterns).size).toBe(patterns.length);
  expect(RUNTIME_MESSAGES.filter(([, why]) => why === "")).toStrictEqual([]);
});

test("allows a timer in test code only to the reviewed timed waits, each named", () => {
  expect([TEST_FILES, TIMED_WAITS]).toStrictEqual([["**/test/**", "**/testing/**", "**/e2e/**", "**/*.test.*", "**/*.spec.*"], ["**/e2e/audio-clock.ts"]]);
});

test("refuses exactly fetch and the timers of a sleep by scope, each entry in its order, the last matching one deciding", () => {
  const fetch = { name: "fetch", message: "Fetch through the package's checked helper (its fetch.ts), which refuses a response that is not 2xx and names the URL." };
  const sleep = "Wait for what the test needs (a poll, an event, the next recorded repaint), never for a time (CLAUDE.md: tests are deterministic).";
  const timers = [
    { name: "setTimeout", message: sleep },
    { name: "setInterval", message: sleep },
  ];
  const rule = (globals: readonly object[]): unknown => ["error", { globals, checkGlobalObject: true }];
  const entries = strict(POLICY, [])
    .filter((config) => config.rules?.["no-restricted-globals"] !== undefined)
    .map((config) => [config.files ?? "everywhere", config.ignores ?? [], config.rules?.["no-restricted-globals"]]);
  expect(entries).toStrictEqual([
    ["everywhere", [], rule([fetch])],
    [["**/test/**", "**/testing/**", "**/e2e/**", "**/*.test.*", "**/*.spec.*"], [], rule([fetch, ...timers])],
    [["**/fetch.ts"], [], rule([])],
    [["**/test/**", "**/testing/**", "**/e2e/**", "**/*.test.*", "**/*.spec.*"].map((tests) => ["**/fetch.ts", tests]), [], rule(timers)],
    [["**/e2e/audio-clock.ts"], [], rule([fetch])],
  ]);
});

test("refuses exactly Node's timer modules in test code, but to the reviewed timed waits", () => {
  const message = "Wait for what the test needs (a poll, an event, the next recorded repaint), never for a time (CLAUDE.md: tests are deterministic).";
  const entries = strict(POLICY, [])
    .filter((config) => config.rules?.["no-restricted-imports"] !== undefined)
    .map((config) => [config.files, config.ignores ?? [], config.rules?.["no-restricted-imports"]]);
  expect(entries).toStrictEqual([
    [
      ["**/test/**", "**/testing/**", "**/e2e/**", "**/*.test.*", "**/*.spec.*"],
      ["**/e2e/audio-clock.ts"],
      ["error", { paths: ["node:timers", "node:timers/promises", "timers", "timers/promises"].map((name) => ({ name, message })) }],
    ],
  ]);
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

test("a module driver keeps every general restriction and adds only the calls of an export", () => {
  const exportMessage = "Call a module's export only through the driver's boundary helper";
  const drivers = strict(POLICY, []).find((config) => config.files?.includes(MODULE_DRIVERS[0] ?? "") === true);
  const syntax = RULES["no-restricted-syntax"];
  const drivenSyntax = drivers?.rules?.["no-restricted-syntax"];
  expect(Array.isArray(syntax) && Array.isArray(drivenSyntax)).toBe(true);
  const general = Array.isArray(syntax) ? syntax : [];
  const driven = Array.isArray(drivenSyntax) ? drivenSyntax : [];
  // The general rule: "error", then restrictions that each say why; none on the calls of an export.
  expect(general[0]).toBe("error");
  for (const entry of general.slice(1)) {
    expect(typeof entry === "object" && entry !== null && "message" in entry, JSON.stringify(entry)).toBe(true);
    expect(JSON.stringify(entry)).not.toContain(exportMessage);
  }
  // A driver's: exactly the general one (the clock reads included), then the calls of an export.
  expect(driven.slice(0, general.length)).toStrictEqual(general);
  const added = driven.slice(general.length);
  expect(added.length).toBeGreaterThan(0);
  for (const entry of added) {
    expect(JSON.stringify(entry)).toContain(exportMessage);
  }
  expect(drivers?.rules?.["no-restricted-properties"]).toStrictEqual(RULES["no-restricted-properties"]);
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
