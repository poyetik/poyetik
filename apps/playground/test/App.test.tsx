import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { App } from "../src/App.tsx";
import type { Player, Signal } from "../src/player.ts";

/** A promise settled by the test. */
interface Pending {
  readonly promise: Promise<void>;
  readonly resolve: () => void;
  readonly reject: (reason: unknown) => void;
}

/** A pending promise. */
function pending(): Pending {
  let resolve: () => void = () => undefined;
  let reject: (reason: unknown) => void = () => undefined;
  const promise = new Promise<void>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

/** A player whose calls the test settles. */
class FakePlayer implements Player {
  readonly calls: string[] = [];
  readonly starts: Pending[] = [];
  readonly stops: Pending[] = [];
  signal: Signal | undefined = undefined;

  start(): Promise<void> {
    this.calls.push("start");
    const call = pending();
    this.starts.push(call);
    return call.promise;
  }

  stop(): Promise<void> {
    this.calls.push("stop");
    const call = pending();
    this.stops.push(call);
    return call.promise;
  }

  latest(): Signal | undefined {
    return this.signal;
  }
}

/** Settles `call` and lets React apply what follows. */
async function settle(action: () => void): Promise<void> {
  await act(async () => {
    action();
    await Promise.resolve();
  });
}

const button = (): HTMLButtonElement => screen.getByRole("button");
const status = (): string | null => screen.getByRole("status").textContent;

afterEach(() => {
  vi.useRealTimers();
});

test("starts stopped", () => {
  render(<App player={new FakePlayer()} />);
  expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Playground");
  expect(button().textContent).toBe("Start");
  expect(button().disabled).toBe(false);
  expect(status()).toBe("Stopped");
  expect(screen.queryByRole("alert")).toBeNull();
});

test("starts and stops the player", async () => {
  const player = new FakePlayer();
  render(<App player={player} />);
  fireEvent.click(button());
  expect(player.calls).toStrictEqual(["start"]);
  expect(status()).toBe("Starting…");
  expect(button().textContent).toBe("Start");
  expect(button().disabled).toBe(true);
  await settle(() => player.starts[0]?.resolve());
  expect(status()).toBe("Playing");
  expect(button().textContent).toBe("Stop");
  expect(button().disabled).toBe(false);

  fireEvent.click(button());
  expect(player.calls).toStrictEqual(["start", "stop"]);
  expect(status()).toBe("Stopping…");
  expect(button().textContent).toBe("Stop");
  expect(button().disabled).toBe(true);
  await settle(() => player.stops[0]?.resolve());
  expect(status()).toBe("Stopped");
  expect(button().textContent).toBe("Start");
});

test("ignores clicks while busy", () => {
  const player = new FakePlayer();
  render(<App player={player} />);
  fireEvent.click(button());
  fireEvent.click(button());
  expect(player.calls).toStrictEqual(["start"]);
});

test("shows why starting failed, until the next attempt", async () => {
  const player = new FakePlayer();
  render(<App player={player} />);
  fireEvent.click(button());
  await settle(() => player.starts[0]?.reject(new Error("no audio device")));
  expect(screen.getByRole("alert").textContent).toBe("Error: no audio device");
  expect(status()).toBe("Stopped");
  expect(button().textContent).toBe("Start");
  fireEvent.click(button());
  expect(screen.queryByRole("alert")).toBeNull();
});

test("shows why stopping failed", async () => {
  const player = new FakePlayer();
  render(<App player={player} />);
  fireEvent.click(button());
  await settle(() => player.starts[0]?.resolve());
  fireEvent.click(button());
  await settle(() => player.stops[0]?.reject(new Error("device lost")));
  expect(screen.getByRole("alert").textContent).toBe("Error: device lost");
  expect(status()).toBe("Stopped");
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
