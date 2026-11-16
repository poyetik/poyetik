/**
 * The engine blocks the package ships a module for, in frames (`spec/engine-design.md`,
 * decision 2): `audio_core::ENGINE_BLOCKS`, one module each, `dist/engine-<frames>.wasm`
 * (`@poyetik/audio-wasm/engine-<frames>.wasm`). The one TypeScript list of them, read by
 * the build (`scripts/module.ts`) and the tests; `audio-host`'s tests read this line and
 * compare it with `audio_core::ENGINE_BLOCKS`, so keep it on one line.
 */
export const ENGINE_BLOCKS = [16, 32, 64, 128, 256, 512, 1024] as const;

/** An engine block the package ships a module for, in frames: one of {@link ENGINE_BLOCKS}. */
export type EngineBlock = (typeof ENGINE_BLOCKS)[number];
