# spec

The specifications every implementation follows: the **patch format**, the contract shared by the engine, the SDKs, the editor, the AI assistant and MCP; the **render hash**, by which every target proves it renders the same bits; the **image hash**, by which the visual library does the same for images; and the **measurements**, by which every box proves its quality. The design decisions of the engine and of the visual library come with them.

| Document | Contents |
|---|---|
| [`patch-format.md`](patch-format.md) | The patch format, version 1 (minimal: boxes, parameters, cables) |
| [`render-hash.md`](render-hash.md) | The render hash, version 2 (`render-hash/2`): the render format (precision and sample width) and the arithmetic mode it records, the hashed bytes, their order, the versioning rule |
| [`render-hash-vectors.json`](render-hash-vectors.json) | Known-answer vectors of the render hash, read by the tests of every implementation |
| [`safe-message-vectors.json`](safe-message-vectors.json) | Known-answer vectors of how every command-line tool and engine message escapes and bounds untrusted text (control, bidirectional formatting and line separator characters as `\u{..}`, at most 1,000 characters): read by the tests of `audio_patch::safe_message` and of the copies of `visual-cli`, `visual-gpu` and `xtask` (Rust), and of `oneLine` (`@poyetik/config`, TypeScript) |
| [`measurements.md`](measurements.md) | The objective measurements (spectrum, THD+N, THD, SNR, SFDR, aliasing level): definitions and the standards they follow, the error of the measurements themselves, the bounds a sine meets, the thresholds of `osc~` in each render format, the exact measurements of the visual square, and how to add measurements for a new box or engine component |
| [`measurement-vectors.json`](measurement-vectors.json) | Known-answer vectors of the measurements and of the bounds, read by the tests of `audio_testkit::measure` (Rust), `osc~` and the oracle (TypeScript) |
| [`benchmarks.md`](benchmarks.md) | The cost benchmarks: what is counted, how it is judged, and the record of the design choices measured with them |
| [`benchmark-baseline.json`](benchmark-baseline.json) | The baseline of [`benchmarks.md`](benchmarks.md) every check compares with, written only by `cargo xtask bless-benchmarks` |
| [`visual-benchmark-baseline.json`](visual-benchmark-baseline.json) | The visual library's counts of [`benchmarks.md`](benchmarks.md) ("Visual library") every check compares with, written only by `cargo xtask bless-benchmarks` |
| [`gpu-time-runs.txt`](gpu-time-runs.txt) | Every run the GPU's time bound is derived from, and those recorded beside them (session, time, load, power, condition, medians, renders measured), read only by the law tests of `visual-gpu`, which recompute the bound and the figures `states_the_runs_figures_as_the_record_gives_them` lists; of the one GPU, system and driver the baseline holds; never edited: a new session adds its rows, a new idle session (whatever its trigger) or another GPU, system or driver starts it anew |
| [`gpu-time-baseline.txt`](gpu-time-baseline.txt) | The GPU's time to render the square (the median of 5 runs), one line per GPU, which `visual-gpu check` compares with on that GPU, written only by `visual-gpu bless` (through `cargo xtask bless-benchmarks`) |
| [`benchmark-thresholds.json`](benchmark-thresholds.json) | The tolerance of the counts and the measured construction times the browser bounds derive from |
| [`engine-design.md`](engine-design.md) | The engine design decisions: what the engine will do (events, engine blocks, precision, multichannel, polyphony, latency, hot swap, the control boundary, payloads, randomness, the box contract, resources…), why, and in what order; the formats above change with the steps that implement them |
| [`box-contract.md`](box-contract.md) | The box contract across a module boundary (the ABI of decision 25): its version, the buffers, the canonical NaN, the metadata and the module; version 0 until #12 step 18 |
| [`image-hash.md`](image-hash.md) | The image hash, version 1 (`image-hash/1`): the hashed bytes of a sequence of images, their order, the versioning rule |
| [`image-hash-vectors.json`](image-hash-vectors.json) | Known-answer vectors of the image hash, read by the tests of every implementation |
| [`visual-design.md`](visual-design.md) | The visual library's design decisions for its first image (provisional): a standalone library, exact time, images, exact coverage, the visual box contract, the square, the structure and laws, the exact CPU reference renderer, the display encoding, and what is not decided yet |
| [`limitations.md`](limitations.md) | What the project does not prove, or proves only in part, and the trade-offs it accepted, each with where it is decided or tracked |

The format grows incrementally, step by step of #3: only what the engine actually supports is specified.

## Files only a tool writes

Each is read back by its check only exactly as its tool writes it, an emptied file refused, never read as holding nothing (`CLAUDE.md`, "Scripts are tested functions"). The test proving it is named here, a Rust test by its name and a TypeScript one by its title: `cargo xtask check` requires every Rust test named in this folder to exist (`spec_names_gate`), every title the table names to be a TypeScript test's, and every test whose name says it reads a file only as its tool writes it to be named in the table (`reader_tests_gate`), so that a reader's proof renamed, removed or left out is seen.

| File | Written by | Its check's reader proven by | Emptied, refused by |
|---|---|---|---|
| `spec/gpu-time-baseline.txt` | `cargo xtask bless-benchmarks` | `reads_back_the_baseline_it_writes_and_refuses_any_other_text` and `holds_a_baseline_as_bless_writes_it` (`visual-gpu`), which round-trip it, `reads_the_baseline_only_as_bless_writes_it` holding the refusals | the same tests |
| `spec/visual-benchmark-baseline.json` | `cargo xtask bless-benchmarks` | "reads the counts only as bless writes them, in the measurements' order" (`tooling/wasm-build/test/bench.test.ts`) | "refuses what is not a JSON object of whole counts" (the same file) |
| `conformance/references.json`, `conformance/algorithms.json` | `cargo xtask bless` | `check_command_reads_its_files_only_as_bless_writes_them` (`audio-cli`) | the same test |
| `conformance/visual/references.json` | `cargo run -p visual-cli -- bless conformance/visual` | `reads_the_references_only_as_bless_writes_them` (`visual-cli`); the WebAssembly modules' conformance runner, `parseVisualReferences`: "reads the references only as visual-cli bless writes them, in the order of their names' UTF-8 bytes" (`bindings/visual-wasm/test/conformance.test.ts`) | the same tests |
| `conformance/visual/display.json` | `cargo run -p visual-cli -- bless conformance/visual` | `checks_the_display_corpus_only_as_bless_writes_it` (`visual-cli`); the GPU's tests, `reads_the_display_corpus_only_as_visual_cli_writes_it` (`visual-gpu`); the WebAssembly modules' conformance runner and the playground's end-to-end tests, `displayCorpus`: "reads the display corpus only as visual-cli writes it" (`bindings/visual-wasm/test/conformance.test.ts`) | `names_where_a_file_first_differs_from_the_corpus` (`visual-cli`) |

The checking readers that still accept other layouts, the engine benchmarks' baseline (`spec/benchmark-baseline.json`) and the live limiter's references (`conformance/live-references.json`), join this table with roadmap #24 step 169.
