import { act, fireEvent, render, screen } from "@testing-library/react";
import axe from "axe-core";
import { afterEach, expect, test, vi } from "vitest";
import type { SubnormalArithmetic } from "@poyetik/engine-wasm/web";
import { App, BUTTON_LABELS, FLUSHING, REFUSED } from "../src/App.tsx";
import { explain, type Failure, type Missing, type Player, type PlayerState, type Signal } from "../src/player.ts";

/** A player whose state the test drives, recording the calls. */
class FakePlayer implements Player {
  /** Names of the player methods the page called, in order. */
  readonly calls: string[] = [];
  /** What {@link FakePlayer.latest} returns: the test sets it to feed the oscilloscope. */
  signal: Signal | undefined = undefined;
  /** What {@link FakePlayer.arithmetic} returns: the test sets it, before the change of state that shows it. */
  subnormals: SubnormalArithmetic | undefined = undefined;
  /** The state the test last moved to with {@link FakePlayer.become}. */
  #state: PlayerState = { kind: "stopped" };
  /** The page's subscriptions, told by {@link FakePlayer.become}. */
  readonly #listeners = new Set<() => void>();

  /** Records the call and resolves at once: the state changes only through {@link FakePlayer.become}. */
  start(): Promise<void> {
    this.calls.push("start");
    return Promise.resolve();
  }

  /** Records the call and resolves at once: the state changes only through {@link FakePlayer.become}. */
  stop(): Promise<void> {
    this.calls.push("stop");
    return Promise.resolve();
  }

  /** Records the call and resolves at once: the state changes only through {@link FakePlayer.become}. */
  resume(): Promise<void> {
    this.calls.push("resume");
    return Promise.resolve();
  }

  /** The state set by the test, the same object until the next {@link FakePlayer.become}, as React's snapshots require. */
  state(): PlayerState {
    return this.#state;
  }

  /** Adds `listener` to those told on every change; returns how to remove it. */
  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  /** The signal the test set, or undefined. */
  latest(): Signal | undefined {
    return this.signal;
  }

  /** The arithmetic the test set, or undefined. */
  arithmetic(): SubnormalArithmetic | undefined {
    return this.subnormals;
  }

  /** Moves to `state`, as the real player would, and tells the page. */
  become(state: PlayerState): void {
    act(() => {
      this.#state = state;
      this.#listeners.forEach((listener) => {
        listener();
      });
    });
  }
}

/** The single button of the page; throws when there is none or several. */
const button = (): HTMLButtonElement => screen.getByRole("button");
/** What the button says: its visible label, not the invisible copies that size it. */
const label = (): string | null | undefined => button().querySelector(":scope > span:not([aria-hidden])")?.textContent;
/** The text of the status region: the state the page announces. */
const status = (): string | null => screen.getByRole("status").textContent;

afterEach(() => {
  vi.useRealTimers();
});

test("starts stopped", () => {
  render(<App player={new FakePlayer()} />);
  expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Playground");
  expect(label()).toBe("Start");
  expect(button().getAttribute("aria-disabled")).toBe("false");
  expect(status()).toBe("Stopped");
  expect(screen.queryByRole("alert")).toBeNull();
});

test("sizes the button by every label it can say, invisibly", () => {
  render(<App player={new FakePlayer()} />);
  const sizers = [...button().querySelectorAll(":scope > span[aria-hidden='true'].sizer")].map((sizer) => sizer.textContent);
  expect(sizers).toStrictEqual([...BUTTON_LABELS]);
  expect(BUTTON_LABELS).toStrictEqual(["Start", "Cancel", "Stop", "Resume"]);
});

test("warns before the button that it plays a continuous tone", () => {
  render(<App player={new FakePlayer()} />);
  const warning = screen.getByText("Plays a continuous tone: lower your volume first.");
  expect(warning.compareDocumentPosition(button()) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
});

/**
 * Clicks the button, as the `detail`-th click of a series (0: a keyboard activation, 2:
 * the second click of a double-click), and returns what its handler threw: React reports
 * it on the window, where it is caught here, so that a handler calling an action that is
 * not there fails this test instead of crashing the test runner.
 */
function click(detail = 1): unknown[] {
  const thrown: unknown[] = [];
  const listener = (event: ErrorEvent): void => {
    thrown.push(event.error);
    event.preventDefault();
  };
  window.addEventListener("error", listener);
  try {
    fireEvent.click(button(), { detail });
  } finally {
    window.removeEventListener("error", listener);
  }
  return thrown;
}

test.each([
  [{ kind: "stopped" }, "Stopped", "Start", "false", "start", null],
  [{ kind: "starting" }, "Starting…", "Cancel", "false", "stop", null],
  [{ kind: "playing" }, "Playing", "Stop", "false", "stop", null],
  [{ kind: "suspended", refused: false }, "Paused by the browser", "Resume", "false", "resume", null],
  [{ kind: "suspended", refused: true }, "Paused by the browser", "Resume", "false", "resume", REFUSED],
  [{ kind: "stopping" }, "Stopping…", "Stop", "true", undefined, null],
  [{ kind: "failed", cause: "device", detail: "NotSupportedError: no device" }, "Failed", "Start", "false", "start", explain("device")],
  [{ kind: "unsupported", lacking: "web-audio" }, "Not supported", "Start", "true", undefined, explain("web-audio")],
] as const)("shows %o and acts on it", (state, shown, said, disabled, call, alert) => {
  const player = new FakePlayer();
  render(<App player={player} />);
  player.become(state);
  expect(status()).toBe(shown);
  expect(label()).toBe(said);
  expect(screen.getByRole("button", { name: said })).toBe(button());
  expect(button().getAttribute("aria-disabled")).toBe(disabled);
  expect(button().dataset["action"]).toBe(said.toLowerCase());
  expect(screen.queryByRole("alert")?.textContent ?? null).toBe(alert);
  // The second click of a double-click does nothing: it would undo the first (a tone
  // starting again right after Stop).
  expect(click(2)).toStrictEqual([]);
  expect(click(3)).toStrictEqual([]);
  expect(player.calls).toStrictEqual([]);
  expect(click(1)).toStrictEqual([]);
  expect(player.calls).toStrictEqual(call === undefined ? [] : [call]);
  // A keyboard activation is a click of detail 0.
  expect(click(0)).toStrictEqual([]);
  expect(player.calls).toStrictEqual(call === undefined ? [] : [call, call]);
});

test("ignores Enter and Space held down: a repeated key does not activate the button again", () => {
  render(<App player={new FakePlayer()} />);
  // fireEvent returns false when the handler prevented the default action (the activation).
  expect(fireEvent.keyDown(button(), { key: "Enter", repeat: true })).toBe(false);
  expect(fireEvent.keyDown(button(), { key: " ", repeat: true })).toBe(false);
  expect(fireEvent.keyDown(button(), { key: "Enter", repeat: false })).toBe(true);
  expect(fireEvent.keyDown(button(), { key: " ", repeat: false })).toBe(true);
  expect(fireEvent.keyDown(button(), { key: "a", repeat: true })).toBe(true);
});

test("keeps the button focused through every transition, into every state", () => {
  // A record: a new kind of state left out of this test does not compile.
  const every: Readonly<Record<PlayerState["kind"], PlayerState>> = {
    starting: { kind: "starting" },
    playing: { kind: "playing" },
    suspended: { kind: "suspended", refused: false },
    stopping: { kind: "stopping" },
    stopped: { kind: "stopped" },
    failed: { kind: "failed", cause: "engine", detail: "Error: trapped" },
    unsupported: { kind: "unsupported", lacking: "webassembly" },
  };
  const player = new FakePlayer();
  render(<App player={player} />);
  button().focus();
  for (const state of Object.values(every)) {
    player.become(state);
    expect(document.activeElement).toBe(button());
  }
});

test("explains a failure next to the button, and describes the button with it; the detail only on demand", () => {
  const player = new FakePlayer();
  render(<App player={player} />);
  player.become({ kind: "failed", cause: "load", detail: "Error: cannot load the engine from engine.wasm (HTTP 404)" });
  const alert = screen.getByRole("alert");
  expect(alert.textContent).toBe(explain("load"));
  expect(alert.id).toBe("player-alert");
  expect(button().getAttribute("aria-describedby")).toBe(alert.id);
  const details = screen.getByRole("group");
  expect(details.tagName).toBe("DETAILS");
  expect(details.hasAttribute("open")).toBe(false);
  expect(details.textContent).toBe("DetailsError: cannot load the engine from engine.wasm (HTTP 404)");
  expect(alert.textContent).not.toContain("HTTP 404");
  player.become({ kind: "starting" });
  expect(screen.queryByRole("alert")).toBeNull();
  expect(screen.queryByRole("group")).toBeNull();
  expect(button().hasAttribute("aria-describedby")).toBe(false);
});

test("explains on load what the browser lacks, with no way to start", () => {
  const player = new FakePlayer();
  player.become({ kind: "unsupported", lacking: "secure-context" });
  render(<App player={player} />);
  expect(screen.getByRole("alert").textContent).toBe(explain("secure-context"));
  expect(button().getAttribute("aria-disabled")).toBe("true");
  expect(screen.queryByRole("group")).toBeNull();
  expect(click()).toStrictEqual([]);
  expect(player.calls).toStrictEqual([]);
});

test("says when the browser kept the audio paused after Resume, and only then", () => {
  const player = new FakePlayer();
  render(<App player={player} />);
  player.become({ kind: "suspended", refused: false });
  expect(screen.queryByRole("alert")).toBeNull();
  expect(button().hasAttribute("aria-describedby")).toBe(false);
  player.become({ kind: "suspended", refused: true });
  const alert = screen.getByRole("alert");
  expect(alert.textContent).toBe(REFUSED);
  expect(button().getAttribute("aria-describedby")).toBe(alert.id);
  expect(REFUSED).toBe("The browser kept audio paused. Try again, or check that this tab may play sound.");
});

/** Every subnormal kept, in both formats: IEEE arithmetic, written as a literal. */
const IEEE: SubnormalArithmetic = { binary32: { operands: true, results: true }, binary64: { operands: true, results: true } };

/** IEEE arithmetic but for one of its four observations, each in turn: every way to flush some subnormals alone. */
const ONE_FLUSHED: readonly SubnormalArithmetic[] = [
  { ...IEEE, binary32: { operands: false, results: true } },
  { ...IEEE, binary32: { operands: true, results: false } },
  { ...IEEE, binary64: { operands: false, results: true } },
  { ...IEEE, binary64: { operands: true, results: false } },
];

test.each([undefined, IEEE])("says nothing of the arithmetic when it is unknown or IEEE: %o", (subnormals) => {
  const player = new FakePlayer();
  player.subnormals = subnormals;
  render(<App player={player} />);
  player.become({ kind: "playing" });
  expect(screen.queryByText(FLUSHING)).toBeNull();
});

test.each(ONE_FLUSHED)("says that the tone is not guaranteed bit-identical when the browser flushes subnormals: %o", (subnormals) => {
  const player = new FakePlayer();
  render(<App player={player} />);
  player.become({ kind: "starting" });
  expect(screen.queryByText(FLUSHING)).toBeNull();
  player.subnormals = subnormals;
  player.become({ kind: "playing" });
  expect(screen.getByText(FLUSHING).tagName).toBe("P");
});

test("is accessible with the note on the arithmetic shown", async () => {
  const player = new FakePlayer();
  player.subnormals = { binary32: { operands: false, results: false }, binary64: { operands: false, results: false } };
  render(<App player={player} />);
  player.become({ kind: "playing" });
  expect(screen.getByText(FLUSHING).tagName).toBe("P");
  expect(await audit()).toStrictEqual({ violations: [], incomplete: UNDECIDED });
});

test("repaints the oscilloscope only while playing", () => {
  vi.useFakeTimers({ toFake: ["requestAnimationFrame", "cancelAnimationFrame"] });
  const player = new FakePlayer();
  render(<App player={player} />);
  expect(vi.getTimerCount()).toBe(0);
  player.become({ kind: "playing" });
  expect(vi.getTimerCount()).toBe(1);
  player.become({ kind: "suspended", refused: false });
  expect(vi.getTimerCount()).toBe(0);
});

test("shows what the player plays", () => {
  vi.useFakeTimers({ toFake: ["requestAnimationFrame", "cancelAnimationFrame"] });
  const player = new FakePlayer();
  player.signal = { samples: new Float32Array([0.5, -0.5]), rate: 8000 };
  render(<App player={player} />);
  act(() => {
    vi.advanceTimersToNextFrame();
  });
  expect(screen.getByTestId("peak").textContent).toBe("0.50");
  expect(screen.getByTestId("rate").textContent).toBe("8000 Hz");
});

test("follows a new player", () => {
  vi.useFakeTimers({ toFake: ["requestAnimationFrame", "cancelAnimationFrame"] });
  const first = new FakePlayer();
  first.signal = { samples: new Float32Array([0.5]), rate: 8000 };
  const second = new FakePlayer();
  second.signal = { samples: new Float32Array([0.25]), rate: 16_000 };
  const { rerender } = render(<App player={first} />);
  rerender(<App player={second} />);
  act(() => {
    vi.advanceTimersToNextFrame();
  });
  expect(screen.getByTestId("rate").textContent).toBe("16000 Hz");
});

test("follows the state of a new player", () => {
  const first = new FakePlayer();
  const second = new FakePlayer();
  const { rerender } = render(<App player={first} />);
  rerender(<App player={second} />);
  second.become({ kind: "playing" });
  expect(status()).toBe("Playing");
});

/**
 * Every state the page can show, by kind, each in every variant it renders differently:
 * a record of records, so that a new kind of state, failure or lack left out of the
 * accessibility audit does not compile.
 */
const EVERY_STATE: Readonly<Record<PlayerState["kind"], readonly PlayerState[]>> = {
  stopped: [{ kind: "stopped" }],
  starting: [{ kind: "starting" }],
  playing: [{ kind: "playing" }],
  suspended: [
    { kind: "suspended", refused: false },
    { kind: "suspended", refused: true },
  ],
  stopping: [{ kind: "stopping" }],
  failed: Object.values({
    load: { kind: "failed", cause: "load", detail: "Error: cannot load the engine from engine.wasm (HTTP 404)" },
    engine: { kind: "failed", cause: "engine", detail: "Error: trapped" },
    device: { kind: "failed", cause: "device", detail: "NotSupportedError: no device" },
  } satisfies Record<Failure, PlayerState>),
  unsupported: Object.values({
    "secure-context": { kind: "unsupported", lacking: "secure-context" },
    "web-audio": { kind: "unsupported", lacking: "web-audio" },
    "audio-worklet": { kind: "unsupported", lacking: "audio-worklet" },
    webassembly: { kind: "unsupported", lacking: "webassembly" },
  } satisfies Record<Missing, PlayerState>),
};

/**
 * What jsdom leaves axe unable to decide, whatever the state: with no layout, it cannot
 * tell whether the main landmark and the heading are visible. The real page is audited
 * end to end, where both are decided.
 */
const UNDECIDED = ["landmark-one-main", "page-has-heading-one"];

/**
 * Audits the whole document with axe, as the page sets it up (its language and title):
 * the violations, each as `rule: offending markup`, and the rules axe could not decide.
 * jsdom has no layout, so color contrast is left to the end-to-end audits.
 */
async function audit(): Promise<{ violations: string[]; incomplete: string[] }> {
  document.documentElement.lang = "en";
  document.title = "Playground · Poyetik";
  const results = await axe.run(document.documentElement, { rules: { "color-contrast": { enabled: false } } });
  return {
    violations: results.violations.map((violation) => `${violation.id}: ${violation.nodes.map((node) => node.html).join(" | ")}`),
    incomplete: results.incomplete.map((rule) => rule.id).sort(),
  };
}

test.each(Object.values(EVERY_STATE).flat())("is accessible in %o", async (state) => {
  const player = new FakePlayer();
  render(<App player={player} />);
  player.become(state);
  expect(await audit()).toStrictEqual({ violations: [], incomplete: UNDECIDED });
  // A failure also shows its detail on demand: audited open too.
  const details = screen.queryByRole("group");
  expect(details !== null).toBe(state.kind === "failed");
  if (details instanceof HTMLDetailsElement) {
    details.open = true;
    expect(await audit()).toStrictEqual({ violations: [], incomplete: UNDECIDED });
  }
});

test("finds a duplicate id, so that a clean audit proves something", async () => {
  const player = new FakePlayer();
  render(<App player={player} />);
  player.become({ kind: "failed", cause: "load", detail: "Error: no network" });
  const copy = document.createElement("p");
  copy.id = "player-alert";
  document.body.append(copy);
  try {
    // axe leaves a duplicate id referred to by ARIA for review, not as a violation: hence
    // the exact list of undecided rules every audit checks.
    expect(await audit()).toStrictEqual({ violations: [], incomplete: ["duplicate-id-aria", ...UNDECIDED] });
  } finally {
    copy.remove();
  }
});
