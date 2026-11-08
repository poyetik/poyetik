# playground

The Rust engine in the browser: a 440 Hz sine with an on/off button and a live oscilloscope. Vite + React; it grows into the editor.

| Path | What |
|---|---|
| `src/player.ts` | Plays the patch with the engine (`@poyetik/engine-wasm`) and keeps its latest samples |
| `src/scope.ts` | The oscilloscope as data: scene and measurements, pure functions |
| `src/draw.ts` | Draws the scene on a Canvas 2D |
| `src/Oscilloscope.tsx`, `src/App.tsx` | The interface |
| `e2e/` | End-to-end tests on Chromium, Firefox, WebKit and mobile WebKit, with axe |

```sh
pnpm dev     # http://localhost:5173 (build bindings/wasm first: pnpm --dir ../../bindings/wasm build)
pnpm check   # types, lint, unit tests at 100% coverage, build, end-to-end tests
```

`?muted` silences the speakers; the oscilloscope still shows the engine output.
