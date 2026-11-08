import { readdirSync } from "node:fs";
import path from "node:path";
import { ESLint } from "eslint";
import ts from "typescript";
import { expect, test } from "vitest";
import { strict } from "../eslint.ts";

/** The fixtures of the linting policy: each file but `clean.ts` breaks one rule. */
const POLICY = path.join(import.meta.dirname, "policy");

/** The fixtures of the compiler settings: each file breaks one setting of the base tsconfig. */
const COMPILER = path.join(import.meta.dirname, "compiler");

/** Errors, not warnings: a warning would let the check pass. */
const ERROR = 2;

test("each rule of the policy rejects what it forbids, and only that", async () => {
  const eslint = new ESLint({ cwd: POLICY, overrideConfigFile: true, overrideConfig: strict(POLICY, ["ignored.ts"]) });
  const results = await eslint.lintFiles(["*.ts"]);
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
    "undocumented-function": [documented],
    "undocumented-exported-const": [documented],
    "undocumented-module-const": [documented],
    "undocumented-type": [documented],
    "undocumented-interface": [documented],
    "undocumented-class": [documented],
    // The property, the getter, the setter and the method; not the constructor (see clean.ts).
    "undocumented-class-members": [documented, documented, documented, documented],
    "empty-description": ["jsdoc/require-description"],
    // The inline comment does not silence the rule, and is reported itself.
    "loose-equality": ["eqeqeq", "inline"],
    "param-mutation": ["no-param-reassign"],
    "param-reassign": ["no-param-reassign"],
    "let-never-reassigned": ["prefer-const"],
    "short-name": ["id-length"],
    shadow: ["@typescript-eslint/no-shadow"],
    var: ["no-var"],
  });
  const severities = results.flatMap((result) => result.messages.filter((message) => message.ruleId !== null).map((message) => message.severity));
  expect(new Set(severities)).toStrictEqual(new Set([ERROR]));
  const messages = results.flatMap((result) => result.messages.map((message) => message.message));
  expect(messages).toContain("Use a union of string literals or an `as const` object.");
  expect(messages).toContain("No global augmentation.");
  // The package's own ignores, then the outputs every package ignores.
  for (const ignored of ["ignored.ts", "coverage/x.ts", "reports/x.ts", ".stryker-tmp/x.ts", "playwright-report/x.ts", "test-results/x.ts"]) {
    expect(await eslint.isPathIgnored(path.join(POLICY, ignored)), ignored).toBe(true);
  }
  expect(await eslint.isPathIgnored(path.join(POLICY, "clean.ts"))).toBe(false);
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
