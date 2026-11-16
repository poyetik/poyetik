# oracle

The independent conformance oracle. For every case of `conformance/` and every render format (`binary64`, `binary64-as-binary32`, `binary32`), it rereads the WAV file the native engine wrote (`target/conformance/<case>.<format>.wav`, from `audio-cli export`), recomputes its render hash (`render-hash/2`), checks the signal against its mathematical definition within the error bound of its format and measures its quality against thresholds derived from its expectation and its format (`spec/measurements.md`), sharing no code with the engine: a defect common to the engine and its own checks cannot hide. It also reads the visual conformance cases (`src/visual-inputs.ts`), as `visual-cli` reads them, for the tests of the visual library's WebAssembly module.

| Path | What |
|---|---|
| `src/format.ts` | The render formats: the precision of each and the width of its samples |
| `src/wav.ts` | Reads a 32- or 64-bit float WAV file: format, rate, channels, sample width, samples, every field checked |
| `src/hash.ts` | The render hash, written again from its specification (`spec/render-hash.md`) |
| `src/signal.ts` | The expected signals and their error bounds, by render format |
| `src/measure.ts` | The measurements (spectrum, THD+N, THD, SNR, SFDR, aliasing level) and the bounds of a sine, written again from `spec/measurements.md` |
| `src/limited.ts` | Judges a channel rendered through the true-peak limiter (`limited` expectations), with every detector, from its own interpolator held apart from the engine (BS.1770's table as the standard prints it, the windowed ones computed from their definition): silence over the latency, then a true peak and a THD+N within the bounds it derives |
| `src/quality.ts` | Measures a channel that must hold a sine and holds every level to its derived bound |
| `src/inputs.ts` | Parses the cases (accepting and refusing the same files as `audio-cli`), the references (a hash per render format) and the expectations |
| `src/visual-inputs.ts` | Parses the visual cases of `conformance/visual` (accepting and refusing the same files as `visual-cli`, the first image's index exact past 2⁵³) |
| `src/files.ts` | Reads a regular file with a size limit, never more than one byte past it, and lists a directory; a failure is named by its error code |
| `src/oracle.ts` | The verdict on the whole corpus, one line per case and render format; an empty corpus fails |
| `src/cli.ts`, `src/bin.ts` | The command-line tool (exit 0, 1 or 2; one `error:` line on failure or misuse; `help` prints the usage) |

```sh
pnpm oracle   # judge the corpus exported to ../../target/conformance
pnpm check    # types, lint, unit tests at 100% coverage, then the oracle itself
node src/bin.ts help   # the usage
```
