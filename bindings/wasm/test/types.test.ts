// Type-level tests: what the compiler must refuse in code using the package, checked by
// compiling the fixtures of test/types/ (outside the package's own type check) with the
// package's compiler settings.
import { readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { expect, test } from "vitest";

/** The fixtures: each file compiles, or breaks one type rule of the package. */
const FIXTURES = fileURLToPath(new URL("types/", import.meta.url));

test("only a node created with the marker channel can be captured from the start of its engine", () => {
  const config = ts.getParsedCommandLineOfConfigFile(fileURLToPath(new URL("../tsconfig.json", import.meta.url)), {}, {
    ...ts.sys,
    onUnRecoverableConfigFileDiagnostic: () => undefined,
  });
  const files = readdirSync(FIXTURES).filter((name) => name.endsWith(".ts")).sort();
  const program = ts.createProgram(
    files.map((name) => `${FIXTURES}${name}`),
    { ...config?.options, noEmit: true },
  );
  const reported = Object.fromEntries(
    files.map((name) => [name, ts.getPreEmitDiagnostics(program, program.getSourceFile(`${FIXTURES}${name}`)).map((diagnostic) => diagnostic.code)]),
  );
  // TS2345: an argument not assignable to the parameter.
  expect(reported).toStrictEqual({ "look-alike.ts": [2345], "marked.ts": [], "unmarked.ts": [2345] });
});
