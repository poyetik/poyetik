// Each thread loads the one module it needs (#34 step 6): the audio worklet the engine's
// module, which holds no drawing code, so that the split leaves its start-up as it was; the
// page its own module, which holds no engine.
import { readFileSync } from "node:fs";
import { expect, test } from "vitest";

/** The names a module exports, from its bytes. */
function exported(specifier: string): string[] {
  const bytes = readFileSync(new URL(import.meta.resolve(specifier)));
  return WebAssembly.Module.exports(new WebAssembly.Module(bytes)).map(({ name }) => name);
}

/** Whether the module's bytes hold `text` anywhere (a shader's source, say). */
function holds(specifier: string, text: string): boolean {
  return readFileSync(new URL(import.meta.resolve(specifier))).includes(text);
}

test("gives the audio worklet a module without the drawing code, and the page one without the engine", () => {
  const engine = exported("@poyetik/audio-wasm/engine-128.wasm");
  const page = exported("@poyetik/web-page/page.wasm");
  expect(engine.filter((name) => name.startsWith("visual_") || name.startsWith("page_")), "nothing of the visual library in the worklet's module").toStrictEqual([]);
  expect(engine.some((name) => name.startsWith("engine_"))).toBe(true);
  expect(page.filter((name) => name.startsWith("engine_")), "nothing of the engine in the page's").toStrictEqual([]);
  expect(page.some((name) => name.startsWith("visual_")) && page.some((name) => name.startsWith("page_"))).toBe(true);
  // The library's shaders travel with the page module only.
  expect([holds("@poyetik/audio-wasm/engine-128.wasm", "@compute"), holds("@poyetik/web-page/page.wasm", "@compute")]).toStrictEqual([false, true]);
});
