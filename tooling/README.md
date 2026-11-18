# tooling

Shared development tooling of the monorepo.

| Package | Contents |
|---|---|
| `config/` (`@poyetik/config`) | The strict TypeScript settings (`tsconfig.base.json`) and ESLint configuration (`eslint.ts`) every TypeScript package uses. Its tests prove the policy: each fixture in `test/policy/` breaks one lint rule and `test/compiler/` one compiler setting, and each must be rejected by exactly that rule (`pnpm check`: typecheck, lint, 100% coverage; mutation testing: no surviving mutant) |
| `oracle/` (`@poyetik/oracle`) | The independent conformance oracle: rereads the WAV file the native engine wrote for every case, recomputes its hash and checks the signal against its mathematical definition, with no code shared with the engine; it also reads the visual conformance cases, as `visual-cli` does, refusing the shared invalid corpus |
| `wgsl/` | The WGSL proof suite, Rust crates in a Cargo workspace of their own (`tooling/wgsl/README.md`): `wgsl-test`, which validates WGSL shaders with naga |
| `wasm-build/` (`@poyetik/wasm-build`) | Builds a WebAssembly module of the project reproducibly and proves it, for the engine's and the visual library's packages alike: each names its module and wires two entry points to it |

Each package is checked with `pnpm check` in its folder; `cargo xtask check` runs them all, each followed by its mutation testing (Stryker), which it runs itself; `cargo xtask mutants <package directory or source>` runs every mutant of one.
