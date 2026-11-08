# Poyetik

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

## Audio DSP: the objective

**State-of-the-art audio quality, measured, on every target.** Every box sounds as good as the best professional implementations of the same process, and proves it with numbers in its tests; bit-exact determinism is never traded for it, nor the reverse.

- **Quality is measured, not claimed**: every box has objective measurements with thresholds (spectrum, aliasing level, THD, SNR, frequency response, behavior under parameter changes), compared with high-precision references (double precision or arbitrary precision). Thresholds are documented per box with their justification.
- **Known pitfalls are handled by design**: aliasing (band-limited synthesis, oversampled non-linearities), zipper noise (smoothed parameters, sample-accurate events), unstable filters under modulation (topologies such as TPT / zero-delay feedback), denormal numbers (a deterministic strategy, since WebAssembly has no hardware flush-to-zero), DC offset, numerical precision of phases and coefficients.
- **Real time is a hard constraint**: no allocation, lock, I/O or unbounded work in `process`, nor anywhere on the browser's audio thread. One known exception, tracked by roadmap #12 (step 1): a processor and its WebAssembly instance belong to the audio thread, so constructing the engine processor compiles the module, instantiates it and builds the plan there, once, before it renders.
- **Cost is measured**: benchmarks per box in CI with regression thresholds; an approximation replaces an exact function only when measurements prove it is both faster and within threshold.
- **Engine design decisions come first**: messages, event scheduling, feedback, polyphony, multichannel, rates, parallelism, hot swap, latency and state are decided and recorded before the code that depends on them.

## Documentation

- **`doc/` is local only and never versioned** (git-ignored), and no versioned file but this one refers to it (other files, the `.claude/` skills included, say "the local design notes listed in `CLAUDE.md`"): design discussions, `architecture.md` (architecture, dependency register, decisions log), `licensing.md` (licensing strategy), `interface-testing.md`, `debug-mode.md`, `integrations.md`, brainstorms. It may be missing in a fresh clone; when present, read the relevant files before any design decision. Never copy its strategy content into a versioned file.
- **No license is chosen yet**: never add a `LICENSE` file or change the `UNLICENSED` identifiers without the maintainer's explicit decision.
- `doc/2026-11-07-initial-brainstorm.md` is the founding brainstorm. Read it before any architecture decision when it is present.

Project rules will be added here as we go.

## Repository structure

A **monorepo of separate parts** with clear contracts (details in the local `doc/architecture.md`, section 2):
`spec/` (patch format, render hash), `crates/` (reusable engine core and tooling), `bindings/` (developer SDKs), `apps/` (applications), `conformance/` (reference cases), `tooling/` (shared development configuration).
One Cargo workspace and one pnpm workspace at the root. A part only uses another through its documented public interface. Parts are built **incrementally**: empty folders only hold a short README; never specify ahead of need.

## Roadmap (`apps/roadmap/`)

Public project kanban. The data lives in `apps/roadmap/board.json`, versioned in git. A static Astro page with no JavaScript displays it.

- Roadmap changes are made by running the CLI (Claude runs it on the maintainer's request).
- **Never edit `board.json` by hand**: always use `pnpm roadmap <command>` from `apps/roadmap/` (`list`, `show`, `add`, `move`, `edit`, `rm`, `title`, `step …`, `validate`). Run `pnpm roadmap help` for usage.
- **The roadmap is public: never reference `doc/`** (local, unversioned notes) in a ticket title, description or step. A ticket says what is to be done, in its own words.
- Columns: `later` (Later), `next` (Next), `doing` (In progress), `done` (Done). Milestones are written like "M1 Engine", "M2 Editor"…
- Cards have ordered **steps** (`step add|done|undo|edit|rm`). A step is either done or not; **in a card of the `doing` column, the first step that is not done is automatically "In progress"**. A card in any other column has no step in progress, so nothing shows as in progress until the card is moved to `doing`. When a step is finished, run `step done <id>` to tick it, which moves "In progress" to the next step (outside `doing`, give the step number: `step done <id> <n>`).
- Page: `pnpm dev` (http://localhost:4321). Full check chain: `pnpm check`.

## CI

**One command runs every check: `cargo xtask check`** (Rust formatting, lints, tests, portability builds, 100% coverage gate, mutation testing, Miri, conformance, supply chain, then every TypeScript package: the shared policy itself, each rule proven to reject what it forbids, the oracle, the engine in the browser, the playground and the roadmap). `.github/workflows/quality.yml` runs this command; the `.githooks/pre-push` hook runs `cargo xtask pre-push`, which refuses uncommitted changes and a pushed commit other than the one checked out, then runs the same check. So a green local check means a green CI. **CI runs on demand**, since the full check is long and commits are frequent: on a push only when the last commit's message contains `[ci]` (Claude adds it when the maintainer asks for CI), on every pull request, and by hand (Run workflow on GitHub).

Working while the check runs: when it is useful and harms nothing, development may continue in a separate git worktree (`git worktree add`, its own branch) while `cargo xtask check` validates a finished commit in the main one, since the full check (mutation testing above all) takes a long time. It is an option, not a habit: skip it when the next work depends on the result, or when the extra load would disturb the check (load-sensitive browser tests). The validated commit is not done, and the work built on it is not pushed, until that check is green; a failure is fixed by a new commit, validated again.

Every new check is added to `xtask` (`check_steps`), never directly to the hook or the workflow. The hook is enabled through `core.hooksPath`, set automatically by `pnpm install` at the root. **The hooks are never bypassed by default.** One exception, decided by the maintainer: when the maintainer explicitly asks for it for a given push, Claude may skip the pre-push check (`git push --no-verify`) and says so; the commits pushed that way are not validated until a later `cargo xtask check`, local or in CI, is green. The commit-msg hook is never bypassed (`git commit --no-verify` stays forbidden).

## Command-line tools

Every command-line tool of the repository (`engine-cli`, `cargo xtask`, the oracle, the roadmap CLI, and every one to come) follows one contract, tested on the real entry point:
- exit code 0 on success, 1 when the work failed, 2 for a usage error;
- every error is one line on stderr, `error: ...`, naming the file or the command involved; never a stack trace;
- a usage error says what is wrong and where the usage is, `error: <reason> (see: <tool> help)`; only `<tool> help` prints the usage, on stdout;
- a closed stdout (`| head`) is never an error and never changes the exit code;
- a tool with several problems to report prints them on stdout, one per line, and one `error:` summary on stderr;
- untrusted text quoted in an error (a box name, a field of a document) is escaped and bounded, so that it can neither drive the terminal nor forge a second line.

## Portability

**The engine core is fully portable: no platform-specific code, ever.** `engine-core` (and every crate it will be split into) is `no_std`, has no OS, clock, thread, file, network or JS access, and builds for every target. Everything platform-specific (audio I/O, files, permissions, UI hosting) belongs to the thin shells around it (`bindings/`, native players, plugin wrappers, Tauri apps).

This is proven, not assumed: `cargo xtask check` builds the core and the boxes (`engine-objects`) for macOS (ARM), Linux (x86), WebAssembly and a bare-metal ARM microcontroller with no OS at all.

## Technologies and dependencies

- **As few as possible.** Every dependency (library or tool) is a liability: prefer the standard library, then a tool or library already used in the project, then a new one.
- **Modern, mainstream and well maintained.** Only widely used projects with active maintenance and a healthy community. Nothing obscure, experimental or abandoned.
- **Every dependency is justified** in the dependency register of the local `doc/architecture.md` (what it is for, why it is needed, why this one). Adding one means adding its entry first.
- Versions are locked (`Cargo.lock`, `pnpm-lock.yaml`) and the toolchains are pinned.

## Strict typing and linting policy

Non-negotiable, and **enforced by tooling, not by goodwill**: CI and the pre-push hook reject any violation. No shortcut may weaken the whole.

### Principle: parse at the boundaries, trust types inside
- External data (JSON, CLI arguments, files, network, FFI) enters as `unknown` (TS) or raw bytes (Rust) and is **validated once, at the boundary**, by functions that *prove* the type (TS type guards, Rust constructors returning `Result`).
- Every limit a specification states (a size, a range, a count) is enforced at the earliest point, before the input is held whole, by a type of fixed width where one fits (never a pointer-wide type, whose range differs between targets), and is tested at the limit and one past it. Every reader of one format accepts and rejects the same documents, checked on a shared corpus.
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
  - no `unsafe` code anywhere (`#![forbid(unsafe_code)]`) except two minimal, audited files, checked by `cargo xtask check` to be the only ones lifting the lint, and by Miri: the FFI module of `engine-wasm`, and the counting allocator `engine-alloc-counter` (test-only, every call delegated unchanged to the system allocator), which proves that boxes never allocate while processing;
  - `missing_docs`, `unused_results`, `unreachable_pub`, `rust_2018_idioms`;
  - Clippy `all`, `pedantic`, `nursery` and `cargo` groups, plus selected `restriction` lints: no `unwrap`/`expect`/`panic!`/`todo!`/`unimplemented!`, no `as` numeric casts (use `From`/`TryFrom`), no indexing that can panic, no float equality, no `dbg!`/`print!` in library code, no lint suppressions (`#[allow]`) without an explicit, reviewed `reason`.
- Determinism rules as Clippy `disallowed_methods` / `disallowed_types` (`f32::sin`, `mul_add`, `Instant::now`, `HashMap` with random state…).
- `cargo-deny`: allowed licenses only (MIT/Apache/BSD), no known vulnerabilities, no duplicate or unexpected dependencies.
- Coverage 100% lines, functions and branches (`cargo-llvm-cov`; branch coverage needs a pinned nightly, `nightly-2026-11-01`, used for that measurement only), reached by **each crate's own unit tests**: `cargo xtask check` measures each crate of `RUST_CRATES` alone (`--lib`), so a path exercised only from another crate or an integration test is not covered (a fixture crate proves the gate refuses it), and each instantiation of a generic function counts separately. The one exception is a binary's `main.rs`, which unit tests cannot reach: it is measured by its crate's tests. Code no test can reach is removed by construction, not tested around.
- **No surviving mutant** (`cargo-mutants --cap-lints=true`, so that mutants are never skipped because of lints), and Miri on any `unsafe`.
- **Every unviable mutant (one that does not compile) must be reviewed.** Confirm that the type system or `no_std` really makes that wrong implementation impossible, and that no legitimate variant of it escapes the tests; then record it with its reason in `mutants-unviable.txt`. `cargo xtask check` fails on any unreviewed unviable mutant and on any stale entry, so the list always matches reality.

## Documentation, versioning and commits

Enforced by `cargo xtask check` and the git hooks, like everything else.

### Documentation
- **Every item is documented**, public or private: Rust `missing_docs` + Clippy `missing_docs_in_private_items`; TypeScript JSDoc on every named item, exported or not: functions, classes and their members, types, module-level constants (`eslint-plugin-jsdoc`, `publicOnly: false`).
- Docs explain **why and what**, not only how: purpose, invariants, units, errors (`# Errors` sections in Rust), panics (there must be none).
- `cargo doc` runs with the rustdoc lints as errors: no broken or private links, no invalid code blocks, crate-level docs required.
- `README.md` and each part's `README.md` explain what it is and how to run and check it (checked: every TypeScript package has a README, every part is named in the README of its folder, every folder in the root README); `GLOSSARY.md` holds the words of the project, one word per concept; the local `doc/architecture.md` holds the architecture, the dependency register and the decisions log. Every architecture decision adds a line to that log.

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
- **Mutation testing: no surviving mutant** (`cargo-mutants` for Rust, Stryker for TS, the equivalent tool elsewhere). A truly equivalent mutant is disabled by a `// Stryker disable next-line <Mutator>: <reason>` comment and listed with its justification in `mutants-equivalent.txt`; `cargo xtask check` fails on any disable comment not listed, any stale entry, and any broader disable. That list is public and reviewed.
- **Strict static analysis**: `clippy -D warnings`, `rustfmt --check`, TypeScript `strict` with no `any`, ESLint with zero warnings, no exceptions.
- **Supply chain**: `cargo-deny` and npm audit (licenses, vulnerabilities, provenance), locked dependencies.

### Behavior tests
- **Reference renders**: every box has test patches rendered offline (patch + events + seed → samples + hash).
- **Cross-target conformance**: the same corpus produces **the same hashes natively and in WASM**, in CI.
- **A reference hash only changes through a dedicated command**, and the commit explains why. Never silently.
- **Property tests** (`proptest`), at minimum:
  - the same render however it is split into host buffers;
  - two identical renders give the same result;
  - never NaN or infinity in the output.
- **Real-time safety**: no allocation in `process`, measured from the first call on a fresh instance, checked automatically by the counting allocator (`engine-alloc-counter`, re-exported by `engine-testkit`), installed in the tests of every crate that processes audio: each box (box laws), every plan (plan laws over random patches), the WASM engine and its C interface. On the browser's audio thread, no per-block copies or messages: `pnpm test:allocation` (`bindings/wasm`) drives the engine processor from a fresh instance and counts V8's garbage collections, with every tier and with the interpreter alone, and finds none for quanta that are whole engine blocks (Web Audio's 128 frames included). Known deviations, each pinned by a test or ticket: the construction of the engine processor (see the real-time constraint above); a quantum that is not a whole number of blocks allocates until V8 optimizes the copy; the tap processor copies every block (test and measurement only); SpiderMonkey and JavaScriptCore are not measured yet (roadmap #12).
- **Never test only at the default**: every dimension of the environment the code receives is tested across its domain, never at a single value: sample rate (8 kHz to 384 kHz, including 44.1 kHz), how the host splits time (host buffer sizes, including sizes that are not a multiple of the block, random splits), duration, channel count (including 0 and 1), parameter values crossed with rates, special values (0, -0, subnormal, NaN, infinities, Nyquist), patch sizes, browsers and platforms, screen sizes and densities, locale and time zone. Test values deliberately differ from defaults, so that a hard-coded default cannot pass. The shared values come from one test matrix, never retyped. A dimension is covered only when the tests iterate every value of the matrix, in each crate's own tests, and their result depends on it, checked against an independent reference rather than the code against itself (a plan hard-coding 48 kHz must fail).
- **Invariants are tested at the edges of their domain**: every documented invariant and numeric claim is checked at the extremes of every parameter and sample rate and their combinations (zero, negative, Nyquist and above, subnormal, largest accepted sizes), and over long runs (drift, overflow). Every tolerance has a derivation or a measured distribution, written next to it; never a guess. A derivation states its preconditions and holds per regime, each regime tested and each precondition checked by a test reading the real constants; a test reference is exact where it can be (phases reduced in integers), and its own error stays inside the bound.
- **Objective DSP measurements**: every box has measured audio quality in its tests, with thresholds (spectrum, aliasing level, THD, SNR, frequency response, parameter changes without zipper noise), compared with high-precision references. State-of-the-art DSP is a requirement, not a goal.
- **Fuzzing**: patch parsing, the command layer, then native-vs-WASM differential fuzzing.
- **`unsafe` code**: forbidden by default. If truly required, it is isolated, checked with Miri and, for critical parts, proven with Kani.

### Every other aspect
- **Commands and patch model**: every command is tested (success, rejection, undo/redo round trip), with property tests over random command sequences (full undo = initial state, identical serialization round trip).
- **Format compatibility**: every version of the patch format keeps test files. Old patches always load and migrate, checked in CI.
- **Web interfaces (playground, editor)**: the local `doc/interface-testing.md` is the required test strategy, layer by layer (scene, camera and animations, interactions, renderer, accessibility layer, components, end-to-end, visual regression, performance). It is mandatory, heavy on purpose, and applies from the first line of every interface change. In short:
  - unit and component tests;
  - end-to-end tests (Playwright) on **Chrome, Firefox and WebKit**;
  - visual regression via screenshots;
  - accessibility (axe);
  - the sound produced in the browser is captured (`OfflineAudioContext`) and compared to the native hash.
- **Performance**: engine and editor benchmarks (CPU per box, latency, load time, memory) tracked in CI. A regression beyond the set threshold fails CI.
- **Exported artifacts**: each target runs in test mode, its render is compared to the reference hash, and plugins pass the official validators (`pluginval`, `clap-validator`, `auval`, the VST3 validator).
- **Native shells**: Swift/Kotlin unit tests, plus lifecycle tests (interruptions, background, device changes, denied permissions) on simulators and reference devices.
- **Security**: CSP verified, no API key ever sent to the server (checked by tests), fuzzing of everything that reads external input (patches, media, network messages, MCP).
- **AI assistant**: an evaluation suite (requests → valid patch + expected render metrics), with a success rate tracked per model. A drop fails CI.
- **Reproducible builds**: two builds of the same commit produce identical artifacts, checked in CI.

### Deep review
`/deep-review` (`.claude/skills/deep-review/`) runs an exhaustive multi-agent review along every quality axis (seventeen, scientific rigor and edge cases included), verifies each finding adversarially, then analyzes each one (admissibility, options, trade-offs) before anything is applied. Run it before closing a roadmap ticket and before every release. No finding is applied without the maintainer's decision and a fresh analysis at the moment of applying it. Every review also turns the classes of defects it reveals into lessons: preferably a tool that prevents or detects them (a gate, a law, a lint, a type), otherwise a rule in this file, worded to apply beyond the case that revealed it.

### Transparency
Results are meant to be published for every release: coverage, mutants, per-target conformance hashes, known deviations.
