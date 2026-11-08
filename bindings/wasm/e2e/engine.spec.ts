import { expect, test } from "@playwright/test";
import { readdirSync, readFileSync } from "node:fs";

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
 * default rate and 44.1 kHz.
 */
const LIVE = ["sine-440", "sine-440-at-44k1"];

/** The reference hash of a case. */
function reference(name: string): string {
  const hash: unknown = typeof references === "object" && references !== null ? Reflect.get(references, name) : undefined;
  if (typeof hash !== "string") {
    throw new Error(`no reference for ${name}`);
  }
  return hash;
}

/** A SHA-256 hash in lowercase hex, as the bench page shows it once a render finished. */
const HASH = /^[0-9a-f]{64}$/;

test("covers the whole corpus, with a reference for every case", () => {
  expect(CASES.length).toBeGreaterThan(1);
  for (const name of [...CASES, ...LIVE]) {
    expect(reference(name)).toMatch(HASH);
  }
  expect(LIVE.every((name) => CASES.includes(name))).toBe(true);
});

for (const name of CASES) {
  test(`renders ${name} offline bit-identical to the native engine`, async ({ page }) => {
    await page.goto(`e2e/site/index.html?case=${name}`);
    await page.getByRole("button", { name: "Render offline" }).click();
    await expect(page.locator("#offline-hash")).toHaveText(HASH);
    await expect(page.locator("#offline-hash")).toHaveText(reference(name));
    await expect(page.locator("#error")).toHaveText("");
  });
}

for (const name of LIVE) {
  test(`plays ${name} live bit-identical to the native engine`, async ({ page }) => {
    await page.goto(`e2e/site/index.html?case=${name}`);
    await page.getByRole("button", { name: "Capture live" }).click();
    await expect(page.locator("#live-hash")).toHaveText(HASH, { timeout: 15_000 });
    await expect(page.locator("#live-hash")).toHaveText(reference(name));
    await expect(page.locator("#error")).toHaveText("");
  });
}

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
  await expect(page.locator("#offline-hash")).toHaveText(reference("sine-440"));
  await page.getByRole("button", { name: "Capture live" }).click();
  await expect(page.locator("#live-hash")).toHaveText(reference("sine-440"), { timeout: 15_000 });
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
});

test("explains why a patch is rejected", async ({ page }) => {
  await page.goto("e2e/site/index.html?case=sine-440");
  await page.getByRole("button", { name: "Load an invalid patch" }).click();
  await expect(page.locator("#error")).toContainText("nope~");
});
