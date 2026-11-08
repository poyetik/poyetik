// The shared test matrix (conformance/test-matrix.json), read by the roadmap's tests and its
// Playwright configuration: the time zones and locales are never retyped.
import { readFileSync } from "node:fs";

/** The matrix, parsed but not yet trusted. */
const MATRIX: unknown = JSON.parse(readFileSync(new URL("../../../conformance/test-matrix.json", import.meta.url), "utf8"));

/** The non-empty list of strings under `key` in the matrix; throws if it is missing or of another shape. */
function strings(key: string): readonly [string, ...string[]] {
  const values: unknown = Reflect.get(Object(MATRIX), key);
  if (!Array.isArray(values) || !values.every((value) => typeof value === "string")) {
    throw new Error(`invalid test matrix: ${key} must be a list of strings`);
  }
  const [first, ...rest] = values;
  if (first === undefined) {
    throw new Error(`invalid test matrix: ${key} is empty`);
  }
  return [first, ...rest];
}

/** Every IANA time zone of the matrix. */
export const TIME_ZONES = strings("time_zones");

/** Every locale of the matrix (BCP 47 tags). */
export const LOCALES = strings("locales");

/**
 * The offset `Date#getTimezoneOffset` gives in `timeZone` at the instant `time` (minutes behind UTC;
 * 2026-01-05 by default), computed by `Intl` independently of `TZ`: a test asserts it to prove that
 * its zone really took effect.
 */
export function offsetOf(timeZone: string, time: number = Date.UTC(2026, 0, 5, 12)): number {
  const name = new Intl.DateTimeFormat("en-US", { timeZone, timeZoneName: "longOffset" })
    .formatToParts(time)
    .find((part) => part.type === "timeZoneName")?.value;
  const match = /^GMT(?:([+-])(\d{2}):(\d{2}))?$/.exec(name ?? "");
  if (match === null) {
    throw new Error(`unknown offset for ${timeZone}: ${String(name)}`);
  }
  const [, sign, hours = "0", minutes = "0"] = match;
  const total = Number(hours) * 60 + Number(minutes);
  // East of UTC is a negative offset; `0 - total` keeps UTC at +0, as `getTimezoneOffset` gives it.
  return sign === "+" ? 0 - total : total;
}
