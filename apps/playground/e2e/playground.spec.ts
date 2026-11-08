import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";
import { CSP } from "../csp.ts";

/** Errors of the page during the current test: uncaught errors, console errors, failed requests. */
let errors: string[] = [];

// Every error fails the test: nothing may go wrong silently, in the page, the worklet or
// the network (a missing favicon included). A test expecting a failure says which.
let expected: RegExp | undefined;
test.beforeEach(({ page }) => {
  errors = [];
  expected = undefined;
  page.on("pageerror", (error) => errors.push(`page error: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") {
      errors.push(`console error: ${message.text()}`);
    }
  });
  page.on("response", (response) => {
    if (response.status() >= 400) {
      errors.push(`HTTP ${String(response.status())}: ${response.url()}`);
    }
  });
});

test.afterEach(() => {
  expect(errors.filter((error) => !(expected?.test(error) ?? false))).toStrictEqual([]);
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

test("draws the waveform on the oscilloscope: full height, full width", async ({ page }) => {
  await page.goto("./?muted");
  const canvas = page.getByRole("img", { name: "Waveform of the engine output" });
  /** The extent of the pixels in the trace color, as fractions of the canvas size. */
  const extent = (): Promise<{ top: number; bottom: number; columns: number }> =>
    canvas.evaluate((element) => {
      if (!(element instanceof HTMLCanvasElement)) {
        throw new Error("not a canvas");
      }
      const { width, height } = element;
      const { data } = element.getContext("2d")?.getImageData(0, 0, width, height) ?? { data: [] };
      let top = height;
      let bottom = -1;
      const columns = new Set<number>();
      for (let index = 0; index < data.length; index += 4) {
        // The trace color, #4fd1a5: green dominant, unlike the grid and the background.
        if ((data[index + 1] ?? 0) > 150 && (data[index] ?? 0) < 120) {
          const pixel = index / 4;
          const row = Math.floor(pixel / width);
          top = Math.min(top, row);
          bottom = Math.max(bottom, row);
          columns.add(pixel % width);
        }
      }
      return { top: top / height, bottom: (bottom + 1) / height, columns: columns.size / width };
    });
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByTestId("frequency")).toHaveText("440.0 Hz");
  // A full-scale sine reaches ±0.9 of the half height (5% and 95% of the height), and the
  // trace spans the width but for the sub-sample shift at the left edge (at most 1%).
  await expect
    .poll(async () => {
      const { top, bottom, columns } = await extent();
      return top <= 0.07 && bottom >= 0.93 && columns >= 0.98;
    })
    .toBe(true);
});

test("is accessible", async ({ page }) => {
  await page.goto("./?muted");
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Playing");
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
});

test("plays and stops from the keyboard, the button keeping the focus", async ({ page }) => {
  await page.goto("./?muted");
  const button = page.getByRole("button");
  await button.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("status")).toHaveText("Playing");
  await expect(button).toBeFocused();
  await expect(button).toHaveText("Stop");
  await page.keyboard.press("Space");
  await expect(page.getByRole("status")).toHaveText("Stopped");
  await expect(button).toBeFocused();
});

test("explains a failure next to the button, accessibly, and can start again", async ({ page }) => {
  expected = /\.wasm|status of 404/;
  await page.route("**/*.wasm", (route) => route.fulfill({ status: 404, body: "" }));
  await page.goto("./?muted");
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Failed");
  const alert = page.getByRole("alert");
  await expect(alert).toContainText("cannot load the engine from");
  await expect(alert).toContainText("(HTTP 404)");
  await expect(page.getByRole("button")).toHaveAttribute("aria-describedby", "player-alert");
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);

  await page.unroute("**/*.wasm");
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Playing");
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("is accessible once stopped again", async ({ page }) => {
  await page.goto("./?muted");
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Playing");
  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByRole("status")).toHaveText("Stopped");
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
});

test("declares its Content Security Policy and breaks none of it while playing", async ({ page }) => {
  // The control below makes the browser report a blocked image (and a request refused).
  expected = /blocked\.png/;
  await page.addInitScript(() => {
    const violations: string[] = [];
    Reflect.set(window, "violations", violations);
    document.addEventListener("securitypolicyviolation", (event) => {
      violations.push(`${event.violatedDirective} ${event.blockedURI}`);
    });
  });
  await page.goto("./?muted");
  await expect(page.locator('meta[http-equiv="Content-Security-Policy"]')).toHaveAttribute("content", CSP);
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByTestId("frequency")).toHaveText("440.0 Hz");
  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByRole("status")).toHaveText("Stopped");
  const violations = (): Promise<unknown> => page.evaluate((): unknown => Reflect.get(window, "violations"));
  expect(await violations()).toStrictEqual([]);
  // Control: the policy is enforced and violations are seen. An image from another site
  // is blocked before any request.
  await page.evaluate(() => {
    const image = document.createElement("img");
    image.src = "https://example.invalid/blocked.png";
    document.body.append(image);
  });
  await expect.poll(violations).toStrictEqual(["img-src https://example.invalid/blocked.png"]);
});

test("gives the oscilloscope one backing pixel per device pixel", async ({ page }) => {
  await page.goto("./?muted");
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByTestId("frequency")).toHaveText("440.0 Hz");
  const canvas = page.getByRole("img", { name: "Waveform of the engine output" });
  const size = (): Promise<{ width: number; expected: number }> =>
    canvas.evaluate((element) => {
      if (!(element instanceof HTMLCanvasElement)) {
        throw new Error("not a canvas");
      }
      return { width: element.width, expected: Math.round(element.clientWidth * window.devicePixelRatio) };
    });
  await expect.poll(async () => {
    const { width, expected: wanted } = await size();
    return width === wanted;
  }).toBe(true);
  await expect(canvas).toHaveAttribute("aria-describedby", "scope-readout");
});

test("keeps the oscilloscope colors in forced colors (high contrast)", async ({ page }) => {
  // The page text follows the system colors there; the canvas keeps its own background,
  // against which its trace was designed (forced-color-adjust: none).
  await page.goto("./?muted");
  const canvas = page.getByRole("img", { name: "Waveform of the engine output" });
  const background = (): Promise<string> => canvas.evaluate((element) => getComputedStyle(element).backgroundColor);
  const normal = await background();
  await page.emulateMedia({ forcedColors: "active", colorScheme: "light" });
  expect(await background()).toBe(normal);
});

for (const rate of [44_100, 96_000]) {
  test(`plays the 440 Hz sine on a device at ${String(rate)} Hz`, async ({ page }) => {
    // Every AudioContext of the page runs at this rate, as on a device that imposes it.
    await page.addInitScript((forced) => {
      const Native = window.AudioContext;
      window.AudioContext = class extends Native {
        constructor(options?: AudioContextOptions) {
          super({ ...options, sampleRate: forced });
        }
      };
    }, rate);
    await page.goto("./?muted");
    await page.getByRole("button", { name: "Start" }).click();
    await expect(page.getByTestId("rate")).toHaveText(`${String(rate)} Hz`);
    await expect(page.getByTestId("frequency")).toHaveText("440.0 Hz");
  });
}
