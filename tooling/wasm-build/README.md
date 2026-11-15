# wasm-build

Builds a WebAssembly module of the project, proves the build reproducible and counts what the module executes, for every package that ships one: the audio engine's (`bindings/audio-wasm`, `audio-wasm`) and the visual library's (`bindings/visual-wasm`, `visual-wasm`). Each package names its modules (`WasmModule`: its cargo package, what messages call it, the file it is copied to, other files it is copied to, the variables its build is given; the engine has one per engine block) and keeps two thin entry points calling these functions; the logic is written and tested once, here.

| Path | What |
|---|---|
| `src/wasm.ts` | The module descriptor, the cargo arguments, the rustc flags that remap every path of the building machine, reading cargo's messages and rustc's answers, and what makes two builds not reproducible |
| `src/instrument.ts` | The instrumenter of the cost benchmarks (`spec/benchmarks.md`): rewrites a module so that it counts the WebAssembly instructions it executes, deterministically; used by every library's benchmarks |
| `test/assemble.ts` | Hand-assembled modules for the instrumenter's tests, exported (`@poyetik/wasm-build/testing/assemble`) to the benchmarks' tests of each library |
| `src/cli.ts` | The two commands, everything they touch injected (`Io`): `buildMain` (build each module in a target directory of its own, `wasm-build/<file>` of the one cargo would use, so that a module's variables never rebuild another's crates, and copy it to its files in `dist/`) and `reproducibleMain` (build every module in a fresh target directory, then again in a second one in the reverse order, the two directories shared by the modules; compare each module's two builds and look for paths of the machine; report every module, then one `error:` line counting those that are not reproducible) |
| `src/io.ts` | The real outside world of the commands: this process, its files and cargo |

```sh
pnpm check   # types, lint, unit tests at 100% coverage
```
