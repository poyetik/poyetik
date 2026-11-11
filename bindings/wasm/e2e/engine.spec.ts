import { expect, test } from "@playwright/test";
import { readdirSync, readFileSync } from "node:fs";
import { BOX_BENCHMARKS, boxBoundMs, constructionBoundMs, parseThresholds } from "../benchmarks/suite.ts";
import { LOCALES, TIME_ZONES } from "../test/fixtures.ts";

/** The conformance corpus at the repository root, shared with the native engine tests. */
const CONFORMANCE = new URL("../../../conformance/", import.meta.url);

/** Reference hashes of the conformance corpus, rendered by the native engine. */
const references: unknown = JSON.parse(readFileSync(new URL("references.json", CONFORMANCE), "utf8"));

/** Every case of the corpus, by name. */
const CASES = readdirSync(new URL("cases/", CONFORMANCE))
  .filter((file) => file.endsWith(".json"))
  .map((file) => file.slice(0, -".json".length))
  .sort();

/**
 * Cases also captured live, through the real audio graph (in real time, so a subset): the
 * default rate and 44.1 kHz, and the case whose render given as binary32 delivers binary32
 * subnormals (2^-126 Hz at 8 kHz), which a real-time audio thread under flush-to-zero would
 * change, as engine-fpenv's flushing control shows natively.
 */
const LIVE = ["sine-440", "sine-440-at-44k1", "sine-1.1754944e-38-at-8k"];

/**
 * Every precision the engine computes in, each with the render format a browser captures:
 * the render given as binary32, as Web Audio takes it.
 */
const PRECISIONS = [
  { precision: "binary64", format: "binary64-as-binary32" },
  { precision: "binary32", format: "binary32" },
] as const;

/** The reference hash of a case in a render format. */
function reference(name: string, format: string): string {
  const hashes: unknown = typeof references === "object" && references !== null ? Reflect.get(references, name) : undefined;
  const hash: unknown = typeof hashes === "object" && hashes !== null ? Reflect.get(hashes, format) : undefined;
  if (typeof hash !== "string") {
    throw new Error(`no ${format} reference for ${name}`);
  }
  return hash;
}

/** A SHA-256 hash in lowercase hex, as the bench page shows it once a render finished. */
const HASH = /^[0-9a-f]{64}$/;

test("covers the whole corpus, with a reference for every case", () => {
  expect(CASES.length).toBeGreaterThan(1);
  for (const name of [...CASES, ...LIVE]) {
    for (const { format } of PRECISIONS) {
      expect(reference(name, format)).toMatch(HASH);
    }
  }
  expect(new Set(CASES.map((name) => reference(name, "binary32"))).size).toBe(CASES.length);
  expect(LIVE.every((name) => CASES.includes(name))).toBe(true);
});

for (const { precision, format } of PRECISIONS) {
  for (const name of CASES) {
    test(`renders ${name} offline in ${precision} bit-identical to the native engine`, async ({ page }) => {
      await page.goto(`e2e/site/index.html?case=${name}&precision=${precision}`);
      await page.getByRole("button", { name: "Render offline" }).click();
      await expect(page.locator("#offline-hash")).toHaveText(HASH);
      await expect(page.locator("#offline-hash")).toHaveText(reference(name, format));
      await expect(page.locator("#error")).toHaveText("");
    });
  }

  for (const name of LIVE) {
    test(`plays ${name} live in ${precision} bit-identical to the native engine`, async ({ page }) => {
      await page.goto(`e2e/site/index.html?case=${name}&precision=${precision}`);
      await page.getByRole("button", { name: "Capture live" }).click();
      await expect(page.locator("#live-hash")).toHaveText(HASH, { timeout: 15_000 });
      await expect(page.locator("#live-hash")).toHaveText(reference(name, format));
      await expect(page.locator("#error")).toHaveText("");
    });
  }
}

test("computes in binary64 unless asked otherwise", async ({ page }) => {
  await page.goto("e2e/site/index.html?case=sine-440");
  await page.getByRole("button", { name: "Render offline" }).click();
  await expect(page.locator("#offline-hash")).toHaveText(reference("sine-440", "binary64-as-binary32"));
});

test("refuses an unknown precision", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("e2e/site/index.html?case=sine-440&precision=binary16");
  await expect.poll(() => errors).toStrictEqual(["unknown precision: binary16"]);
  // Shown on the page too, for whoever opens it by hand.
  await expect(page.locator("#error")).toHaveText("Error: unknown precision: binary16");
});

/**
 * The policy of a page running the engine, as the bench page declares it: the site only,
 * plus compiling WebAssembly. Written here on purpose: loosening the page's policy (an
 * `'unsafe-eval'`, a wider source) fails this test until it is changed here too.
 */
const POLICY = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'";

test("runs the engine offline and live under its Content Security Policy, breaking none of it, which is enforced", async ({ page }) => {
  // Two channels: the page's own violation events, and console errors, for what a violation
  // in the worklet scope may report only there.
  const errors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") {
      errors.push(message.text());
    }
  });
  await page.addInitScript(() => {
    const seen: string[] = [];
    Reflect.set(window, "violations", seen);
    document.addEventListener("securitypolicyviolation", (event) => {
      seen.push(`${event.violatedDirective} ${event.blockedURI}`);
    });
  });
  await page.goto("e2e/site/index.html?case=sine-440");
  await expect(page.locator('meta[http-equiv="Content-Security-Policy"]')).toHaveAttribute("content", POLICY);
  await page.getByRole("button", { name: "Render offline" }).click();
  await expect(page.locator("#offline-hash")).toHaveText(reference("sine-440", "binary64-as-binary32"));
  await page.getByRole("button", { name: "Capture live" }).click();
  await expect(page.locator("#live-hash")).toHaveText(reference("sine-440", "binary64-as-binary32"), { timeout: 15_000 });
  const violations = (): Promise<unknown> => page.evaluate((): unknown => Reflect.get(window, "violations"));
  expect(await violations()).toStrictEqual([]);
  expect(errors).toStrictEqual([]);
  // Control: an image from another site is blocked and reported.
  await page.evaluate(() => {
    const image = document.createElement("img");
    image.src = "https://example.invalid/blocked.png";
    document.body.append(image);
  });
  await expect.poll(violations).toStrictEqual(["img-src https://example.invalid/blocked.png"]);
});

test("names the file it could not fetch", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("e2e/site/index.html?case=missing-case");
  await expect.poll(() => errors).toStrictEqual(["GET /conformance/cases/missing-case.json: 404"]);
  await expect(page.locator("#error")).toHaveText("Error: GET /conformance/cases/missing-case.json: 404");
});

test("explains why a patch is rejected, and shows no stale error after a later success", async ({ page }) => {
  await page.goto("e2e/site/index.html?case=sine-440");
  await page.getByRole("button", { name: "Load an invalid patch" }).click();
  await expect(page.locator("#error")).toContainText("nope~");
  await page.getByRole("button", { name: "Render offline" }).click();
  await expect(page.locator("#offline-hash")).toHaveText(reference("sine-440", "binary64-as-binary32"));
  await expect(page.locator("#error")).toHaveText("");
  await expect(page.getByRole("button", { name: "Load an invalid patch" })).toBeEnabled();
});

test("runs in a time zone and a locale of the test matrix, not the machine's", async ({ page }, testInfo) => {
  const { timezoneId, locale } = testInfo.project.use;
  expect(TIME_ZONES).toContain(timezoneId);
  expect(LOCALES).toContain(locale);
  await page.goto("e2e/site/index.html?case=sine-440");
  // The emulation took effect in the page itself.
  expect(await page.evaluate(() => Intl.DateTimeFormat().resolvedOptions().timeZone)).toBe(timezoneId);
  // Compared in their maximal form: WebKit drops a region its language implies ("ja-JP" reads "ja").
  const maximal = (tag: string): string => new Intl.Locale(tag).maximize().toString();
  const [language, formatting] = await page.evaluate(() => [navigator.language, Intl.DateTimeFormat().resolvedOptions().locale]);
  const expected = maximal(locale ?? "");
  expect([maximal(language), maximal(formatting)]).toStrictEqual([expected, expected]);
});

/** The thresholds of the benchmarks (spec/benchmarks.md), the construction bound among them. */
const THRESHOLDS = parseThresholds(readFileSync(new URL("../../../spec/benchmark-thresholds.json", import.meta.url), "utf8"));

test("constructs the engine processor within its bound, every time", async ({ page, browserName }) => {
  const { runs } = THRESHOLDS.construction;
  const bound = constructionBoundMs(THRESHOLDS, browserName);
  await page.goto(`e2e/site/index.html?case=sine-440&runs=${String(runs)}`);
  await page.getByRole("button", { name: "Time construction" }).click();
  const output = page.locator("#construction-ms");
  await expect(output).toHaveText(/^\d/, { timeout: 60_000 });
  const times = ((await output.textContent()) ?? "").split(" ").map(Number);
  // Printed, so that a new measurement of the bounds can read them (spec/benchmarks.md).
  process.stdout.write(`construction of the engine processor in ${browserName}, milliseconds: ${times.join(" ")}\n`);
  expect(times).toHaveLength(runs);
  expect(times.every((time) => Number.isFinite(time) && time >= 0)).toBe(true);
  expect(Math.max(...times)).toBeLessThanOrEqual(bound);
  await expect(page.locator("#error")).toHaveText("");
});

/**
 * The median of `times`.
 * @throws {Error} when there is none: no median, rather than a silent 0.
 */
function median(times: readonly number[]): number {
  if (times.length === 0) {
    throw new Error("no time to take the median of");
  }
  const sorted = [...times].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1 ? (sorted[middle] ?? 0) : ((sorted[middle - 1] ?? 0) + (sorted[middle] ?? 0)) / 2;
}

test("renders every box benchmark in both precisions within its bound, every time, and prints the cost ratio", async ({ page, browserName }) => {
  test.setTimeout(180_000);
  const { blocks, runs } = THRESHOLDS.boxes;
  const bound = boxBoundMs(THRESHOLDS, browserName);
  await page.goto("e2e/site/index.html?case=sine-440");
  // The page's top-level awaits are done once a button is enabled.
  await expect(page.getByRole("button", { name: "Render offline" })).toBeEnabled();
  const time = (patch: unknown, rate: number, precision: string): Promise<number[]> =>
    page.evaluate(
      async ([text, at, asked, count, times]) => {
        const timer: unknown = Reflect.get(window, "timeRendering");
        if (typeof timer !== "function") {
          throw new Error("no timeRendering on the bench page");
        }
        const measured: unknown = await Reflect.apply(timer, undefined, [text, at, asked, count, times]);
        if (!Array.isArray(measured)) {
          throw new Error(`timeRendering gave ${String(measured)}, not a list of times`);
        }
        return measured.map(Number);
      },
      [JSON.stringify(patch), rate, precision, blocks, runs] as const,
    );
  const slowest: number[] = [];
  for (const benchmark of BOX_BENCHMARKS) {
    const cost: Record<string, number> = {};
    for (const precision of ["binary64", "binary32"]) {
      const holding = await time(benchmark.patch, benchmark.rate, precision);
      const without = await time(benchmark.without, benchmark.rate, precision);
      for (const times of [holding, without]) {
        expect(times).toHaveLength(runs);
        expect(times.every((value) => Number.isFinite(value) && value >= 0)).toBe(true);
      }
      slowest.push(Math.max(...holding));
      cost[precision] = median(holding) - median(without);
      // Printed, so that a new measurement of the bound can read them (spec/benchmarks.md).
      process.stdout.write(`${benchmark.box} ${benchmark.setting} in ${precision} in ${browserName}, ${String(blocks)} blocks, milliseconds: ${holding.join(" ")} (without the box: ${without.join(" ")})\n`);
    }
    const [wide, narrow] = [cost["binary64"] ?? 0, cost["binary32"] ?? 0];
    const ratio = wide > 0 && narrow > 0 ? (narrow / wide).toFixed(2) : "not measurable (a cost at or below the noise)";
    process.stdout.write(`${benchmark.box} ${benchmark.setting} in ${browserName}: binary32 / binary64 cost ratio ${ratio}\n`);
  }
  expect(Math.max(...slowest)).toBeLessThanOrEqual(bound);
});
