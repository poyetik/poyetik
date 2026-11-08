# oracle

The independent conformance oracle. For every case of `conformance/`, it rereads the WAV file the native engine wrote (`target/conformance/`, from `engine-cli export`), recomputes its render hash and checks the signal against its mathematical definition, sharing no code with the engine: a defect common to the engine and its own checks cannot hide.

| Path | What |
|---|---|
| `src/wav.ts` | Reads a WAV file: format, rate, channels, samples, every field checked |
| `src/hash.ts` | The render hash, written again from its specification (`spec/render-hash.md`) |
| `src/signal.ts` | The expected signals and their error bounds |
| `src/inputs.ts` | Parses the cases, the references and the expectations |
| `src/oracle.ts` | The verdict on the whole corpus; an empty corpus fails |
| `src/cli.ts`, `src/bin.ts` | The command-line tool (exit 0, 1 or 2; one `error:` line on failure) |

```sh
pnpm oracle   # judge the corpus exported to ../../target/conformance
pnpm check    # types, lint, unit tests at 100% coverage, mutation testing, then the oracle itself
```
