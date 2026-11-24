# config

`@poyetik/config`: the strict TypeScript policy of every package, in one place.

| Path | What |
|---|---|
| `tsconfig.base.json` | The strictest compiler settings, extended by every package |
| `eslint.ts` | `strict(directory, ignores, ...extra)`: the shared ESLint configuration (no casts, no `any`, no inline disabling, strict booleans, exhaustive switches, every item documented …) |
| `one-line.ts` | `oneLine(text)`: untrusted text escaped and bounded for one line of a command-line tool, as `audio_patch::safe_message` does in Rust (tested on `spec/safe-message-vectors.json`); `firstUnsafe(text)` for the readers that refuse such text |
| `playwright.ts` | `freePort(variable, host)` and `inTurn(values, index)`: what the Playwright configurations of the packages compute (a free port per run, the test matrix spread over the browser projects), measured and mutated here |
| `gates.ts` | `uncited` and `unmatched`: the rules of `CLAUDE.md` the lint messages cite, and the reviewed patterns a test still matches, measured and mutated here |
| `test/policy/` | One fixture per lint rule, each breaking it, plus `clean.ts` that nothing rejects; `ts/` holds a module driver's code (`MODULE_DRIVERS`) |
| `test/compiler/` | One fixture per strict compiler setting, each breaking it |
| `test/linted.test.ts` | Every tracked TypeScript or JavaScript file of the repository is linted by its package's configuration, but for a reviewed list, each with what proves it instead |

The tests prove the policy: each fixture must be rejected by exactly the rule it breaks, as an error. A rule turned off or weakened fails them, and so does a rule the policy sets (`RULES`) or a JSDoc context it requires (`JSDOC_CONTEXTS`) that no fixture breaks.

```sh
pnpm check   # types, lint (the policy lints itself), the policy tests at 100% coverage
```
