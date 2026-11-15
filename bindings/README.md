# bindings

Thin **developer SDKs** around the libraries' cores. All the logic stays in the core; each binding only adapts it to one environment, and each one passes its library's conformance corpus, with the hashes of the native renderer.

- [`audio-wasm/`](audio-wasm/): `audio-wasm`, the engine in browsers (`AudioWorklet`) and in Node.
- [`visual-wasm/`](visual-wasm/): `visual-wasm`, the visual library in Node, and in browsers once the playground draws with it (#34 step 6).

Planned: a Rust crate, an npm package and a C library (#32 steps 2, 6 and 7).
