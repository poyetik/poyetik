import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";

/** Reference hashes of the conformance corpus, rendered by the native engine. */
const references: unknown = JSON.parse(readFileSync(new URL("../../../conformance/references.json", import.meta.url), "utf8"));

/** The reference hash of a case. */
function reference(name: string): string {
  const hash: unknown = typeof references === "object" && references !== null ? Reflect.get(references, name) : undefined;
  if (typeof hash !== "string") {
    throw new Error(`no reference for ${name}`);
  }
  return hash;
}

const HASH = /^[0-9a-f]{64}$/;

test("renders sine-440 offline bit-identical to the native engine", async ({ page }) => {
  await page.goto("e2e/site/index.html?case=sine-440");
  await page.getByRole("button", { name: "Render offline" }).click();
  await expect(page.locator("#offline-hash")).toHaveText(HASH);
  await expect(page.locator("#offline-hash")).toHaveText(reference("sine-440"));
  await expect(page.locator("#error")).toHaveText("");
});

test("plays sine-440 live bit-identical to the native engine", async ({ page }) => {
  await page.goto("e2e/site/index.html?case=sine-440");
  await page.getByRole("button", { name: "Capture live" }).click();
  await expect(page.locator("#live-hash")).toHaveText(HASH, { timeout: 15_000 });
  await expect(page.locator("#live-hash")).toHaveText(reference("sine-440"));
  await expect(page.locator("#error")).toHaveText("");
});

test("explains why a patch is rejected", async ({ page }) => {
  await page.goto("e2e/site/index.html?case=sine-440");
  await page.getByRole("button", { name: "Load an invalid patch" }).click();
  await expect(page.locator("#error")).toContainText("nope~");
});
