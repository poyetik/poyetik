/** A stand-in for Playwright's page, whose sleep is refused whatever holds it. */
declare const page: { readonly waitForTimeout: (milliseconds: number) => Promise<void>; readonly mainFrame: () => { readonly waitForTimeout: (milliseconds: number) => Promise<void> } };

await page.waitForTimeout(37);
await page.mainFrame().waitForTimeout(100);
