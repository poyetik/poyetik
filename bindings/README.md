# bindings

Thin **developer SDKs** around the engine core. All the logic stays in the core; each binding only adapts it to one environment, and each one passes the same conformance corpus.

- [`wasm/`](wasm/): `engine-wasm`, the engine in browsers (`AudioWorklet`) and in Node.

Planned: a Rust crate, an npm package and a C library (#32 steps 2, 6 and 7).
