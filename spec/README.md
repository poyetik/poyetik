# spec

The specifications every implementation follows: the **patch format**, the contract shared by the engine, the SDKs, the editor, the AI assistant and MCP; the **render hash**, by which every target proves it renders the same bits; the **image hash**, by which the visual library does the same for images; and the **measurements**, by which every box proves its quality. The design decisions of the engine and of the visual library come with them.

| Document | Contents |
|---|---|
| [`patch-format.md`](patch-format.md) | The patch format, version 1 (minimal: boxes, parameters, cables) |
| [`render-hash.md`](render-hash.md) | The render hash, version 2 (`render-hash/2`): the render format (precision and sample width) and the arithmetic mode it records, the hashed bytes, their order, the versioning rule |
| [`render-hash-vectors.json`](render-hash-vectors.json) | Known-answer vectors of the render hash, read by the tests of every implementation |
| [`safe-message-vectors.json`](safe-message-vectors.json) | Known-answer vectors of how every command-line tool and engine message escapes and bounds untrusted text (control, bidirectional formatting and line separator characters as `\u{..}`, at most 1,000 characters): read by the tests of `audio_patch::safe_message` and of `visual-cli`'s copy (Rust), and of `oneLine` (`@poyetik/config`, TypeScript) |
| [`measurements.md`](measurements.md) | The objective measurements (spectrum, THD+N, THD, SNR, SFDR, aliasing level): definitions and the standards they follow, the error of the measurements themselves, the bounds a sine meets, the thresholds of `osc~` in each render format, the exact measurements of the visual square, and how to add measurements for a new box or engine component |
| [`measurement-vectors.json`](measurement-vectors.json) | Known-answer vectors of the measurements and of the bounds, read by the tests of `audio_testkit::measure` (Rust), `osc~` and the oracle (TypeScript) |
| [`benchmarks.md`](benchmarks.md) | The cost benchmarks: what is counted, how it is judged, and the record of the design choices measured with them |
| [`benchmark-baseline.json`](benchmark-baseline.json) | The baseline of [`benchmarks.md`](benchmarks.md) every check compares with, written only by `cargo xtask bless-benchmarks` |
| [`benchmark-thresholds.json`](benchmark-thresholds.json) | The tolerance of the counts and the measured construction times the browser bounds derive from |
| [`engine-design.md`](engine-design.md) | The engine design decisions: what the engine will do (events, block sizes, precision, multichannel, polyphony, latency, hot swap, the control boundary, payloads, randomness, the box contract, resources…), why, and in what order; the formats above change with the steps that implement them |
| [`image-hash.md`](image-hash.md) | The image hash, version 1 (`image-hash/1`): the hashed bytes of a sequence of images, their order, the versioning rule |
| [`image-hash-vectors.json`](image-hash-vectors.json) | Known-answer vectors of the image hash, read by the tests of every implementation |
| [`visual-design.md`](visual-design.md) | The visual library's design decisions for its first image (provisional): a standalone library, exact time, images, exact coverage, the visual box contract, the square, the structure and laws, the exact CPU reference renderer, and what is not decided yet |
| [`limitations.md`](limitations.md) | What the project does not prove, or proves only in part, and the trade-offs it accepted, each with where it is decided or tracked |

The format grows incrementally, step by step of #3: only what the engine actually supports is specified.
