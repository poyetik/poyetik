import { act } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { SINE_440 } from "../src/patch.ts";
import type { PlayerOptions } from "../src/player.ts";
import { FADE, GAIN, HISTORY_SECONDS } from "../src/settings.ts";

/** Options given to each player created by the entry point. */
const created: PlayerOptions[] = [];

vi.mock("../src/player.ts", () => ({
  EnginePlayer: class {
    constructor(options: PlayerOptions) {
      created.push(options);
    }

    /** Nothing played: the stub never plays. */
    latest(): undefined {
      return undefined;
    }

    /** Never observed: the stub never loads the engine. */
    arithmetic(): undefined {
      return undefined;
    }

    /** The only state of the stub, shared so that `state()` is stable. */
    readonly #state = { kind: "stopped" } as const;

    /** Always stopped: the entry point is tested, not the player. */
    state(): { readonly kind: "stopped" } {
      // The same object every time, like the real player: React compares snapshots.
      return this.#state;
    }

    /** Never notifies, since the state never changes; the unsubscribe does nothing. */
    subscribe(): () => void {
      return () => undefined;
    }
  },
}));

beforeEach(() => {
  created.length = 0;
  vi.resetModules();
  document.body.innerHTML = '<div id="root"></div>';
});

afterEach(() => {
  history.replaceState(null, "", "/");
});

/** Runs the entry point. */
async function main(): Promise<void> {
  await act(async () => {
    await import("../src/main.tsx");
  });
}

test("mounts the playground with an audible player", async () => {
  await main();
  expect(document.querySelector("h1")?.textContent).toBe("Playground");
  expect(created).toHaveLength(1);
  expect(created[0]).toMatchObject({ gain: GAIN, fade: FADE, historySeconds: HISTORY_SECONDS, environment: globalThis });
  expect(created[0]?.wasm).toMatch(/engine\.wasm/);
  expect(created[0]?.worklet).toMatch(/processors/);
  expect(created[0]?.patch).toStrictEqual(SINE_440);
});

test("mutes the speakers with ?muted", async () => {
  history.replaceState(null, "", "/?muted");
  await main();
  expect(created[0]?.gain).toBe(0);
});

test("needs a #root element", async () => {
  document.body.innerHTML = "";
  await expect(import("../src/main.tsx")).rejects.toThrow("no #root element");
});
