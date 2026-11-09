# Benchmarks

What the engine's cost benchmarks measure, how their thresholds are derived, what they cannot see, and how to add one (roadmap ticket #12, step 6). They exist so that the cost of the engine's later decisions (`engine-design.md`, implementation order) is measured, not guessed.

| File | Contents | Changed by |
|---|---|---|
| [`benchmark-baseline.json`](benchmark-baseline.json) | The instruction count of every benchmark | `cargo xtask bless-benchmarks` only, in a commit that says why |
| [`benchmark-thresholds.json`](benchmark-thresholds.json) | The tolerance of the counts, and the measured times the construction bound derives from | By hand, in a reviewed commit, with the derivation below |
| `bindings/wasm/benchmarks/` | The instrumenter, the benchmarks, the comparison and the law | Code, under the package's proofs (100% coverage, no surviving mutant) |

Two kinds of measurement, with opposite properties:

1. **Instruction counts** of the engine's WebAssembly module, for each box at representative settings and for building plans of several sizes: deterministic, identical on every machine, compared exactly with the baseline. `pnpm benchmarks:check` in `bindings/wasm` (part of its `pnpm check`, so of `cargo xtask check`).
2. **Wall time** of constructing the engine processor in Chromium, Firefox and WebKit: noisy, asserted only as an upper bound with a wide margin. An end-to-end test of `bindings/wasm` (`pnpm test:e2e`, part of the same check).

## Instruction counts

### How they are counted

The module the browsers run (`bindings/wasm/dist/engine.wasm`, a reproducible build) is rewritten by `benchmarks/instrument.ts`: before every straight-line run of each function, it adds the run's number of instructions to a mutable 64-bit global, exported as `instructions`. A run starts where a branch may land (the start of a function, after `block`, `loop`, `if`, `else`, `end`, and after `br`, `br_if`, `br_table`, `return`, `unreachable`), so whenever one of its instructions runs, all of them do. Calls do not end a run: execution comes back after them.

**What counts as one instruction:** every WebAssembly instruction executed, except the structure markers `block`, `loop`, `else` and `end`, which compute nothing. The instructions the instrumentation adds are not counted.

**Proofs.** Hand-assembled modules with counts worked out by hand (a loop of any number of passes, each arm of an `if`, a call); every understood instruction decoded with its immediates and compared byte for byte with the expected rewriting; every conformance case rendered by the instrumented module with its native reference hash, so the instrumentation changes no result; every instruction the instrumenter does not understand is refused, naming it and its byte, never skipped, so a new compiler feature (vector instructions, tail calls) fails the check until the instrumenter learns it.

**Why it is deterministic.** The count depends only on the bytes of the module and on what it is asked to do. The module is built reproducibly with the pinned toolchain, so the counts are the same on a developer's Mac and on the CI runner, whatever the load of the machine: a baseline is shared by every machine, and a change of a count always comes from a change of the code, the compiler or the benchmark.

### What they do not see

A WebAssembly instruction count is a proxy for cost, not a time:

- **Native code**: each browser compiles the module to machine code with its own compilers and tiers; one WebAssembly instruction becomes zero, one or many machine instructions, differently in each engine. The native engine (`engine-cli`, plugins) is not measured at all.
- **Memory and caches**: a load that misses every cache counts as much as one that hits; the cost of memory bandwidth (decision 10's 64-bit samples) is invisible.
- **Vector width**: a vector instruction counts as one, however many lanes it computes (the module has no vector instructions today; the instrumenter refuses them until it is taught them).
- **Instruction cost**: a division, a square root and an addition each count as one.

So a count catches what changes the work done (an algorithm, a loop, an approximation, a copy); it does not rank two designs whose work differs in kind. Wall-time measurements on each browser engine, per box, are planned with the 32-bit precision setting (step 11), which needs them to measure the real ratio decision 10 speaks of.

### What is measured

**Each box at representative settings** (`BOX_BENCHMARKS` in `benchmarks/suite.ts`): the instructions of `BLOCKS` blocks of a patch holding the box, minus those of the same blocks of the same patch without it, each in a fresh instance. So a count is the box's `process` plus what the plan does for it each block (gathering its inputs, copying its outputs into the bus), which is what a patch pays for it. The settings cover low and high sample rates and frequencies, a constant output, and an output fed or not; their names are in the baseline.

**Building plans** (`PLAN_SIZES`, at `PLAN_RATE`): the instructions of loading a patch of n oscillators feeding the two inlets of one `dac~` in turn (n + 1 boxes, n cables), copying the patch into the module, parsing it and building its plan included: what the engine processor does when it is constructed (`engine-design.md`, decision 15).

### The n log n law

`Plan::build` sorts the boxes in O((N + E) log N) (`engine-core`, `plan.rs`). The check asserts it on the counts: for each plan size and the next, the ratio of their counts must not exceed the ratio of n log2 n. A cost a·n log n + b·n + c with non-negative terms always passes (its ratio is at most that of n log n), so the law holds whatever the constant parts; a quadratic term fails as soon as it dominates, since a quadratic cost grows as the square of the ratio of sizes, more than n log n allows for any sizes above 2. Unit tests prove the law accepts n log n, linear and constant costs at its edge and refuses a quadratic cost; `bless` refuses to write a baseline that breaks it.

Today's boxes have either no inlet (`osc~`) or no outlet (`dac~`), so the plans are bipartite; deeper topologies join the plan benchmarks with the first box that has both.

### Threshold: the tolerance

`tolerance_percent` of `benchmark-thresholds.json`, in both directions, for every count.

**Derivation.** The counts have no noise: a count differs from the baseline only when the code, the compiler or a benchmark changed. So the tolerance does not absorb noise; it decides which change needs a reviewed new baseline:

- **above**: a regression of a box or of plan building by more than the tolerance fails the check. A smaller drift (a refactoring of the plan's per-block work, a compiler upgrade reordering code) passes, so that unrelated work does not force a re-bless at every commit; drifts do not accumulate unseen, since the baseline only moves through a reviewed bless and each change is compared with it.
- **below**: a count that fell by more than the tolerance also fails, so that the baseline stays tight: an improvement is recorded by a bless, and a later regression is measured from it, not from the old count.

### Changing the baseline

Only through `cargo xtask bless-benchmarks`, which builds the module, counts every benchmark and rewrites `benchmark-baseline.json` (refusing to when the n log n law fails). The commit that changes the baseline says why the counts changed. Lowering a count by a bless is always allowed; raising one needs the reason in that commit, with what the extra cost buys.

## Construction of the engine processor

The engine processor compiles the module, instantiates it, parses the patch and builds its plan on the audio thread, once, before it renders (`engine-design.md`, decision 15; a known deviation of CLAUDE.md's real-time constraint). The end-to-end test "constructs the engine processor within its bound, every time" times it `runs` times per browser engine (Chromium, Firefox, WebKit, at desktop and iPhone sizes), each time from creating an engine node (copying the module and the patch into its options included) to its processor's `ready` message, on the bench page's "Time construction" button, with the sine-440 patch. It prints the times it measured.

**What is asserted:** every time is at most the bound of its browser engine, `ceil(measured_max_ms × margin)` (`constructionBoundMs` in `benchmarks/suite.ts`), from `benchmark-thresholds.json`.

**Derivation.** Wall time on a shared machine is noisy, and browsers coarsen their timers (the bench page reads tenths of a millisecond in Chromium and whole milliseconds in Firefox and WebKit). `measured_max_ms` is the slowest of the constructions measured as `measured_on` says; `margin` covers a slower CI runner, a loaded machine and timer granularity. Such a bound catches what changes the construction by an order of magnitude (an eager compilation of a much larger module, a plan built in quadratic time, a copy of the module per block); it cannot catch a regression of a few percent, which is what the instruction counts of plan building are for.

**What CI cannot run yet.** The measured values come from the maintainer's Mac; no CI run has completed yet (CLAUDE.md, CI), so the bounds are not yet proven on the x86_64 Linux runner, whose WebKit is WebKitGTK. If the first CI runs show slower constructions, the measurement is taken again on the runner and recorded (`measured_on` names where), in a reviewed commit: never by widening the margin to pass.

**Measuring again.** Run `pnpm test:e2e` in `bindings/wasm` several times on an idle machine; each run prints the times per browser engine. Record the slowest per engine in `measured_max_ms`, and where, when and how many in `measured_on`.

## Adding a benchmark

For a new box type (its registration in `engine-objects`' `REGISTRY` requires one: a test of `engine-objects` refuses a registry and a baseline that name different box types):

1. Add its settings to `BOX_BENCHMARKS` (`bindings/wasm/benchmarks/suite.ts`): a `setting` in words, a sample rate, a `patch` holding the box, and `without`, the same patch without it. Choose settings that change its cost (rates, parameters that change its work, inlets fed or not), including the extremes of its parameters; give a box with inlets a source in both patches, so that only the box itself differs.
2. Update the literal list of settings in `test/benchmarks/suite.test.ts`.
3. Run `cargo xtask bless-benchmarks`, and commit the new baseline with the box, saying that a box was added.

For a new kind of benchmark (an engine component, a topology): add it to `benchmarks/suite.ts` and to the shape of the baseline (`parseBaseline`, `formatBaseline`), with its tests, then bless. For a new scaling claim, state it as a law like the n log n law, with a unit test that it refuses a cost that breaks it.
