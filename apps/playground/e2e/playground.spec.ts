import { readFileSync } from "node:fs";
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { PageModule } from "@poyetik/web-page/page";
import { CSP } from "../csp.ts";
import { type SquareNow, clockSamples, recordRepaints, squareNow as readSquare } from "./audio-clock.ts";
import { displayCorpus } from "@poyetik/visual-wasm/testing/display-corpus";
import { encodeOnGpu } from "./display-encoding.ts";
import { BUFFER, BYTES, DISPLAY_BINDINGS, WORKGROUPS, displayGroups, displayParams } from "../src/gpu-square.ts";
import { keepGpuCanvases } from "./gpu-canvas.ts";
import { BUTTON_LABELS, FAILURE_DETAILS as SOUND_DETAILS, REFUSED } from "../src/App.tsx";
import { explain } from "../src/player.ts";
import { MAX_FREQUENCY_RATIO } from "../src/scope.ts";
import { COARSE_CLOCK_FROM, RENDER_QUANTUM } from "../src/audio-clock.ts";
import { SQUARE } from "../src/settings.ts";
import { DRAWN_BY, FAILED, FAILURE_DETAILS as SQUARE_DETAILS, LOGGED, MOVING, NAMES, REMEDY, STILL, UNPLAYABLE } from "../src/Square.tsx";
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
  // The audio clock each repaint saw, which the square's image is held to: see audio-clock.ts.
  await page.addInitScript(recordRepaints);
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
  // The square says it stays still, never a sweep this browser cannot show.
  await expect(page.getByRole("img", { name: NAMES.still, exact: true })).toHaveAccessibleDescription(UNPLAYABLE);
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
  // The engine's request refused, and the square's too, which the page logs as an error.
  expected = new RegExp(`\\.wasm|status of 404|^console error: ${LOGGED}`, "u");
  await page.route("**/*.wasm", (route) => route.fulfill({ status: 404, body: "" }));
  await page.goto("./?muted");
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Failed");
  await expect(page.getByRole("alert")).toHaveText(explain("load"));
  await expect(page.getByRole("button", { name: "Start" })).toHaveAttribute("aria-describedby", "player-alert");
  // The detail, for those who debug, is shown on demand and names what could not be loaded
  // (the square's own failure, its module refused too, has its own details).
  const details = page.getByRole("group").filter({ has: page.getByText(/cannot load the engine/u) });
  await expect(details.locator("code")).toBeHidden();
  await details.getByText(SOUND_DETAILS).click();
  // The square's module refused too: two disclosures, each named by what failed.
  expect(await page.locator("summary").allTextContents()).toStrictEqual([SOUND_DETAILS, SQUARE_DETAILS]);
  await expect(details.locator("code")).toHaveText(/^Error: cannot load the engine from \S+\.wasm \(HTTP 404\)$/);
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);

  await page.unroute("**/*.wasm");
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Playing");
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("says the square cannot be shown, accessibly, when the page module cannot load", async ({ page }) => {
  // The build names the module page-<hash>.wasm. The page logs the square's failure as an error.
  expected = new RegExp(`page[^/]*\\.wasm|status of 404|^console error: ${LOGGED}`, "u");
  await page.route(/\/page[^/]*\.wasm$/u, (route) => route.fulfill({ status: 404, body: "" }));
  await page.goto("./?muted");
  const line = `${FAILED} ${REMEDY.download}`;
  const announced = page.getByTestId("square-failure");
  await expect(announced).toHaveText(line);
  await expect(announced).toHaveAttribute("aria-live", "polite");
  // Said once, by the live region, and still describing the canvas.
  await expect(page.getByText(line)).toHaveCount(1);
  await expect(page.getByRole("img", { name: NAMES.failed })).toHaveAccessibleDescription(line);
  // The box's own black, never an empty gray box.
  await expect(page.getByRole("img", { name: NAMES.failed })).toHaveCSS("background-color", "rgb(0, 0, 0)");
  const details = page.getByRole("region", { name: "Square" }).getByRole("group");
  await details.getByText(SQUARE_DETAILS).click();
  await expect(details.locator("code")).toHaveText(/page[^/]*\.wasm \(HTTP 404\)$/u);
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
  // Logged once, as the error it is: the load's own, with its cause.
  await expect.poll(() => errors.filter((error) => error.startsWith(`console error: ${LOGGED}`))).toStrictEqual([`console error: ${LOGGED}${String(await details.locator("code").textContent())}`]);
});

test("shows the square's black before its first image, and when it cannot be shown", async ({ page }) => {
  // The page module is held until released: the square stays loading.
  let release = (): void => undefined;
  const loaded = new Promise<undefined>((resolve) => {
    release = () => {
      resolve(undefined);
    };
  });
  await page.route(/\/page[^/]*\.wasm$/u, async (route) => {
    await loaded;
    await route.continue();
  });
  await page.goto("./?muted");
  const loading = page.getByRole("img", { name: NAMES.loading, exact: true });
  await expect(loading).toHaveCSS("background-color", "rgb(0, 0, 0)");
  release();
  await expect(page.getByRole("img", { name: NAMES.still, exact: true })).toHaveCSS("background-color", "rgb(0, 0, 0)");
  // Control: the oscilloscope's canvas keeps the page's surface, not the square's black.
  await expect(page.getByRole("img", { name: "Waveform of the engine output" })).not.toHaveCSS("background-color", "rgb(0, 0, 0)");
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

/** The page module, as its package builds it, with the square of the playground set up: the reference the canvas is compared with. */
async function squareReference(): Promise<PageModule> {
  return PageModule.instantiate(readFileSync(new URL(import.meta.resolve("@poyetik/web-page/page.wasm"))), SQUARE);
}

/**
 * What the square's canvas shows and the audio clock says now, the canvas found by its exact
 * `name` (the name of the phase the test expects); needs {@link trackAudio}.
 */
async function squareNow(page: Page, name: string): Promise<SquareNow> {
  return page.getByRole("img", { name, exact: true }).evaluate(readSquare);
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

/**
 * Holds that the square follows the audio clock at every repaint, `samples` times 37 ms
 * apart, once it first does: the image the canvas shows is exactly the image of the clock
 * its last repaint read, which lies between the clock read before the repaint's first
 * callback and after its last (`recordRepaints`), however slowly the machine repaints and
 * however the browser coarsens its clock; and the images shown change, so that a canvas
 * stuck on one image cannot pass. Needs {@link trackAudio} before the page loads.
 */
async function followsTheClock(page: Page, reference: PageModule, samples = 20): Promise<void> {
  /** The image shown now, and the images of the clock around the last repaint; null before a repaint saw a running clock. */
  const read = async (): Promise<{ shown: bigint | undefined; earliest: bigint; latest: bigint } | null> => {
    const { pixels, repaint } = await squareNow(page, NAMES.moving);
    if (repaint === null) {
      return null;
    }
    const image = (time: number): bigint => reference.imageAt(BigInt(Math.round(time * repaint.rate)), repaint.rate, SQUARE.rate);
    const [earliest, latest] = [image(repaint.before), image(repaint.after)];
    return { shown: shownImage(reference, pixels, latest, 60), earliest, latest };
  };
  /** Whether `now` shows an image of its repaint's clock. */
  const follows = (now: Awaited<ReturnType<typeof read>>): boolean => now?.shown !== undefined && now.shown >= now.earliest && now.shown <= now.latest;
  // The first repaints after the start: once one has drawn the clock's image.
  await expect.poll(async () => follows(await read()), "the square drawing the clock's image").toBe(true);
  const shown = new Set<bigint | undefined>();
  for (let sample = 0; sample < samples; sample += 1) {
    const now = await read();
    expect(follows(now), `image ${String(now?.shown)} shown, the repaint's clock at images ${String(now?.earliest)} to ${String(now?.latest)}`).toBe(true);
    shown.add(now?.shown);
    await page.waitForTimeout(37);
  }
  expect(shown.size, "the square moved").toBeGreaterThan(1);
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
  const box = await page.getByRole("img", { name: NAMES.still, exact: true }).boundingBox();
  expect(Math.abs((box?.width ?? 0) / (box?.height ?? 1) - SQUARE.width / SQUARE.height), "16:9 on screen").toBeLessThan(0.02);
  const reference = await squareReference();
  const before = await squareNow(page, NAMES.still);
  expect(shownImage(reference, before.pixels, 0n, 1), "the first image").toBe(0n);
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Playing");
  // The image of the clock at the last repaint, exactly: never behind by a guessed bound.
  await followsTheClock(page, reference);
  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByRole("status")).toHaveText("Stopped");
  await expect.poll(async () => shownImage(reference, (await squareNow(page, NAMES.still)).pixels, 0n, 1), "the first image again").toBe(0n);
});

test("keeps the square still, and says so, when the system asks for reduced motion", async ({ page }) => {
  await trackAudio(page);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("./?muted");
  await expect(page.getByText(STILL)).toBeVisible();
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).toHaveText("Playing");
  const reference = await squareReference();
  for (let sample = 0; sample < 5; sample += 1) {
    expect(shownImage(reference, (await squareNow(page, NAMES.still)).pixels, 0n, 1), "the first image, whatever the clock").toBe(0n);
    await page.waitForTimeout(100);
  }
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
  // The system stops asking: the square moves with the sound, following its clock.
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await expect(page.getByText(MOVING)).toBeVisible();
  await followsTheClock(page, reference);
});

/**
 * The engines that report their audio clock rounded to 1 ms from {@link COARSE_CLOCK_FROM}
 * on, as `src/audio-clock.ts` and `spec/limitations.md` document: Firefox only. Below it,
 * and in every other engine, the clock counts whole render quanta.
 */
const COARSENS: Readonly<Record<string, boolean>> = { chromium: false, firefox: true, webkit: false };

for (const rate of [COARSE_CLOCK_FROM, ...SAMPLE_RATES.filter((each) => each > COARSE_CLOCK_FROM)]) {
  test(`reads the audio clock at ${String(rate)} Hz as documented, and the square follows it`, async ({ page, browserName }) => {
    await trackAudio(page, rate);
    await page.goto("./?muted");
    await page.getByRole("button", { name: "Start" }).click();
    await expect(page.getByRole("status")).toHaveText("Playing");
    await expect(page.getByTestId("rate")).toHaveText(`${String(rate)} Hz`);
    const times = await page.evaluate(clockSamples, { count: 40, apart: 7 });
    /** Whether `value` is a whole number, to the rounding of a time in binary64. */
    const whole = (value: number): boolean => Math.abs(value - Math.round(value)) < 1e-6;
    const quanta = times.every((time) => whole((time * rate) / RENDER_QUANTUM));
    const milliseconds = times.every((time) => whole(time * 1000));
    if (COARSENS[browserName] === true) {
      expect(milliseconds, `every time a whole millisecond: ${times.join(", ")}`).toBe(true);
    } else {
      expect(quanta, `every time whole render quanta: ${times.join(", ")}`).toBe(true);
      // Above the boundary a quantum is shorter than 1 ms: a clock not coarsened shows times between milliseconds.
      expect(milliseconds, `some time between milliseconds: ${times.join(", ")}`).toBe(rate === COARSE_CLOCK_FROM);
    }
    await followsTheClock(page, await squareReference());
  });
}

/**
 * The display corpus (`conformance/visual/display.json`, which `visual-cli bless` writes),
 * read by the one strict reader every TypeScript test shares: each pixel's four values as
 * bits, and the four bytes the native encoding gives them.
 */
function corpus(): { readonly bits: number[]; readonly bytes: number[] } {
  const { values, bytes } = displayCorpus(readFileSync(new URL("../../../conformance/visual/display.json", import.meta.url), "utf8"));
  return { bits: Array.from(new Uint32Array(values.buffer, values.byteOffset, values.length)), bytes: Array.from(bytes) };
}

/** The byte the GPU's output holds before the dispatch: a pixel the shader leaves unwritten shows it, never a zero a new buffer holds. */
const SENTINEL = 0xa5;

test("encodes the display corpus on the browser's GPU to the native bytes, every pixel", async ({ page, browserName }) => {
  // The library's display shader, compiled by this browser's own WGSL compiler, over values
  // no square holds (translucent, NaN, infinities, subnormals, negatives, above 1): every
  // path of the encoding, which the square shown reaches only opaque.
  await page.goto("./?muted");
  const reference = await squareReference();
  const { bits, bytes } = corpus();
  const pixels = bits.length / 4;
  const layout = { usage: BUFFER, params: Array.from(displayParams(pixels)), groups: displayGroups(pixels), bindings: DISPLAY_BINDINGS, pixelBytes: BYTES.displayPixel };
  // The corpus ends inside a workgroup, whose last invocations must write nothing: a shader
  // whose bound stopped a pixel early would leave the last pixel's sentinel bytes, which
  // its expected bytes are not.
  expect(pixels % WORKGROUPS.display, "a partial last workgroup").not.toBe(0);
  expect(bytes.slice(-4).every((byte) => byte === SENTINEL), "the last pixel's bytes are not the sentinel").toBe(false);
  const encoded = await page.evaluate(encodeOnGpu, { shader: reference.shader("display"), thresholds: Array.from(reference.thresholds()), bits, layout, sentinel: SENTINEL });
  if (typeof encoded === "string") {
    // Chromium always has a GPU (SwiftShader at least); the others only as their machine allows.
    expect(browserName, encoded).not.toBe("chromium");
    expect(encoded).toMatch(/^no (WebGPU|adapter)$/u);
    return;
  }
  expect(encoded).toHaveLength(bytes.length);
  const first = encoded.findIndex((byte, at) => byte !== bytes[at]);
  const pixel = Math.floor(first / 4);
  expect(first, `pixel ${String(pixel)}: values ${bits.slice(pixel * 4, pixel * 4 + 4).map((word) => word.toString(16)).join(" ")}, bytes ${encoded.slice(pixel * 4, pixel * 4 + 4).join(" ")} for ${bytes.slice(pixel * 4, pixel * 4 + 4).join(" ")}`).toBe(-1);
});
