# wasm-build

Builds a WebAssembly module of the project and proves the build reproducible, for every package that ships one: the audio engine's (`bindings/audio-wasm`, `audio-wasm`) and the visual library's (`bindings/visual-wasm`, `visual-wasm`). Each package names its modules (`WasmModule`: its cargo package, what messages call it, the file it is copied to, other files it is copied to, the variables its build is given; the engine has one per engine block) and keeps two thin entry points calling these functions; the logic is written and tested once, here.

| Path | What |
|---|---|
| `src/wasm.ts` | The module descriptor, the cargo arguments, the rustc flags that remap every path of the building machine, reading cargo's messages and rustc's answers, and what makes two builds not reproducible |
| `src/cli.ts` | The two commands, everything they touch injected (`Io`): `buildMain` (build each module, copy it to its files in `dist/`) and `reproducibleMain` (build each twice in fresh target directories, compare the bytes, look for paths of the machine) |
| `src/io.ts` | The real outside world of the commands: this process, its files and cargo |

```sh
pnpm check   # types, lint, unit tests at 100% coverage
```
