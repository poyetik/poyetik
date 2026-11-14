// Type-level tests: what the compiler must refuse in code using the package, checked by
// compiling the fixtures of test/types/ (outside the package's own type check) with the
// package's compiler settings.
import { readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { expect, test } from "vitest";

/** The fixtures: each file compiles, or breaks one type rule of the package. */
const FIXTURES = fileURLToPath(new URL("types/", import.meta.url));

// A TypeScript program over the fixtures, type-checked whole: a fixed amount of work,
// which took 0.6 to 1.5 s plain on a loaded 10-core Mac, alone or beside the package's
// other files, and more on a loaded machine (Vitest's default is 5 s). 60 s leaves a
// margin of 40 times the slowest plain run.
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
}, 60_000);
