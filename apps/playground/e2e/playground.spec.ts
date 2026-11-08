import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { CSP } from "../csp.ts";
import { BUTTON_LABELS, REFUSED } from "../src/App.tsx";
import { explain } from "../src/player.ts";
import { MAX_FREQUENCY_RATIO } from "../src/scope.ts";
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
  await expect(page).toHaveTitle("Playground · Poyetik");
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

for (const width of [320, undefined]) {
  test(`keeps the button a 44 px target, as wide whatever it says, at ${width === undefined ? "the default width" : `${String(width)} px`}`, async ({ page }) => {
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

test("keeps the oscilloscope colors in forced colors (high contrast)", async ({ page, browserName }) => {
  // The page text follows the system colors there; the canvas keeps its own background,
  // against which its trace was designed (forced-color-adjust: none).
  await page.goto("./?muted");
  const canvas = page.getByRole("img", { name: "Waveform of the engine output" });
  /** The background of the canvas, and the color of the page text: the control, which forced colors change. */
  const colors = (): Promise<{ canvas: string; text: string; forced: boolean }> =>
    canvas.evaluate((element) => ({
      canvas: getComputedStyle(element).backgroundColor,
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
});

for (const rate of SAMPLE_RATES) {
  test(`plays the 440 Hz sine on a device at ${String(rate)} Hz`, async ({ page }) => {
    await trackAudio(page, rate);
    await page.goto("./?muted");
    await page.getByRole("button", { name: "Start" }).click();
    await expect(page.getByTestId("rate")).toHaveText(`${String(rate)} Hz`);
    await expect(page.getByTestId("peak")).toHaveText("1.00");
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
