// The shared test matrix (conformance/test-matrix.json), read by the TypeScript tests:
// the same sample rates as the Rust tests, never retyped.
//
// bindings/wasm has its own reader (test/fixtures.ts): a package cannot import another's
// test files (relative imports break in the mutation testing sandbox). When a third
// TypeScript package needs the matrix (the editor), the readers move to one shared,
// tested workspace package instead of a third copy.
import { readFileSync } from "node:fs";
import { findUp } from "./repository.ts";

/** The matrix file, parsed. */
function matrix(): unknown {
  return JSON.parse(readFileSync(findUp("conformance/test-matrix.json"), "utf8"));
}

/** Every sample rate of the matrix, in hertz. */
export const SAMPLE_RATES: readonly number[] = ((): number[] => {
  const rates: unknown = Reflect.get(Object(matrix()), "sample_rates");
  if (!Array.isArray(rates) || !rates.every((rate) => typeof rate === "number")) {
    throw new Error("invalid test matrix");
  }
  return rates.map(Number);
})();
