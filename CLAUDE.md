# poyetik (working name)

## Language: English only

**Everything in this repository is written in English**: code, identifiers, comments, CLI output, UI text, tests, documentation, roadmap tickets, commit messages, PR descriptions, CI and hook output. No exceptions.


## Guiding principle: maximum proof

We want **the maximum proof we can get**, everywhere, at all times. Every property we claim must be backed by an automated check that runs in CI and in the pre-push hook. In order of preference:

1. **Impossible by construction**: the type system, the compiler or the tooling makes the mistake unrepresentable or uncompilable (`no_std`, `forbid(unsafe_code)`, newtypes, unions of literals, banned methods, no casts).
2. **Formally checked**: model checking or proofs where it is feasible (Kani, Miri).
3. **Exhaustively tested**: 100% line and branch coverage, no surviving mutant, property tests, fuzzing, cross-target conformance.
4. **Reviewed**: only for what none of the above can reach, and documented as such.

When in doubt, choose the stronger guarantee, even if it costs more work. A shortcut that weakens a guarantee is never acceptable.

## Goal

A visual audio patching environment (boxes connected by cables) running in the browser, with a Rust engine (WebAssembly on the web, native elsewhere).

A finished patch can be exported natively, user interface included, to: web, macOS, Windows, Linux, iOS, Android, and plugins (VST3, CLAP, AU…).

Core promise: **the engine produces exactly the same samples, bit for bit, on every target**. A strict, public testing policy proves it.

## Documentation

- `doc/` holds the history of design discussions and architecture decisions.
- `doc/licensing.md` is the licensing strategy. **No license is chosen yet**: never add a `LICENSE` file or change the `UNLICENSED` identifiers without the maintainer's explicit decision.
- `doc/2026-11-07-initial-brainstorm.md` is the founding brainstorm. Read it before any architecture decision when it is present. **Brainstorm notes stay local and are never versioned** (`doc/*brainstorm*.md` is git-ignored), so they may be missing in a fresh clone.

Project rules will be added here as we go.

## Repository structure

A **monorepo of separate parts** with clear contracts (details in `doc/architecture.md`, section 2):
`spec/` (patch format), `crates/` (reusable engine core and tooling), `bindings/` (developer SDKs), `apps/` (applications), `conformance/` (reference cases), `tooling/` (shared development configuration), `doc/`.
One Cargo workspace and one pnpm workspace at the root. A part only uses another through its documented public interface. Parts are built **incrementally**: empty folders only hold a short README; never specify ahead of need.

## Roadmap (`apps/roadmap/`)

Public project kanban. The data lives in `apps/roadmap/board.json`, versioned in git. A static Astro page with no JavaScript displays it.

- Roadmap changes are made by running the CLI (Claude runs it on the maintainer's request).
- **Never edit `board.json` by hand**: always use `pnpm roadmap <command>` from `apps/roadmap/` (`list`, `show`, `add`, `move`, `edit`, `rm`, `validate`). Run `pnpm roadmap help` for usage.
- Columns: `later` (Later), `next` (Next), `doing` (In progress), `done` (Done). Milestones are written like "M1 Engine", "M2 Editor"…
- Cards have ordered **steps** (`task add|done|undo|edit|rm`). A step is either done or not; **the first step that is not done is automatically "In progress"**. When a step is finished, run `task done <id>` to tick it, which moves "In progress" to the next step.
- Page: `pnpm dev` (http://localhost:4321). Full check chain: `pnpm check`.

## CI

**One command runs every check: `cargo xtask check`** (Rust formatting, lints, tests, portability builds, 100% coverage gate, mutation testing, Miri, conformance, supply chain, then every TypeScript package: the engine in the browser, the playground and the roadmap). The `.githooks/pre-push` hook and `.github/workflows/quality.yml` (every push and pull request) both run exactly this command, so a green local check means a green CI.

Every new check is added to `xtask` (`check_steps`), never directly to the hook or the workflow. The hook is enabled through `core.hooksPath`, set automatically by `pnpm install` at the root. **Never bypass the hook** (`--no-verify` is forbidden).

## Portability

**The engine core is fully portable: no platform-specific code, ever.** `engine-core` (and every crate it will be split into) is `no_std`, has no OS, clock, thread, file, network or JS access, and builds for every target. Everything platform-specific (audio I/O, files, permissions, UI hosting) belongs to the thin shells around it (`bindings/`, native players, plugin wrappers, Tauri apps).

This is proven, not assumed: `cargo xtask check` builds the core and the boxes (`engine-objects`) for macOS (ARM), Linux (x86), WebAssembly and a bare-metal ARM microcontroller with no OS at all.

## Technologies and dependencies

- **As few as possible.** Every dependency (library or tool) is a liability: prefer the standard library, then a tool or library already used in the project, then a new one.
- **Modern, mainstream and well maintained.** Only widely used projects with active maintenance and a healthy community. Nothing obscure, experimental or abandoned.
- **Every dependency is justified** in the dependency register of `doc/architecture.md` (what it is for, why it is needed, why this one). Adding one means adding its entry first.
- Versions are locked (`Cargo.lock`, `pnpm-lock.yaml`) and the toolchains are pinned.

## Strict typing and linting policy

Non-negotiable, and **enforced by tooling, not by goodwill**: CI and the pre-push hook reject any violation. No shortcut may weaken the whole.

### Principle: parse at the boundaries, trust types inside
- External data (JSON, CLI arguments, files, network, FFI) enters as `unknown` (TS) or raw bytes (Rust) and is **validated once, at the boundary**, by functions that *prove* the type (TS type guards, Rust constructors returning `Result`).
- Internal code only accepts proven types and **never re-checks** what the type system already guarantees. Invalid states must be unrepresentable (unions of literals, newtypes, private fields with checked constructors).
- If a value cannot be typed precisely, fix the types. Never silence the checker.

### TypeScript
- `tsconfig`: the strictest settings (`strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `noPropertyAccessFromIndexSignature`, `verbatimModuleSyntax`, `isolatedModules`, `erasableSyntaxOnly`, `useUnknownInCatchVariables`…), shared by every package through `@poyetik/config` (`tooling/config`); the roadmap extends Astro `strictest` with the same extras.
- ESLint: one configuration for every package, `@poyetik/config/eslint`; a package only adds what its framework needs (React hooks rules).
- ESLint: `strictTypeChecked` + `stylisticTypeChecked` with zero warnings, plus:
  - **no type assertions** (`as T`, `<T>x`), except `as const`, and no non-null assertions (`!`);
  - **no `any`**, explicit or implicit;
  - **no `@ts-ignore`, `@ts-expect-error`, `@ts-nocheck`**;
  - **no inline `eslint-disable`** comments: a rule is either respected or changed in the config, for everyone, with a reason;
  - strict boolean conditions (no truthiness on strings, numbers or nullable values), exhaustive `switch`, explicit types on exported functions, no `enum`, `readonly` where possible, type-only imports.
- Tests follow exactly the same rules: no casts to reach "impossible" states. If a state is impossible by type, it needs no test and no runtime check.

### Rust
- Pinned toolchain (`rust-toolchain.toml`), edition 2024, `rustfmt --check`.
- Workspace-level lints, inherited by every crate, all as errors:
  - `unsafe_code = "forbid"` everywhere except two minimal, audited modules: the FFI module of `engine-wasm`, and the counting allocator of `engine-testkit` (test-only, every call delegated unchanged to the system allocator), which proves that boxes never allocate while processing;
  - `missing_docs`, `unused_results`, `unreachable_pub`, `rust_2018_idioms`;
  - Clippy `all`, `pedantic`, `nursery` and `cargo` groups, plus selected `restriction` lints: no `unwrap`/`expect`/`panic!`/`todo!`/`unimplemented!`, no `as` numeric casts (use `From`/`TryFrom`), no indexing that can panic, no float equality, no `dbg!`/`print!` in library code, no lint suppressions (`#[allow]`) without an explicit, reviewed `reason`.
- Determinism rules as Clippy `disallowed_methods` / `disallowed_types` (`f32::sin`, `mul_add`, `Instant::now`, `HashMap` with random state…).
- `cargo-deny`: allowed licenses only (MIT/Apache/BSD), no known vulnerabilities, no duplicate or unexpected dependencies.
- Coverage 100% lines, functions and branches (`cargo-llvm-cov`; branch coverage needs a pinned nightly, `nightly-2026-11-01`, used for that measurement only), **no surviving mutant** (`cargo-mutants --cap-lints=true`, so that mutants are never skipped because of lints), Miri on any `unsafe`.
- **Every unviable mutant (one that does not compile) must be reviewed.** Confirm that the type system or `no_std` really makes that wrong implementation impossible, and that no legitimate variant of it escapes the tests; then record it with its reason in `mutants-unviable.txt`. `cargo xtask check` fails on any unreviewed unviable mutant and on any stale entry, so the list always matches reality.

## Documentation, versioning and commits

Enforced by `cargo xtask check` and the git hooks, like everything else.

### Documentation
- **Every item is documented**, public or private: Rust `missing_docs` + Clippy `missing_docs_in_private_items`; TypeScript JSDoc on every exported declaration (`eslint-plugin-jsdoc`).
- Docs explain **why and what**, not only how: purpose, invariants, units, errors (`# Errors` sections in Rust), panics (there must be none).
- `cargo doc` runs with the rustdoc lints as errors: no broken or private links, no invalid code blocks, crate-level docs required.
- `README.md` explains what the project is and how to run and check it; `doc/architecture.md` holds the architecture, the dependency register and the decisions log. Every architecture decision adds a line to that log.

### Versioning and changelog
- **One version for the whole project**, following [Semantic Versioning](https://semver.org): the workspace `version` in `Cargo.toml` and the `version` of every `package.json` must match (checked).
- **Every notable change adds an entry under `## [Unreleased]` in `CHANGELOG.md`**, in the same commit ([Keep a Changelog](https://keepachangelog.com) sections: Added, Changed, Deprecated, Removed, Fixed, Security). The changelog format is checked: `# Changelog`, `## [Unreleased]` first, then `## [X.Y.Z] - YYYY-MM-DD` in strictly decreasing order, never newer than the project version.
- While the version is `0.x`, a breaking change bumps the minor version; from `1.0.0` on, it bumps the major version.
- Releasing: move the `[Unreleased]` entries under a new `## [X.Y.Z] - date` heading, set the version everywhere, commit, then tag `vX.Y.Z`.

### Commits
- **[Conventional Commits](https://www.conventionalcommits.org)**, in English: `type(scope)!: summary`, with type one of `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `build`, `ci`, `chore`, `style`, `revert`; optional lowercase scope (`engine-core`, `xtask`, `roadmap`…); `!` for breaking changes; header of at most 72 characters, no trailing period, a blank line before the body.
- Checked by the `.githooks/commit-msg` hook (`cargo xtask commit-msg`). One logical change per commit; never commit a red `cargo xtask check`.

## Testing policy

Non-negotiable. A change that does not meet these rules is not done.

### Principles
- **No code without tests.** For the engine, the test is written first and must fail before the implementation.
- **Every bug fix starts with a regression test** that reproduces the bug and fails.
- **Tests are deterministic**: no system clock, no `sleep`, no network, no unseeded randomness, no dependency on execution order. A flaky test is a bug to fix, never something to re-run until it passes.
- **Never work around a test**:
  - do not delete it, disable it (`#[ignore]`, `.skip`) or weaken it;
  - do not lower a threshold or widen a tolerance;
  - do not update a reference hash to make CI pass.
  If a test looks wrong, stop and discuss it.
- **CI is the source of truth.** A single local command reproduces CI exactly.

### Scope: the whole project
Maximum testing applies to **all code, in every language**: Rust engine, patch model, WASM/worklet glue, web editor (TS), native shells (Swift, Kotlin), plugin wrappers, build service, AI assistant, tooling scripts. Nothing is "too simple to test".

### Measured requirements (all code)
- **Coverage: 100% of lines and branches** (`cargo-llvm-cov` for Rust, the equivalent tool for each other language). CI fails below that.
- **Mutation testing: no surviving mutant** (`cargo-mutants` for Rust, Stryker for TS, the equivalent tool elsewhere). A truly equivalent mutant is listed in a dedicated file with its justification. That list is public and reviewed.
- **Strict static analysis**: `clippy -D warnings`, `rustfmt --check`, TypeScript `strict` with no `any`, ESLint with zero warnings, no exceptions.
- **Supply chain**: `cargo-deny` and npm audit (licenses, vulnerabilities, provenance), locked dependencies.

### Behavior tests
- **Reference renders**: every object has test patches rendered offline (patch + events + seed → samples + hash).
- **Cross-target conformance**: the same corpus produces **the same hashes natively and in WASM**, in CI.
- **A reference hash only changes through a dedicated command**, and the commit explains why. Never silently.
- **Property tests** (`proptest`), at minimum:
  - the same render regardless of how blocks are split;
  - two identical renders give the same result;
  - never NaN or infinity in the output.
- **Real-time safety**: no allocation in `process`, checked automatically (`assert_no_alloc`).
- **Fuzzing**: patch parsing, the command layer, then native-vs-WASM differential fuzzing.
- **`unsafe` code**: forbidden by default. If truly required, it is isolated, checked with Miri and, for critical parts, proven with Kani.

### Every other aspect
- **Commands and patch model**: every command is tested (success, rejection, undo/redo round trip), with property tests over random command sequences (full undo = initial state, identical serialization round trip).
- **Format compatibility**: every version of the patch format keeps test files. Old patches always load and migrate, checked in CI.
- **Web interfaces (playground, editor)**: `doc/interface-testing.md` is the required test strategy, layer by layer (scene, camera and animations, interactions, renderer, accessibility layer, components, end-to-end, visual regression, performance). It is mandatory, heavy on purpose, and applies from the first line of every interface change. In short:
  - unit and component tests;
  - end-to-end tests (Playwright) on **Chrome, Firefox and WebKit**;
  - visual regression via screenshots;
  - accessibility (axe);
  - the sound produced in the browser is captured (`OfflineAudioContext`) and compared to the native hash.
- **Performance**: engine and editor benchmarks (CPU per object, latency, load time, memory) tracked in CI. A regression beyond the set threshold fails CI.
- **Exported artifacts**: each target runs in test mode, its render is compared to the reference hash, and plugins pass the official validators (`pluginval`, `clap-validator`, `auval`, the VST3 validator).
- **Native shells**: Swift/Kotlin unit tests, plus lifecycle tests (interruptions, background, device changes, denied permissions) on simulators and reference devices.
- **Security**: CSP verified, no API key ever sent to the server (checked by tests), fuzzing of everything that reads external input (patches, media, network messages, MCP).
- **AI assistant**: an evaluation suite (requests → valid patch + expected render metrics), with a success rate tracked per model. A drop fails CI.
- **Reproducible builds**: two builds of the same commit produce identical artifacts, checked in CI.

### Transparency
Results are meant to be published for every release: coverage, mutants, per-target conformance hashes, known deviations.
