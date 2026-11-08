import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { App, REFUSED } from "../src/App.tsx";
import { explain, type Player, type PlayerState, type Signal } from "../src/player.ts";

/** A player whose state the test drives, recording the calls. */
class FakePlayer implements Player {
  /** Names of the player methods the page called, in order. */
  readonly calls: string[] = [];
  /** What {@link FakePlayer.latest} returns: the test sets it to feed the oscilloscope. */
  signal: Signal | undefined = undefined;
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

/** The single button of the page, Start or Stop; throws when there is none or several. */
const button = (): HTMLButtonElement => screen.getByRole("button");
/** The text of the status region: the state the page announces. */
const status = (): string | null => screen.getByRole("status").textContent;

afterEach(() => {
  vi.useRealTimers();
});

test("starts stopped", () => {
  render(<App player={new FakePlayer()} />);
  expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Playground");
  expect(button().textContent).toBe("Start");
  expect(button().getAttribute("aria-disabled")).toBe("false");
  expect(status()).toBe("Stopped");
  expect(screen.queryByRole("alert")).toBeNull();
});

test("warns before the button that it plays a continuous tone", () => {
  render(<App player={new FakePlayer()} />);
  const warning = screen.getByText("Plays a continuous tone: lower your volume first.");
  expect(warning.compareDocumentPosition(button()) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
});

test.each([
  [{ kind: "stopped" }, "Stopped", "Start", "false", "start", null],
  [{ kind: "starting" }, "Starting…", "Cancel", "false", "stop", null],
  [{ kind: "playing" }, "Playing", "Stop", "false", "stop", null],
  [{ kind: "suspended", refused: false }, "Paused by the browser", "Resume", "false", "resume", null],
  [{ kind: "suspended", refused: true }, "Paused by the browser", "Resume", "false", "resume", REFUSED],
  [{ kind: "stopping" }, "Stopping…", "Stop", "true", undefined, null],
  [{ kind: "failed", cause: "device", detail: "NotSupportedError: no device" }, "Failed", "Start", "false", "start", explain("device")],
  [{ kind: "unsupported", lacking: "web-audio" }, "Not supported", "Start", "true", undefined, explain("web-audio")],
] as const)("shows %o and acts on it", (state, shown, label, disabled, call, alert) => {
  const player = new FakePlayer();
  render(<App player={player} />);
  player.become(state);
  expect(status()).toBe(shown);
  expect(button().textContent).toBe(label);
  expect(button().getAttribute("aria-disabled")).toBe(disabled);
  expect(button().dataset["action"]).toBe(label.toLowerCase());
  expect(screen.queryByRole("alert")?.textContent ?? null).toBe(alert);
  fireEvent.click(button());
  expect(player.calls).toStrictEqual(call === undefined ? [] : [call]);
});

test("keeps the button focused through every transition", () => {
  const player = new FakePlayer();
  render(<App player={player} />);
  button().focus();
  for (const kind of ["starting", "playing", "stopping", "stopped"] as const) {
    player.become({ kind });
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
  fireEvent.click(button());
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
