# Benchmarks

What the engine's cost benchmarks measure, how their thresholds are derived, what they cannot see, and how to add one (roadmap #12 step 3). They exist so that the cost of the engine's later decisions (`engine-design.md`, implementation order) is measured, not guessed.

| File | Contents | Changed by |
|---|---|---|
| [`benchmark-baseline.json`](benchmark-baseline.json) | The instruction count of every benchmark, and the size of the engine module (`module_bytes`) | `cargo xtask bless-benchmarks` (which runs the preflight of the check, builds the module, then `node scripts/benchmarks.ts bless` in `bindings/wasm`, which no package script runs), in a commit that says why: a process rule; `check` refuses a count beyond the tolerance, an entry nothing measures and a benchmark missing, so only a hand edit within the tolerance would pass it |
| [`benchmark-thresholds.json`](benchmark-thresholds.json) | The tolerance of the counts, and the measured times the construction bound derives from | By hand, in a reviewed commit, with the derivation below |
| `bindings/wasm/benchmarks/` | The instrumenter, the benchmarks, the comparison and the law | Code, under the package's proofs (100% coverage, no surviving mutant) |

Two kinds of measurement, with opposite properties:

1. **Instruction counts** of the engine's WebAssembly module, for each box at representative settings, for the plan's own sums, and for building plans of several sizes, with the size of the module itself: deterministic, identical on every machine, compared exactly with the baseline. `pnpm benchmarks:check` in `bindings/wasm` (part of its `pnpm check`, so of `cargo xtask check`).
2. **Wall time** of constructing the engine processor, and of rendering each box in both precisions, in Chromium, Firefox and WebKit: noisy, asserted only as upper bounds with a wide margin. End-to-end tests of `bindings/wasm` (`pnpm test:e2e`, part of the same check).

## Instruction counts

### How they are counted

The module the browsers run (`bindings/wasm/dist/engine.wasm`, a reproducible build) is rewritten by `benchmarks/instrument.ts`: before every straight-line run of each function, it adds the run's number of instructions to a mutable 64-bit global, exported as `instructions`. A run starts where a branch may land (the start of a function, after `block`, `loop`, `if`, `else`, `end`, and after `br`, `br_if`, `br_table`, `return`, `unreachable`), so whenever one of its instructions runs, all of them do. Calls do not end a run: execution comes back after them. Right before every bulk memory instruction (`memory.copy`, `memory.fill`, `memory.init`), it also adds the words that instruction moves, computed from the length it is given.

**What counts as one instruction:** every WebAssembly instruction executed, except the structure markers `block`, `loop`, `else` and `end`, which compute nothing; and every 8-byte word a bulk memory instruction moves, ceil(length / 8) on top of the instruction itself, since its work grows with its length (one copy of a 64-frame block of binary64 samples counts 1 + 64). A word is charged as one instruction, where a loop copying it would execute a few: the charge is a floor of the work, never zero, so a copy added to every block shows in the counts. The instructions the instrumentation adds are not counted. A bulk table instruction (`table.copy`, `table.fill`, `table.init`), whose work also grows with its operand, is refused, as the engine module holds none; growing the memory or a table counts as one, its work being the host's.

**Proofs.** Hand-assembled modules with counts worked out by hand (a loop of any number of passes, each arm of an `if`, a call, copies and fills of 0, 1, 7, 8, 9, 512 and 65,536 bytes, a fill in a loop, and a length of 2^32 - 1 bytes charged before the module traps); every understood instruction decoded with its immediates and compared byte for byte with the expected rewriting; every conformance case rendered by the instrumented module with its native reference hash, so the instrumentation changes no result; every instruction the instrumenter does not understand is refused, naming it and its byte, never skipped, so a new compiler feature (vector instructions, tail calls) fails the check until the instrumenter learns it.

**Why it is deterministic.** The count depends only on the bytes of the module and on what it is asked to do. The module is built reproducibly with the pinned toolchain, so the counts are the same on a developer's Mac and on the CI runner, whatever the load of the machine: a baseline is shared by every machine, and a change of a count always comes from a change of the code, the compiler or the benchmark.

### What they do not see

A WebAssembly instruction count is a proxy for cost, not a time:

- **Native code**: each browser compiles the module to machine code with its own compilers and tiers; one WebAssembly instruction becomes zero, one or many machine instructions, differently in each engine. The native engine (`engine-cli`, plugins) is not measured at all.
- **Memory and caches**: a load that misses every cache counts as much as one that hits; the cost of memory bandwidth (decision 10's 64-bit samples) is invisible.
- **Vector width**: a vector instruction counts as one, however many lanes it computes (the module has no vector instructions today; the instrumenter refuses them until it is taught them).
- **Instruction cost**: a division, a square root and an addition each count as one; a bulk copy counts one per word it moves, the floor of what a loop would execute for it.

So a count catches what changes the work done (an algorithm, a loop, an approximation, a copy, a bulk copy by the words it moves); it does not rank two designs whose work differs in kind. The wall time of each box in each precision on each browser engine ("Rendering each box" below) measures what counts cannot: the real binary32/binary64 ratio decision 10 speaks of.

### What is measured

**Each box at representative settings and in both precisions** (`BOX_BENCHMARKS` in `benchmarks/suite.ts`; each entry of the baseline names its `precision`): the instructions of `BLOCKS` blocks of a patch holding the box, minus those of the same blocks of the same patch without it, each in a fresh instance, computed in that precision and given as binary32, as the browser's audio thread is given it (so a binary64 count includes rounding the box's share of the output to binary32). So a count is the box's `process` plus what the plan does for it each block (gathering its inputs, its outputs written straight into the bus slots the cables read), which is what a patch pays for it. The settings cover low and high sample rates and frequencies, a constant output, and an output fed or not; their names are in the baseline.

**The plan's sums, in both precisions** (`MIX_BENCHMARKS` in `benchmarks/suite.ts`, under `mixes` in the baseline, named `mixing <setting> in <precision>`): what the plan does per block to add up several terms in the reduction order (`engine-design.md`, decision 28), which no box benchmark sees. Each is the instructions of `BLOCKS` blocks of a patch summing several terms into one place, minus those of the same boxes summing one (which the plan feeds by a plain copy, decided when it is built: so each difference is the sum against a copy), each in a fresh instance, computed in that precision and given as binary32: n oscillators into one inlet of a `dac~` minus the same oscillators with only the first cable, for n = 2 (one addition), 3 (a partial tree), 16 (the full tree) and 17 (a lane holding two terms); and two `dac~` fed by one oscillator on the same channels minus one `dac~` (whose difference also holds the second box's own work, which its box benchmark counts).

**Building plans** (`PLAN_SIZES`, at `PLAN_RATE`): the instructions of loading a patch of n oscillators feeding the two inlets of one `dac~` in turn (n + 1 boxes, n cables), copying the patch into the module, parsing it and building its plan included: what the engine processor does when it is constructed (`engine-design.md`, decision 15).

**The size of the engine module** (`module_bytes` in the baseline, reported as `engine module: <n> bytes`): the bytes of `bindings/wasm/dist/engine.wasm` as built, before instrumentation. What a page downloads and compiles before it plays, and what the engine processor compiles on the audio thread (decision 15): the deterministic proxy of load time, as instruction counts are of CPU time, deterministic for the same reason (the build is reproducible). It is compared with the same tolerance, either way; the absolute budget of the module and a committed digest of its bytes are roadmap #11's.

### The n log n law

`Plan::build` sorts the boxes in O((N + E) log N) (`engine-core`, `plan.rs`). The check asserts it on the counts: for each plan size and the next, the ratio of their counts must not exceed the ratio of n log2 n. A cost a·n log n + b·n + c with non-negative terms always passes (its ratio is at most that of n log n), so the law holds whatever the constant parts; a quadratic term fails as soon as it dominates, since a quadratic cost grows as the square of the ratio of sizes, more than n log n allows for any sizes above 2. The law holds only for plans of 2 oscillators or more (n log n is 0 at 1), each costing something, in increasing size: it reports a plan outside that as a problem, never passes it. Unit tests prove the law accepts n log n, linear and constant costs at its edge and refuses a quadratic cost and every plan outside its domain; `bless` refuses to write a baseline that breaks it.

Today's boxes have either no inlet (`osc~`) or no outlet (`dac~`), so the plans are bipartite; deeper topologies join the plan benchmarks with the first box that has both. The plan's per-block work on fan-in (several cables into one inlet, several output boxes on one channel) is measured by the mixing benchmarks above.

### Threshold: the tolerance

`tolerance_percent` of `benchmark-thresholds.json`, in both directions, for every count.

**Derivation.** The counts have no noise: a count differs from the baseline only when the code, the compiler or a benchmark changed. So the tolerance does not absorb noise; it decides which change needs a reviewed new baseline:

- **above**: a regression of a box, of the plan's sums, of plan building or of the module's size by more than the tolerance fails the check. A smaller drift (a refactoring of the plan's per-block work, a compiler upgrade reordering code) passes, so that unrelated work does not force a re-bless at every commit; drifts do not accumulate unseen, since the baseline only moves through a reviewed bless and each change is compared with it.
- **below**: a count that fell by more than the tolerance also fails, so that the baseline stays tight: an improvement is recorded by a bless, and a later regression is measured from it, not from the old count.

So the tolerance is positive and below 100 (the reader refuses any other): a count cannot fall by 100% or more, so a wider tolerance could never refuse a fall. The baseline's reader also refuses any key it does not name and an entry repeated (the comparison is by name, so a second entry would hide the first), and `measure` refuses two benchmarks of one name.

### Changing the baseline

Through `cargo xtask bless-benchmarks` (the process rule above), which builds the module, counts every benchmark and rewrites `benchmark-baseline.json` (refusing to when the n log n law fails). The commit that changes the baseline says why the counts changed. Lowering a count by a bless is always allowed; raising one needs the reason in that commit, with what the extra cost buys.

### The cost of each precision

`benchmark-baseline.json` gives every box in both precisions; what its counts show, binary32 against binary64 given as binary32 (the counts themselves are in the baseline, the one source):

- `osc~` computes its sine with `libm`'s binary32 sine in binary32, about a third fewer instructions than the binary64 sine; at 0 Hz both sines take their shortest path, and rounding the angle to binary32 makes binary32 the dearer.
- `dac~`'s binary64 count holds the per-sample rounding of its channels to binary32 for delivery, which binary32 does not pay (binary32 is delivered as it is); a patch pays it once per output sample, not per box. Which part of that rounding costs most is not measured yet: roadmap #12 step 22 breaks it down.
- What the counts cannot see (above): a binary64 sample is twice the memory and bandwidth of a binary32 one, a vector holds half as many (the module has no vector instructions yet), and an instruction of the binary64 sine counts as one of the binary32 sine whatever their latencies. The wall times of "Rendering each box" measure the real ratio on each browser engine.

### Same cost through every interface

No architecture choice may cost performance (`engine-design.md`, decision 33): the libraries are crates of one program and one WebAssembly module, and an interface between them costs nothing measurable against calling the code directly: today the instruction counts hold it within the counts' tolerance; each comparison below, when it lands, is exact or within a per-call allowance derived and written for its interface, since the tolerance alone would let an interface cost up to the tolerance of a count. The instruction counts prove it, never an argument:

- **The box contract's ABI** (roadmap #12 step 13): every box benchmark is counted twice, the box called directly and through the ABI form, with the same patch, rate and precision; the count through the ABI is held to the direct count exactly, or within a per-call allowance derived and written for the ABI (its canonical NaNs, decision 25, are work per sample), as the renders are held to the same bits.
- **The public interface** (roadmap #32 step 8): the engine driven through its public interface (Rust, the C interface, TypeScript over the module) is counted against the same patches driven directly, exactly or within a per-call allowance written for each interface.
- **The precedent**: the sample newtypes `Binary64` and `Binary32` (decision 10) added no instruction measured when they replaced plain floats, and are held within the counts' tolerance since (roadmap #24 step 86 makes that comparison exact).
- **The plan's call of each box**: a built-in box type is called directly, through `engine_objects::BuiltIn` (decision 33); only a box type loaded at run time goes through its virtual table, one indirect call per box per block. Measured on 2026-11-11 against the indirect call of every box it replaced, per 64 blocks: `osc~` 192 (binary64) and 256 (binary32) instructions less at every setting (of about 758,000 in binary64 at 440 Hz, counted before bulk memory instructions were charged the words they move, which both sides pay alike), `dac~` 384 to 640 less, two output boxes 448 less, the mixes unchanged; loading plans of 256 to 16,384 oscillators 0.03% to 0.05% more (+104,488 at 16,384) and the module 1,274 bytes more, the price accepted for the faster real-time path. Of the forms measured, this one keeps a box's state on the heap, as the virtual table does: held inline in the enum, every instance as large as the largest box type, `osc~` saved 320 to 384 instructions but loading rose up to 0.26%; with the virtual table's own instances held behind one more pointer, loading rose 0.02% to 0.03% but `osc~` saved at most 64.
- **A box's parameters**: a box is created with `engine_core::Params`, a borrowed slice passed by value (two words). Passing a reference to the owned values instead made loading a plan of 16,384 oscillators 180,235 instructions dearer (measured on 2026-11-11, about 11 per box); the borrowed form leaves every loading count as it was.

A benchmark through an interface that exceeds its direct count beyond its allowance fails the check like any other regression; the interface is changed, never the allowance nor the tolerance.

## Construction of the engine processor

The engine processor compiles the module, instantiates it, parses the patch and builds its plan on the audio thread, once, before it renders (`engine-design.md`, decision 15; a known deviation of CLAUDE.md's real-time constraint). The end-to-end test "constructs the engine processor within its bound, every time" times it `runs` times per browser engine (Chromium, Firefox, WebKit, at desktop and iPhone sizes), each time from creating an engine node (copying the module and the patch into its options included) to its processor's `ready` message, on the bench page's "Time construction" button, with the sine-440 patch. It prints the times it measured.

**What is asserted:** every time is at most the bound of its browser engine, `ceil(measured_max_ms × margin)` (`constructionBoundMs` in `benchmarks/suite.ts`), from `benchmark-thresholds.json`.

**Derivation.** Wall time on a shared machine is noisy, and browsers coarsen their timers (the bench page reads tenths of a millisecond in Chromium and whole milliseconds in Firefox and WebKit). `measured_max_ms` is the slowest of the constructions measured as `measured_on` says; `margin` covers a slower CI runner, a loaded machine and timer granularity. Such a bound catches what changes the construction by an order of magnitude (an eager compilation of a much larger module, a plan built in quadratic time, a copy of the module per block); it cannot catch a regression of a few percent, which is what the instruction counts of plan building are for.

**Where the values come from.** CI runs these bounds, but the measured values come from the maintainer's Mac, never from the runner; the margin is what must cover the x86_64 Linux runner, whose WebKit is WebKitGTK. A bound a CI run fails is measured again on the runner and recorded (`measured_on` names where), in a reviewed commit: never by widening the margin to pass.

**Measuring again.** Run `pnpm test:e2e` in `bindings/wasm` several times on an idle machine; each run prints the times per browser engine. Record the slowest per engine in `measured_max_ms`, and where, when and how many in `measured_on`.

## Rendering each box

The end-to-end test "renders every box benchmark in both precisions within its bound, every time, and prints the cost ratio" times, on the bench page's own thread (the module and the browser engine compiling it are those of the worklet), for every setting of `BOX_BENCHMARKS` and both precisions given as binary32, `blocks` blocks (of the engine's block size, `engine_block_size`, rendered in Web Audio's 128-frame quanta) `runs` times after one warm-up, each in a fresh engine, with and without the box, in Chromium, Firefox and WebKit (desktop and iPhone sizes). It prints every time and the binary32/binary64 cost ratio of the box, from the difference of the medians with and without it: the real ratio decision 10 speaks of, which the instruction counts cannot see (memory, vector width, the cost of each instruction on each browser engine's compiler).

**What is asserted:** every time of the patch holding the box is at most `ceil(measured_max_ms × margin)` of its browser engine, from the `boxes` entry of `benchmark-thresholds.json`. The whole render is bounded, not the difference: a difference of two noisy times can be near zero or negative (an unconnected `dac~`), and the whole render bounds the box's cost from above.

**Derivation.** As for the construction above: `measured_max_ms` is the slowest render measured as `measured_on` says, `margin` covers a slower CI runner, a loaded machine and timer granularity (whole milliseconds in Firefox and WebKit, so `blocks` makes every render last tens of milliseconds). It catches what changes a box's cost by an order of magnitude; the ratio it prints is a measurement, not a threshold, and differences of a few percent are for the instruction counts.

**Where the values come from** and **measuring again**: as for the construction above; CI runs these bounds, and the values come from the maintainer's Mac.

## Adding a benchmark

For a new box type (its registration in `engine-objects`' `REGISTRY` requires one: a test of `engine-objects` refuses a registry and a baseline that name different box types):

1. Add its settings to `BOX_BENCHMARKS` (`bindings/wasm/benchmarks/suite.ts`): a `setting` in words, a sample rate, a `patch` holding the box, and `without`, the same patch without it. Choose settings that change its cost (rates, parameters that change its work, inlets fed or not), including the extremes of its parameters; give a box with inlets a source in both patches, so that only the box itself differs.
2. Update the literal list of settings in `test/benchmarks/suite.test.ts`.
3. Run `cargo xtask bless-benchmarks`, and commit the new baseline with the box, saying that a box was added.

For a new kind of benchmark (an engine component, a topology): add it to `benchmarks/suite.ts` and to the shape of the baseline (`parseBaseline`, `formatBaseline`), with its tests, then bless. For a new scaling claim, state it as a law like the n log n law, with a unit test that it refuses a cost that breaks it.
