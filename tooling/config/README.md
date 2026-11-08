# config

`@poyetik/config`: the strict TypeScript policy of every package, in one place.

| Path | What |
|---|---|
| `tsconfig.base.json` | The strictest compiler settings, extended by every package |
| `eslint.ts` | `strict(directory, ignores, ...extra)`: the shared ESLint configuration (no casts, no `any`, no inline disabling, strict booleans, exhaustive switches, every item documented …) |
| `test/policy/` | One fixture per lint rule, each breaking it, plus `clean.ts` that nothing rejects |
| `test/compiler/` | One fixture per strict compiler setting, each breaking it |

The tests prove the policy: each fixture must be rejected by exactly the rule it breaks, as an error. A rule turned off or weakened fails them, and so does a rule the policy sets (`RULES`) or a JSDoc context it requires (`JSDOC_CONTEXTS`) that no fixture breaks.

```sh
pnpm check   # types, lint (the policy lints itself), the policy tests at 100% coverage, mutation testing
```
