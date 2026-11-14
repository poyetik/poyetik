# Limitations and trade-offs

What the project does not prove, or proves only in part, and the trade-offs it accepted, in one place. Each entry says what is limited, why, what it costs, and where it is decided or tracked; the decision or the item it names is the source, this page only gathers them. An entry leaves this page when the roadmap step it names removes it.

## Bit-identical renders

- **A browser that flushes subnormals on its audio thread.** Firefox on x86_64 runs its audio thread, and the engine's WebAssembly on it, with subnormal operands and results flushed to zero, which no page can read or change; Chromium and WebKit keep IEEE arithmetic, and on aarch64 all three do (measured; [`engine-design.md`](engine-design.md), decision 11). Where the engine flushes, no bit changes; elsewhere, a live render in that browser equals the native one only as long as no subnormal arises outside the engine's flush points: true of every case the browser tests play live, not proven for every patch. The difference is far below anything audible. The engine reports the arithmetic it observes when it starts. Accepted: avoiding it would cost every user latency or speed, for one browser on one architecture.
- **The bits of a NaN.** A sum or a product whose result is NaN gives a NaN whose payload and sign may differ between targets and builds ([`engine-design.md`](engine-design.md), decision 28; #24 step 2 decides how sums treat NaN).

## Real time

- **Constructing the engine processor** compiles the module, instantiates it and builds the plan on the audio thread, once, before it renders (measured; to be bounded by #12 step 30).
- **The tap processor** copies and posts every render quantum while the playground's oscilloscope runs (until #12 step 21).
- **Allocation in the browser** is measured in V8 only; SpiderMonkey and JavaScriptCore are not measured yet (#12 step 21). The known deviations of the measured path are listed with the allocation test (`bindings/wasm`, `pnpm test:allocation`).

## What CI runs on

- **One CI platform.** CI runs on x86_64 Linux. On aarch64 (macOS on Apple silicon), native conformance and the native floating-point mode are proven only by the pre-push check of the maintainer's Mac, until a macOS job (#20 step 1).
- **Browsers are Playwright's builds.** "WebKit" is Playwright's WebKit, not Safari, and "mobile WebKit" is the same engine at a phone's size, not iOS Safari; real phones are tested by #18 step 5.
- **No sound card.** In CI, live playback is proven to reach an audio output, never a device.

## Proofs

- **The default check proves the changes only.** It mutates the changed code; only the full check (`cargo xtask check --full`, CI) proves the rest. A green default check does not imply a green full check.
- **Branch coverage merges generic instantiations.** A branch taken by one precision only counts as covered for both (#24 step 87).
- **Review only.** What no tool can check is reviewed instead, and each case says so where it is stated: the files a doc test loads, the test data paths computed at run time, sums written inside box code, and the provenance of npm packages, which is not checked yet.
