# poyetik (working name)

## Language: English only

**Everything in this repository is written in English**: code, identifiers, comments, CLI output, UI text, tests, documentation, roadmap tickets, commit messages, PR descriptions, CI and hook output. No exceptions.

## Goal

A visual audio patching environment (boxes connected by cables) running in the browser, with a Rust engine (WebAssembly on the web, native elsewhere).

A finished patch can be exported natively, user interface included, to: web, macOS, Windows, Linux, iOS, Android, and plugins (VST3, CLAP, AU…).

Core promise: **the engine produces exactly the same samples, bit for bit, on every target**. A strict, public testing policy proves it.

## Documentation

- `doc/` holds the history of design discussions and architecture decisions.
- `doc/2026-11-07-initial-brainstorm.md` is the founding brainstorm. Read it before any architecture decision when it is present. **Brainstorm notes stay local and are never versioned** (`doc/*brainstorm*.md` is git-ignored), so they may be missing in a fresh clone.

Project rules will be added here as we go.

## Roadmap (`roadmap/`)

Public project kanban. The data lives in `roadmap/board.json`, versioned in git. A static Astro page with no JavaScript displays it.

- Roadmap changes are made by running the CLI (Claude runs it on the maintainer's request).
- **Never edit `board.json` by hand**: always use `pnpm roadmap <command>` from `roadmap/` (`list`, `show`, `add`, `move`, `edit`, `rm`, `validate`). Run `pnpm roadmap help` for usage.
- Columns: `later` (Later), `next` (Next), `doing` (In progress), `done` (Done). Milestones are written like "M1 Engine", "M2 Editor"…
- Page: `pnpm dev` (http://localhost:4321). Full check chain: `pnpm check`.

## CI

`.github/workflows/quality.yml` runs on every push and every pull request. Each new package in the project adds its own jobs there.

The `.githooks/pre-push` hook runs the same checks locally before every push. It is enabled through `core.hooksPath`, set automatically by `pnpm install`. Each new package adds its own check command to it. **Never bypass the hook** (`--no-verify` is forbidden).

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
- **Web editor**:
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
