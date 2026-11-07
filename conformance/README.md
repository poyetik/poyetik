# conformance

The **conformance corpus**: every target and every SDK must reproduce these render hashes exactly.

| Path | Contents |
|---|---|
| `cases/*.json` | Reference cases: a patch, a sample rate and a duration in frames |
| `references.json` | The expected render hash of every case |

- `cargo run -p engine-cli -- check conformance` renders every case natively and compares the hashes; `cargo xtask check` runs it.
- **`references.json` only changes through `cargo xtask bless`**, and the commit must explain why the hashes changed.
- `cargo run -p engine-cli -- render conformance/cases/<case>.json --wav <file.wav>` writes a case as a WAV file to listen to.
