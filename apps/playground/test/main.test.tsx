import { act } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { PlayerOptions } from "../src/player.ts";
import { GAIN, HISTORY } from "../src/settings.ts";

/** Options given to each player created by the entry point. */
const created: PlayerOptions[] = [];

vi.mock("../src/player.ts", () => ({
  EnginePlayer: class {
    constructor(options: PlayerOptions) {
      created.push(options);
    }

    latest(): undefined {
      return undefined;
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
  expect(created[0]).toMatchObject({ gain: GAIN, history: HISTORY, onError: reportError });
  expect(created[0]?.wasm).toMatch(/engine\.wasm/);
  expect(created[0]?.worklet).toMatch(/processors/);
  expect(new TextDecoder().decode(created[0]?.patch)).toContain('"osc~"');
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
