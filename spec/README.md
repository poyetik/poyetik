# spec

The specifications every implementation follows: the **patch format**, the contract shared by the engine, the SDKs, the editor, the AI assistant and MCP; and the **render hash**, by which every target proves it renders the same bits.

| Document | Contents |
|---|---|
| [`patch-format.md`](patch-format.md) | The patch format, version 1 (minimal: boxes, parameters, cables) |
| [`render-hash.md`](render-hash.md) | The render hash, version 1 (`render-hash/1`): the hashed bytes, their order, the versioning rule |
| [`render-hash-vectors.json`](render-hash-vectors.json) | Known-answer vectors of the render hash, read by the tests of every implementation |
| [`safe-message-vectors.json`](safe-message-vectors.json) | Known-answer vectors of how every command-line tool and engine message escapes and bounds untrusted text (control, bidirectional formatting and line separator characters as `\u{..}`, at most 1,000 characters): read by the tests of `engine_patch::safe_message` (Rust) and `oneLine` (`@poyetik/config`, TypeScript) |
| [`engine-design.md`](engine-design.md) | The engine design decisions: what the engine will do (events, block sizes, precision, multichannel, polyphony, latency, hot swap…), why, and in what order; the formats above change with the steps that implement them |

The format grows incrementally (roadmap ticket #3): only what the engine actually supports is specified.
