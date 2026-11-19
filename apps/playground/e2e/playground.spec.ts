import { readFileSync } from "node:fs";
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { PageModule } from "@poyetik/web-page/page";
import { CSP } from "../csp.ts";
import { type SquareNow, squareNow as readSquare } from "./audio-clock.ts";
import { keepGpuCanvases } from "./gpu-canvas.ts";
import { BUTTON_LABELS, REFUSED } from "../src/App.tsx";
import { explain } from "../src/player.ts";
import { MAX_FREQUENCY_RATIO } from "../src/scope.ts";
import { SQUARE } from "../src/settings.ts";
import { DRAWN_BY, FAILED, MOVING, STILL } from "../src/Square.tsx";
import { LOCALES, SAMPLE_RATES, TIME_ZONES } from "../test/matrix.ts";

/**
 * Makes every AudioContext of the page reachable by {@link suspendAudio}, counted by
 * {@link contexts}, and run at `rate` when given, as on a device that imposes it.
 */
async function trackAudio(page: Page, rate?: number): Promise<void> {
  await page.addInitScript(
    ({ forced }: { forced: number | null }) => {
      const Native = window.AudioContext;
      window.AudioContext = class extends Native {
        constructor(options?: AudioContextOptions) {
          super(forced === null ? options : { ...options, sampleRate: forced });
          Reflect.set(window, "audio", this);
          const created: unknown = Reflect.get(window, "contexts");
          Reflect.set(window, "contexts", typeof created === "number" ? created + 1 : 1);
        }
      };
    },
    { forced: rate ?? null },
  );
}

/** How many AudioContexts the page created since it loaded; needs {@link trackAudio}. */
async function contexts(page: Page): Promise<unknown> {
  return page.evaluate((): unknown => Reflect.get(window, "contexts") ?? 0);
}

/** Suspends the last AudioContext of the page, as the browser does for a phone call or another tab. */
async function suspendAudio(page: Page): Promise<void> {
  await page.evaluate(async () => {
    const audio: unknown = Reflect.get(window, "audio");
    if (!(audio instanceof AudioContext)) {
      throw new Error("no AudioContext: call trackAudio before loading the page");
    }
    await audio.suspend();
  });
}

/**
 * Moves the keyboard focus to `target` from the start of the page, by pressing Tab as a
 * keyboard user would: the target must be in the Tab order (a programmatic focus would
 * reach any element). On WebKit, Tab skips buttons unless the user enables it, like Safari:
 * there Option+Tab moves to every control.
 */
async function tabTo(page: Page, target: Locator, browserName: string): Promise<void> {
  const key = browserName === "webkit" ? "Alt+Tab" : "Tab";
  for (let presses = 0; presses < 5 && !(await target.evaluate((element) => element === document.activeElement)); presses += 1) {
    await page.keyboard.press(key);
  }
  await expect(target).toBeFocused();
}

/** The red, green and blue channels (0 to 255) of a computed CSS color, `rgb(…)` or `rgba(…)`. */
function channels(color: string): [number, number, number] {
  const match = /^rgba?\((\d+), (\d+), (\d+)/.exec(color);
  if (match === null) {
    throw new Error(`not an rgb color: ${color}`);
  }
  return [Number(match[1]), Number(match[2]), Number(match[3])];
}

/** The contrast ratio of two colors, by the WCAG 2 formula (1 to 21). */
function contrast(first: string, second: string): number {
  const luminance = (color: string): number => {
    const [red, green, blue] = channels(color).map((channel) => {
      const linear = channel / 255;
      return linear <= 0.040_45 ? linear / 12.92 : ((linear + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * (red ?? 0) + 0.7152 * (green ?? 0) + 0.0722 * (blue ?? 0);
  };
  const [light, dark] = [luminance(first), luminance(second)].sort((left, right) => right - left);
  return ((light ?? 0) + 0.05) / ((dark ?? 0) + 0.05);
}

/** Errors of the page during the current test: uncaught errors, console errors, failed requests. */
let errors: string[] = [];

// Every error fails the test: nothing may go wrong silently, in the page, the worklet or
// the network (a missing favicon included). A test expecting a failure says which.
/** The errors the current test expects (its deliberate failure), left out of the check; undefined expects none. */
let expected: RegExp | undefined;
test.beforeEach(async ({ page }) => {
  // What the GPU draws is read in the task that draws it: see gpu-canvas.ts.
  await page.addInitScript(keepGpuCanvases);
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

/**
 * The start of the note the page shows when the browser's audio thread flushes subnormals,
 * written here: Firefox on x86_64 does, every other browser and every browser on aarch64
 * keeps IEEE arithmetic (spec/engine-design.md, decision 11).
 */
const FLUSHING = /^This browser rounds the smallest numbers to zero while it plays/;

/**
 * The playground plays through the limiter of the protected output stage at its default
 * ceiling, -1 dBTP (spec/engine-design.md, decision 12): the amplitude its sine is limited
 * to, computed here apart from the engine. No sample of it is above.
 */
const CEILING = 10 ** (-1 / 20);

/**
 * The lowest sample peak allowed, as a fraction of the ceiling: a period of the 440 Hz sine
 * is longer than the limiter's default window, where no bound is derived yet
 * (roadmap #24 step 112), so the allowance is twice the shortfall measured at every rate of
 * the test matrix (spec/measurements.md, "A period longer than the window").
 */
const LOWEST_PEAK = 0.98;

/** Waits until the page shows the sample peak of the limited sine: within the ceiling and the lowest peak allowed, to two decimals. */
async function expectLimitedPeak(page: Page): Promise<void> {
  const shown = async (): Promise<number> => Number(await page.getByTestId("peak").textContent());
  await expect.poll(shown).toBeLessThanOrEqual(Number(CEILING.toFixed(2)));
  await expect.poll(shown).toBeGreaterThanOrEqual(Number((LOWEST_PEAK * CEILING).toFixed(2)));
}

/** Whether `browserName` flushes subnormals on its audio thread on the machine running the tests, as measured. */
const flushesSubnormals = (browserName: string): boolean => browserName === "firefox" && process.arch === "x64";

test("plays the 440 Hz sine of the engine and shows it live, saying when the browser flushes subnormals", async ({ page, browserName }) => {
  await page.goto("./?muted");
  await expect(page).toHaveTitle("Playground · Poyetik");
  await expect(page.getByRole("status")).toHaveText("Stopped");
  await expect(page.getByTestId("frequency")).toHaveText("—");

  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Playing");
  await expect(page.getByTestId("frequency")).toHaveText("440.0 Hz");
  await expectLimitedPeak(page);
  await expect(page.getByTestId("rate")).toHaveText(/^\d+ Hz$/);
  await expect(page.getByText(FLUSHING)).toHaveCount(flushesSubnormals(browserName) ? 1 : 0);

  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByRole("status")).toHaveText("Stopped");
  await expect(page.getByTestId("frequency")).toHaveText("—");

  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByTestId("frequency")).toHaveText("440.0 Hz");
});

test("draws the limited waveform on the oscilloscope: its height, full width", async ({ page }) => {
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
  // A full-scale sine reaches ±0.9 of the half height; the limited sine, ±0.9 times its
  // peak, at least the lowest allowed: from 0.5 - 0.45 p to 0.5 + 0.45 p of the height,
  // within 2% of it for the drawing. The trace spans the width but for the sub-sample shift
  // at the left edge (at most 1%).
  const reach = 0.45 * LOWEST_PEAK * CEILING;
  await expect
    .poll(async () => {
      const { top, bottom, columns } = await extent();
      return top <= 0.5 - reach + 0.02 && bottom >= 0.5 + reach - 0.02 && columns >= 0.98;
    })
    .toBe(true);
});

test("is accessible", async ({ page }) => {
  await page.goto("./?muted");
  // Audited once the square is drawn, not while it loads.
  await expect(page.getByTestId("drawn-by")).toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Playing");
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
});

test("measures contrast by the WCAG formula", () => {
  expect(contrast("rgb(0, 0, 0)", "rgb(255, 255, 255)")).toBeCloseTo(21, 10);
  expect(contrast("rgb(255, 255, 255)", "rgba(0, 0, 0, 1)")).toBeCloseTo(21, 10);
  expect(contrast("rgb(118, 118, 118)", "rgb(255, 255, 255)")).toBeCloseTo(4.54, 2);
});

test("plays and stops from the keyboard, the button keeping a visible focus", async ({ page, browserName }) => {
  await page.goto("./?muted");
  const button = page.getByRole("button");
  await tabTo(page, button, browserName);
  // The focus ring: solid, at least 2 CSS pixels, at least 3:1 against the page (WCAG 2.4.7, 1.4.11).
  const ring = await button.evaluate((element) => {
    const style = getComputedStyle(element);
    return { style: style.outlineStyle, width: style.outlineWidth, color: style.outlineColor, page: getComputedStyle(document.documentElement).backgroundColor };
  });
  expect(ring.style).toBe("solid");
  expect(Number.parseFloat(ring.width)).toBeGreaterThanOrEqual(2);
  expect(contrast(ring.color, ring.page)).toBeGreaterThanOrEqual(3);
  await page.keyboard.press("Enter");
  await expect(page.getByRole("status")).toHaveText("Playing");
  await expect(button).toBeFocused();
  await expect(button).toHaveAccessibleName("Stop");
  await page.keyboard.press("Space");
  await expect(page.getByRole("status")).toHaveText("Stopped");
  await expect(button).toBeFocused();
});

test("ignores the second click of a double-click: after Stop, the tone stays stopped", async ({ page }) => {
  await trackAudio(page);
  // What the page saw: the detail of every click, so that the double-click is proven sent.
  await page.addInitScript(() => {
    const details: number[] = [];
    Reflect.set(window, "clicks", details);
    document.addEventListener("click", (event) => details.push(event.detail), true);
  });
  await page.goto("./?muted");
  const button = page.getByRole("button");
  const status = page.getByRole("status");
  await button.click();
  await expect(status).toHaveText("Playing");
  await button.click();
  await expect(status).toHaveText("Stopped");
  expect(await contexts(page)).toBe(1);
  // The second click of a double-click on Stop, now that it says Start.
  const box = await button.boundingBox();
  await page.mouse.move((box?.x ?? 0) + (box?.width ?? 0) / 2, (box?.y ?? 0) + (box?.height ?? 0) / 2);
  await page.mouse.down({ clickCount: 2 });
  await page.mouse.up({ clickCount: 2 });
  expect(await page.evaluate((): unknown => Reflect.get(window, "clicks"))).toStrictEqual([1, 1, 2]);
  // Control: a plain click starts, with one new context. Had the double-click started
  // the tone, this click would have cancelled it, and two contexts would have been made.
  await button.click();
  await expect(status).toHaveText("Playing");
  expect(await contexts(page)).toBe(2);
});

test("ignores Enter held down: the key repeat does not cancel the start", async ({ page }) => {
  // What the page saw: whether each Enter was a repeat, so that the repeat is proven sent.
  await page.addInitScript(() => {
    const repeats: boolean[] = [];
    Reflect.set(window, "repeats", repeats);
    document.addEventListener("keydown", (event) => repeats.push(event.repeat), true);
  });
  // The engine is held until released: the start stays in progress, the button says Cancel.
  let release = (): void => undefined;
  const loaded = new Promise<undefined>((resolve) => {
    release = () => {
      resolve(undefined);
    };
  });
  await page.route("**/*.wasm", async (route) => {
    await loaded;
    await route.continue();
  });
  await page.goto("./?muted");
  const button = page.getByRole("button");
  await button.focus();
  await page.keyboard.down("Enter");
  await expect(button).toHaveAccessibleName("Cancel");
  await page.keyboard.down("Enter");
  await page.keyboard.up("Enter");
  expect(await page.evaluate((): unknown => Reflect.get(window, "repeats"))).toStrictEqual([false, true]);
  // Had the repeat activated Cancel, the start would be over and could never play.
  release();
  await expect(page.getByRole("status")).toHaveText("Playing");
  // Control: Enter pressed again (no repeat) does activate the button.
  await page.keyboard.press("Enter");
  await expect(page.getByRole("status")).toHaveText("Stopped");
});

test("is accessible when the browser pauses the audio, and resumes", async ({ page }) => {
  await trackAudio(page);
  await page.goto("./?muted");
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Playing");
  await suspendAudio(page);
  await expect(page.getByRole("status")).toHaveText("Paused by the browser");
  await expect(page.getByTestId("frequency")).toHaveText("—");
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
  await page.getByRole("button", { name: "Resume" }).click();
  await expect(page.getByRole("status")).toHaveText("Playing");
  await expect(page.getByTestId("frequency")).toHaveText("440.0 Hz");
});

test("is accessible while starting, the engine still loading", async ({ page }) => {
  // The engine is held until axe has run: the start stays in progress.
  let release = (): void => undefined;
  const loaded = new Promise<undefined>((resolve) => {
    release = () => {
      resolve(undefined);
    };
  });
  await page.route("**/*.wasm", async (route) => {
    await loaded;
    await route.continue();
  });
  await page.goto("./?muted");
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Starting…");
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
  await expect(page.getByRole("status")).toHaveText("Starting…");
  release();
  await expect(page.getByRole("status")).toHaveText("Playing");
});

test("explains, accessibly, that a browser without AudioWorklet cannot play", async ({ page }) => {
  await page.addInitScript(() => {
    Reflect.deleteProperty(window, "AudioWorkletNode");
  });
  await page.goto("./?muted");
  // The emulation took effect: the page has no AudioWorkletNode.
  expect(await page.evaluate(() => "AudioWorkletNode" in window)).toBe(false);
  await expect(page.getByRole("status")).toHaveText("Not supported");
  await expect(page.getByRole("alert")).toHaveText(explain("audio-worklet"));
  const button = page.getByRole("button", { name: "Start" });
  await expect(button).toHaveAttribute("aria-disabled", "true");
  await expect(button).toHaveAttribute("aria-describedby", "player-alert");
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
});

test("says, accessibly, when the browser keeps the audio paused after Resume", async ({ page }) => {
  await trackAudio(page);
  await page.goto("./?muted");
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Playing");
  await suspendAudio(page);
  await expect(page.getByRole("status")).toHaveText("Paused by the browser");
  // The browser refuses to resume: the context answers, and stays suspended.
  await page.evaluate(() => {
    Reflect.set(Object(Reflect.get(window, "audio")), "resume", () => Promise.resolve());
  });
  await page.getByRole("button", { name: "Resume" }).click();
  await expect(page.getByRole("alert")).toHaveText(REFUSED);
  await expect(page.getByRole("status")).toHaveText("Paused by the browser");
  expect(await page.evaluate((): unknown => Reflect.get(Object(Reflect.get(window, "audio")), "state"))).toBe("suspended");
  await expect(page.getByRole("button", { name: "Resume" })).toHaveAttribute("aria-describedby", "player-alert");
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
});

/**
 * The layouts the button and the status are checked in: the narrowest width the page
 * supports, also in a font wider than any platform's default (the generic monospace,
 * about 0.6 em per character), since the fonts a platform falls back to decide whether
 * a status fits beside the button; and the default width.
 */
const LAYOUTS: readonly { readonly width?: number; readonly font?: string }[] = [{ width: 320 }, { width: 320, font: "monospace" }, {}];

for (const { width, font } of LAYOUTS) {
  test(`keeps the button a 44 px target, as wide whatever it says, and the status in place, at ${width === undefined ? "the default width" : `${String(width)} px`}${font === undefined ? "" : ` in ${font}`}`, async ({ page }) => {
    if (width !== undefined) {
      await page.setViewportSize({ width, height: 640 });
    }
    await trackAudio(page);
    // The engine is held until released: the start stays in progress (the button cancels it).
    let release = (): void => undefined;
    const loaded = new Promise<undefined>((resolve) => {
      release = () => {
        resolve(undefined);
      };
    });
    await page.route("**/*.wasm", async (route) => {
      await loaded;
      await route.continue();
    });
    await page.goto("./?muted");
    const button = page.getByRole("button");
    const status = page.getByRole("status");
    if (font !== undefined) {
      // Through the CSSOM, which the page's CSP allows. At the default size, 16 px, written
      // out: browsers shrink the default size of the generic monospace alone (to 13 px).
      // The emulation took effect.
      await page.evaluate((family) => {
        document.documentElement.style.font = `16px ${family}`;
      }, font);
      expect(await status.evaluate((element) => [getComputedStyle(element).fontFamily, getComputedStyle(element).fontSize])).toStrictEqual([font, "16px"]);
    }
    /** Where the button and the status are, and how large the button is. */
    const geometry = async (): Promise<{ width: number; height: number; status: number }> => {
      const box = await button.boundingBox();
      const text = await status.boundingBox();
      return { width: box?.width ?? 0, height: box?.height ?? 0, status: text?.x ?? 0 };
    };
    const seen = new Map<string, { width: number; height: number; status: number }>();
    seen.set("Start", await geometry());
    await button.click();
    await expect(button).toHaveAccessibleName("Cancel");
    seen.set("Cancel", await geometry());
    release();
    await expect(status).toHaveText("Playing");
    seen.set("Stop", await geometry());
    await suspendAudio(page);
    await expect(button).toHaveAccessibleName("Resume");
    seen.set("Resume", await geometry());
    expect([...seen.keys()].sort()).toStrictEqual([...BUTTON_LABELS].sort());
    const [first] = seen.values();
    for (const [label, measured] of seen) {
      expect(measured.height, label).toBeGreaterThanOrEqual(44);
      expect(measured, label).toStrictEqual(first);
    }
    if (font !== undefined) {
      // The control: on one line, the longest status is wider than the room beside the
      // button, so the layout above was put to the test.
      const [needed, room] = await status.evaluate((element) => {
        const context = document.createElement("canvas").getContext("2d");
        const controls = element.parentElement;
        const before = element.previousElementSibling;
        if (context === null || controls === null || before === null) {
          return [0, 0];
        }
        context.font = getComputedStyle(element).font;
        const gap = Number.parseFloat(getComputedStyle(controls).columnGap);
        return [context.measureText(element.textContent).width, controls.getBoundingClientRect().right - before.getBoundingClientRect().right - gap];
      });
      expect(await status.textContent()).toBe("Paused by the browser");
      expect(room).toBeGreaterThan(0);
      expect(needed).toBeGreaterThan(room);
    }
  });
}

test("explains a failure next to the button, accessibly, and can start again", async ({ page }) => {
  expected = /\.wasm|status of 404/;
  await page.route("**/*.wasm", (route) => route.fulfill({ status: 404, body: "" }));
  await page.goto("./?muted");
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Failed");
  await expect(page.getByRole("alert")).toHaveText(explain("load"));
  await expect(page.getByRole("button", { name: "Start" })).toHaveAttribute("aria-describedby", "player-alert");
  // The detail, for those who debug, is shown on demand and names what could not be loaded.
  const details = page.getByRole("group");
  await expect(details.locator("code")).toBeHidden();
  await details.getByText("Details").click();
  await expect(details.locator("code")).toHaveText(/^Error: cannot load the engine from \S+\.wasm \(HTTP 404\)$/);
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);

  await page.unroute("**/*.wasm");
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Playing");
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("says the square cannot be shown, accessibly, when the page's module cannot load", async ({ page }) => {
  expected = /page\.wasm|status of 404/;
  await page.route("**/page.wasm", (route) => route.fulfill({ status: 404, body: "" }));
  await page.goto("./?muted");
  await expect(page.getByTestId("square-failure")).toHaveText(FAILED);
  const details = page.getByRole("region", { name: "Square" }).getByRole("group");
  await details.getByText("Details").click();
  await expect(details.locator("code")).toHaveText(/page\.wasm \(HTTP 404\)$/);
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
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
    const seen: string[] = [];
    Reflect.set(window, "violations", seen);
    document.addEventListener("securitypolicyviolation", (event) => {
      seen.push(`${event.violatedDirective} ${event.blockedURI}`);
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

/**
 * The engines that apply forced colors when Playwright emulates them. WebKit matches the
 * media query but forces no color (it has no forced colors mode): there the test proves
 * nothing about the canvas, a known limit it states rather than passing silently.
 */
const FORCED_COLORS: Readonly<Record<string, boolean>> = { chromium: true, firefox: true, webkit: false };

test("keeps the colors of the oscilloscope and the square in forced colors (high contrast)", async ({ page, browserName }) => {
  // The page text follows the system colors there; each canvas keeps its own background
  // and border, against which its picture was designed (forced-color-adjust: none).
  await page.goto("./?muted");
  for (const canvas of [page.getByRole("img", { name: "Waveform of the engine output" }), page.locator("canvas.square")]) {
    await page.emulateMedia({ forcedColors: "none" });
    /** The background and border of the canvas, and the color of the page text: the control, which forced colors change. */
    const colors = (): Promise<{ canvas: string; text: string; forced: boolean }> =>
      canvas.evaluate((element) => ({
        canvas: `${getComputedStyle(element).backgroundColor} ${getComputedStyle(element).borderColor}`,
        text: getComputedStyle(document.body).color,
        forced: matchMedia("(forced-colors: active)").matches,
      }));
    const normal = await colors();
    expect(normal.forced).toBe(false);
    await page.emulateMedia({ forcedColors: "active", colorScheme: "light" });
    const forced = await colors();
    expect(forced.forced).toBe(true);
    // Control: the page text changed, so forced colors were applied (except where the engine
    // has no such mode), and the canvas kept its colors.
    expect(forced.text !== normal.text).toBe(FORCED_COLORS[browserName]);
    expect(forced.canvas).toBe(normal.canvas);
  }
});

for (const rate of SAMPLE_RATES) {
  test(`plays the 440 Hz sine on a device at ${String(rate)} Hz`, async ({ page }) => {
    await trackAudio(page, rate);
    await page.goto("./?muted");
    await page.getByRole("button", { name: "Start" }).click();
    await expect(page.getByTestId("rate")).toHaveText(`${String(rate)} Hz`);
    await expectLimitedPeak(page);
    // Measured only up to a twentieth of the rate: not at 8 kHz, where 440 Hz is above 400 Hz.
    await expect(page.getByTestId("frequency")).toHaveText(440 <= rate * MAX_FREQUENCY_RATIO ? "440.0 Hz" : "—");
  });
}

test("runs in a time zone and a locale of the test matrix, not the machine's", async ({ page }, testInfo) => {
  const { timezoneId, locale } = testInfo.project.use;
  expect(TIME_ZONES).toContain(timezoneId);
  expect(LOCALES).toContain(locale);
  await page.goto("./?muted");
  // The emulation took effect in the page itself.
  expect(await page.evaluate(() => Intl.DateTimeFormat().resolvedOptions().timeZone)).toBe(timezoneId);
  // Compared in their maximal form: WebKit drops a region its language implies ("ja-JP" reads "ja").
  const maximal = (tag: string): string => new Intl.Locale(tag).maximize().toString();
  const [language, formatting] = await page.evaluate(() => [navigator.language, Intl.DateTimeFormat().resolvedOptions().locale]);
  const wanted = maximal(locale ?? "");
  expect([maximal(language), maximal(formatting)]).toStrictEqual([wanted, wanted]);
});

/** The page's module, as its package builds it, with the square of the playground set up: the reference the canvas is compared with. */
async function squareReference(): Promise<PageModule> {
  return PageModule.instantiate(readFileSync(new URL(import.meta.resolve("@poyetik/web-page/page.wasm"))), SQUARE);
}

/** What the square's canvas shows and the audio clock says now; needs {@link trackAudio}. */
async function squareNow(page: Page): Promise<SquareNow> {
  return page.getByRole("img", { name: /^A white square on black, (sweeping from left to right|still)$/u }).evaluate(readSquare);
}

/**
 * The image of the square the canvas shows, searched among the `window` images up to
 * `latest` (the images repeat every sweep, two seconds: a window shorter than a sweep finds
 * one at most); undefined when none matches.
 */
function shownImage(reference: PageModule, pixels: string, latest: bigint, window: number): bigint | undefined {
  const shown = Buffer.from(pixels, "base64");
  for (let index = latest; index > latest - BigInt(window) && index >= 0n; index -= 1n) {
    const bytes = reference.display(index);
    if (shown.equals(new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.length))) {
      return index;
    }
  }
  return undefined;
}

test("shows the first image of the square until the sound starts, then the image of the audio clock", async ({ page, browserName }) => {
  await trackAudio(page);
  await page.goto("./?muted");
  await expect(page.getByText(MOVING)).toBeVisible();
  // Chromium always draws on its GPU (SwiftShader at least); the others as their GPU allows.
  const drawnBy = page.getByTestId("drawn-by");
  await expect(drawnBy).toHaveText(/^Drawn by the (GPU, through WebGPU|CPU, in WebAssembly)\.$/u);
  if (browserName === "chromium") {
    await expect(drawnBy).toHaveText(DRAWN_BY.gpu);
  }
  // Shown undistorted: the canvas's box has its images' proportions (to a pixel's rounding).
  const box = await page.getByRole("img", { name: /^A white square on black, (sweeping from left to right|still)$/u }).boundingBox();
  expect(Math.abs((box?.width ?? 0) / (box?.height ?? 1) - SQUARE.width / SQUARE.height), "16:9 on screen").toBeLessThan(0.02);
  const reference = await squareReference();
  const before = await squareNow(page);
  expect(shownImage(reference, before.pixels, 0n, 1), "the first image").toBe(0n);
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Playing");
  // The canvas shows the image of the clock at its last repaint, before the test reads both:
  // at most one repaint earlier (a sixtieth of a second, one image at 60 images a second, on
  // the 60 Hz displays the browsers emulate), and the clock crossing at most one more image
  // boundary between the repaint and the read: 0 to 2 images behind, never ahead. Measured
  // (2026-11-18, the machine loaded by another check): 0 or 1 in all four browsers.
  for (let sample = 0; sample < 20; sample += 1) {
    const { pixels, time, rate } = await squareNow(page);
    if (time === null || rate === null) {
      throw new Error("no audio clock while playing");
    }
    const clockImage = reference.imageAt(BigInt(Math.round(time * rate)), rate, SQUARE.rate);
    const shown = shownImage(reference, pixels, clockImage, 60);
    expect(shown, `an image of the square at most a second behind the clock, image ${String(clockImage)}`).toBeDefined();
    const lag = clockImage - (shown ?? clockImage + 1n);
    expect(lag >= 0n && lag <= 2n, `image ${String(shown)} shown at the clock's image ${String(clockImage)}`).toBe(true);
    await page.waitForTimeout(37);
  }
  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByRole("status")).toHaveText("Stopped");
  await expect.poll(async () => shownImage(reference, (await squareNow(page)).pixels, 0n, 1), "the first image again").toBe(0n);
});

test("keeps the square still, and says so, when the system asks for reduced motion", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("./?muted");
  await expect(page.getByText(STILL)).toBeVisible();
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Playing");
  const reference = await squareReference();
  for (let sample = 0; sample < 5; sample += 1) {
    expect(shownImage(reference, (await squareNow(page)).pixels, 0n, 1), "the first image, whatever the clock").toBe(0n);
    await page.waitForTimeout(100);
  }
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
  // The system stops asking: the square moves with the sound.
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await expect(page.getByText(MOVING)).toBeVisible();
  await expect.poll(async () => shownImage(reference, (await squareNow(page)).pixels, 0n, 1)).toBeUndefined();
});
