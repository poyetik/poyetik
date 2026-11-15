import { expect, test } from "vitest";
import { ENGINE_MODULES } from "../scripts/module.ts";

test("names one engine module per engine block: its crate, what messages call it, its files in dist/ and the size its build names", () => {
  // The engine blocks a build offers (`audio_core::ENGINE_BLOCKS`, decision 2), written apart;
  // the default one, 64 frames (`audio_core::BLOCK_SIZE`), is also the package's engine.wasm.
  const named = (block: number, copies: object = {}): object => ({
    package: "audio-wasm",
    label: `engine module of ${String(block)}-frame engine blocks`,
    file: `engine-${String(block)}.wasm`,
    ...copies,
    env: { POYETIK_ENGINE_BLOCK: String(block) },
  });
  expect(ENGINE_MODULES).toStrictEqual([named(16), named(32), named(64, { copies: ["engine.wasm"] }), named(128), named(256), named(512), named(1024)]);
});
