# bindings

Thin **developer SDKs** around the libraries' cores. All the logic stays in the core; each binding only adapts it to one environment, and each one passes its library's conformance corpus, with the hashes of the native renderer.

- [`audio-wasm/`](audio-wasm/): `audio-wasm`, the engine in browsers (`AudioWorklet`) and in Node.
- [`web-page/`](web-page/): `web-page`, the page module (the product module a browser page loads): the visual library's C interface and the coordinating layer's, in one module (`spec/engine-design.md`, decision 33). Its own interface takes and returns integers only: first, which image the page shows at a frame of the audio clock.
- [`visual-wasm/`](visual-wasm/): `visual-wasm`, the visual library's own module, for those who use it alone, tested in Node. The product never loads two library modules side by side: it links both libraries, natively into one binary and in browsers into one module per thread holding what that thread runs: the page's is `web-page`'s, with the coordinating layer's crate, `coordinator` (`spec/engine-design.md`, decision 33).

Planned: a Rust crate, an npm package and a C library (#32 steps 2, 6 and 7).
