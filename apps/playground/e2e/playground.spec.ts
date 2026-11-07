import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

/** Errors of the page during the current test. */
let errors: Error[] = [];

// Every page error fails the test: nothing may go wrong silently, in the page or the worklet.
test.beforeEach(({ page }) => {
  errors = [];
  page.on("pageerror", (error) => errors.push(error));
});

test.afterEach(() => {
  expect(errors).toStrictEqual([]);
});

test("plays the 440 Hz sine of the engine and shows it live", async ({ page }) => {
  await page.goto("./?muted");
  await expect(page).toHaveTitle("Playground · poyetik");
  await expect(page.getByRole("status")).toHaveText("Stopped");
  await expect(page.getByTestId("frequency")).toHaveText("—");

  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Playing");
  await expect(page.getByTestId("frequency")).toHaveText("440.0 Hz");
  await expect(page.getByTestId("peak")).toHaveText("1.00");
  await expect(page.getByTestId("rate")).toHaveText(/^\d+ Hz$/);

  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByRole("status")).toHaveText("Stopped");
  await expect(page.getByTestId("frequency")).toHaveText("—");

  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByTestId("frequency")).toHaveText("440.0 Hz");
});

test("draws the waveform on the oscilloscope", async ({ page }) => {
  await page.goto("./?muted");
  const canvas = page.getByRole("img", { name: "Waveform of the engine output" });
  /** Number of canvas pixels in the trace color. */
  const tracePixels = (): Promise<number> =>
    canvas.evaluate((element) => {
      if (!(element instanceof HTMLCanvasElement)) {
        throw new Error("not a canvas");
      }
      const { data } = element.getContext("2d")?.getImageData(0, 0, element.width, element.height) ?? { data: [] };
      let count = 0;
      for (let index = 0; index < data.length; index += 4) {
        // The trace color, #4fd1a5: green dominant, unlike the grid and the background.
        if ((data[index + 1] ?? 0) > 150 && (data[index] ?? 0) < 120) {
          count += 1;
        }
      }
      return count;
    });
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByTestId("frequency")).toHaveText("440.0 Hz");
  await expect.poll(tracePixels).toBeGreaterThan(1000);
});

test("is accessible", async ({ page }) => {
  await page.goto("./?muted");
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Playing");
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
});
