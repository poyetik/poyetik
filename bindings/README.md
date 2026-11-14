# bindings

Thin **developer SDKs** around the libraries' cores. All the logic stays in the core; each binding only adapts it to one environment, and each one passes the same conformance corpus.

- [`audio-wasm/`](audio-wasm/): `audio-wasm`, the engine in browsers (`AudioWorklet`) and in Node.
- [`visual-wasm/`](visual-wasm/): `visual-wasm`, the visual library in browsers and in Node.

Planned: a Rust crate, an npm package and a C library (#32 steps 2, 6 and 7).
