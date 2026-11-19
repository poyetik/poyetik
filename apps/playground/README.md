# playground

The Rust engine in the browser: a 440 Hz sine with an on/off button and a live oscilloscope, and the visual library's square moving in step with the sound. Vite + React; it grows into the editor.

| Path | What |
|---|---|
| `src/player.ts` | Plays the patch with the engine (`@poyetik/audio-wasm`), keeps its latest samples and says the frame of the audio clock |
| `src/audio-clock.ts` | The one reader of the audio clock, and the frame of a time (`frameAt`) |
| `src/scope.ts` | The oscilloscope as data: scene and measurements, pure functions |
| `src/draw.ts` | Draws the scene on a Canvas 2D |
| `src/Square.tsx`, `src/square-module.ts` | The square: the page's module (`@poyetik/web-page`) gives the image of the clock's frame, encoded for the canvas |
| `src/gpu-square.ts` | The square drawn on the GPU through WebGPU, with the library's shaders: the same bytes |
| `src/Oscilloscope.tsx`, `src/App.tsx` | The interface |
| `e2e/` | End-to-end tests on Chromium, Firefox and Playwright's WebKit (desktop and iPhone sizes; not Safari), with axe |

```sh
pnpm dev     # http://localhost:5173 (first build the packages it needs, in the order of its `needs` in crates/xtask/src/workspace.rs: pnpm --dir ../../bindings/audio-wasm build, pnpm --dir ../../bindings/visual-wasm build, pnpm --dir ../../bindings/web-page build)
pnpm check   # types, lint, unit tests at 100% coverage, build, end-to-end tests
```

`?muted` silences the speakers; the oscilloscope still shows the engine output.
