# playground

The Rust engine in the browser: a 440 Hz sine with an on/off button and a live oscilloscope, and the visual library's square, sweeping on the audio clock (`spec/limitations.md`). Vite + React; it grows into the editor.

| Path | What |
|---|---|
| `src/player.ts` | Plays the patch with the engine (`@poyetik/audio-wasm`), keeps its latest samples and says the frame of the audio clock |
| `src/audio-clock.ts` | The one reader of the audio clock, and the frame of a time (`frameAt`) |
| `src/scope.ts` | The oscilloscope as data: scene and measurements, pure functions |
| `src/draw.ts` | Draws the scene on a Canvas 2D |
| `src/Square.tsx`, `src/square-module.ts` | The square: the page module (`@poyetik/web-page`) gives the image of the clock's frame, encoded for the canvas |
| `src/gpu-square.ts` | The square drawn on the GPU through WebGPU, with the library's shaders: the same bytes |
| `gates/` | The logic of the gates its tests run (the accessibility gate, the laws of the remedies and of the presentation), measured like the code |
| `src/Oscilloscope.tsx`, `src/App.tsx` | The interface |
| `src/main.tsx` | The entry point: mounts the playground, loads the engine module built for Web Audio's render quantum (`?muted` silences the speakers, for the end-to-end tests) |
| `src/patch.ts` | The patch the playground plays, a 440 Hz sine on both channels, as UTF-8 text in the patch format |
| `src/history.ts` | The most recent samples of a signal, in a ring of fixed size: nothing allocated per render quantum |
| `src/readout.ts` | Measurements shown as text, readable by people and by the end-to-end tests |
| `src/settings.ts` | The playground's settings: the gain before the speakers, the duration the oscilloscope keeps |
| `src/fetch.ts` | The one place that calls `fetch`, so that no response is read before its status is checked |
| `src/wasm-bytes.ts` | Tells downloaded WebAssembly bytes apart, for the square and the sound alike: no module (a download that failed) or a module this browser cannot compile |
| `src/failure.ts`, `src/cause.ts` | What failed, in words, and whose a failure is (network, environment, defect), which chooses its remedy |
| `e2e/` | End-to-end tests on Chromium, Firefox and Playwright's WebKit (desktop and iPhone sizes; not Safari), with axe |

```sh
pnpm dev     # http://localhost:5173 (first build the packages it needs, in the order of its `needs` in crates/xtask/src/workspace.rs: pnpm --dir ../../bindings/audio-wasm build, pnpm --dir ../../bindings/visual-wasm build, pnpm --dir ../../bindings/web-page build)
pnpm check   # types, lint, unit tests at 100% coverage, build, end-to-end tests
```

`?muted` silences the speakers; the oscilloscope still shows the engine output.
