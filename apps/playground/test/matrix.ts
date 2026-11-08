// The shared test matrix (conformance/test-matrix.json), read by the TypeScript tests:
// the same sample rates as the Rust tests, never retyped.
import { existsSync, readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

/** The matrix file, found by walking up from the package directory (see patch.test.ts). */
function matrix(): unknown {
  for (let directory = pathToFileURL(`${process.cwd()}/`); directory.pathname !== "/"; directory = new URL("../", directory)) {
    const file = new URL("conformance/test-matrix.json", directory);
    if (existsSync(file)) {
      return JSON.parse(readFileSync(file, "utf8"));
    }
  }
  throw new Error("conformance/test-matrix.json not found");
}

/** Every sample rate of the matrix, in hertz. */
export const SAMPLE_RATES: readonly number[] = ((): number[] => {
  const rates: unknown = Reflect.get(Object(matrix()), "sample_rates");
  if (!Array.isArray(rates) || !rates.every((rate) => typeof rate === "number")) {
    throw new Error("invalid test matrix");
  }
  return rates.map(Number);
})();
