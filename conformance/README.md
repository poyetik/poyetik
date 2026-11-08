# conformance

The **conformance corpus**: every target and every SDK must reproduce these render hashes exactly.

| Path | Contents |
|---|---|
| `cases/<name>.json` | Reference cases: a sample rate and a duration, from 1 to 2^29 (536,870,912) frames |
| `patches/<name>.json` | The patch of each case, in the patch format. Every runner (native, Node, browsers) gives the engine the bytes of this file unchanged, never a parsed and rewritten copy |
| `invalid/cases/<name>.json` | Case files every reader must reject (a duration of 0 frames, or above 2^29): the shared rejection corpus of the engine CLI and the oracle |
| `test-matrix.json` | The shared test matrix: the values each dimension of the environment is tested across, read by the Rust and TypeScript tests |
| `references.json` | The expected render hash of every case |
| `expectations.json` | What every case must sound like, independently of any hash: for each channel, the ideal signal. The largest allowed distance from it is derived by the oracle (output rounding, phase accumulation, evaluation), never declared |

- `cargo run -p engine-cli -- check conformance` renders every case natively and compares the hashes; `cargo xtask check` runs it.
- **`references.json` only changes through `cargo xtask bless`**, and the commit must explain why the hashes changed.
- `cargo run -p engine-cli -- render conformance/cases/<name>.json conformance/patches/<name>.json --wav <file.wav>` writes a case as a WAV file to listen to.
- `cargo run -p engine-cli -- export conformance target/conformance` writes the WAV file of every case; the independent oracle (`tooling/oracle`) then rereads them, recomputes their hashes and checks `expectations.json`. `cargo xtask check` runs both.
- Every case needs its patch, a reference hash and expectations, and every patch its case: the engine CLI and the oracle fail on anything missing.
- Size limits, checked while reading (never more than one byte past them): a case file holds at most 64 KiB, a patch file 16 MiB (the limit of the patch format), `references.json` 1 MiB.
