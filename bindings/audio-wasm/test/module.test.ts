import { expect, test } from "vitest";
import { ENGINE_MODULES } from "../scripts/module.ts";
import { ENGINE_BLOCKS } from "../ts/web.ts";

test("names one engine module per engine block the package exports, smallest first: its crate, what messages call it, its files in dist/ and the size its build names", () => {
  // `ENGINE_BLOCKS` is tied to `audio_core::ENGINE_BLOCKS` by audio-host's tests; the
  // default one, 64 frames (`audio_core::BLOCK_SIZE`), is also the package's engine.wasm.
  const named = (block: number): object => ({
    package: "audio-wasm",
    label: `engine module of ${String(block)}-frame engine blocks`,
    file: `engine-${String(block)}.wasm`,
    ...(block === 64 ? { copies: ["engine.wasm"] } : {}),
    env: { POYETIK_ENGINE_BLOCK: String(block) },
  });
  expect(ENGINE_MODULES).toStrictEqual(ENGINE_BLOCKS.map(named));
  expect([...ENGINE_BLOCKS].sort((left, right) => left - right)).toStrictEqual(ENGINE_BLOCKS);
  expect(ENGINE_BLOCKS).toContain(64);
});
