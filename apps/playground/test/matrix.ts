// The shared test matrix (conformance/test-matrix.json), read by the TypeScript tests:
// the same sample rates as the Rust tests, and the time zones and locales of the browsers,
// never retyped.
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

/** The non-empty list of strings under `key` in the matrix; throws if it is missing or of another shape. */
function strings(key: "locales" | "time_zones"): readonly [string, ...string[]] {
  const values: unknown = Reflect.get(Object(matrix()), key);
  if (!Array.isArray(values) || !values.every((value) => typeof value === "string")) {
    throw new Error(`invalid test matrix: ${key} must be a list of strings`);
  }
  const [first, ...rest] = values;
  if (first === undefined) {
    throw new Error(`invalid test matrix: ${key} is empty`);
  }
  return [first, ...rest];
}

/** Every IANA time zone of the matrix: the browsers of the end-to-end tests run in them. */
export const TIME_ZONES = strings("time_zones");

/** Every locale of the matrix (BCP 47 tags): the browsers of the end-to-end tests run in them. */
export const LOCALES = strings("locales");
