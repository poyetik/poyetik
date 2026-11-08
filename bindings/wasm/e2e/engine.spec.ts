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

test("runs the engine under a strict Content Security Policy, breaking none of it", async ({ page }) => {
  const violations: string[] = [];
  page.on("console", (message) => {
    if (message.text().includes("Content Security Policy")) {
      violations.push(message.text());
    }
  });
  await page.goto("e2e/site/index.html?case=sine-440");
  await page.getByRole("button", { name: "Render offline" }).click();
  await expect(page.locator("#offline-hash")).toHaveText(reference("sine-440"));
  expect(violations).toStrictEqual([]);
});

test("explains why a patch is rejected", async ({ page }) => {
  await page.goto("e2e/site/index.html?case=sine-440");
  await page.getByRole("button", { name: "Load an invalid patch" }).click();
  await expect(page.locator("#error")).toContainText("nope~");
});
