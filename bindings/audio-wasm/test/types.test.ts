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
test("only a node created with the marker channel can be captured from the start of its engine, a node offers what its stage's mode sets live, its refusals heard, a loaded patch's stage is the copy of its mode, and a render quantum is no literal", () => {
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
  // TS2345: an argument not assignable to the parameter; TS2322: a value not assignable to
  // the type; TS2339: no such property; TS2554: another number of arguments than an overload
  // naming its type parameter takes.
  expect(reported).toStrictEqual({
    "explicit-mode-without-output.ts": [2554, 2554],
    "extra-field.ts": [2339, 2339],
    "look-alike.ts": [2345],
    "marked.ts": [],
    "protected-switches-nothing.ts": [2339],
    "protected-unheard.ts": [2345],
    "quantum.ts": [2322],
    "raw-sets-nothing.ts": [2339],
    "switchable.ts": [],
    "unmarked.ts": [2345],
  });
}, 60_000);
