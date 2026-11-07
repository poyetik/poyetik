# tooling

Shared development tooling of the monorepo.

| Package | Contents |
|---|---|
| `config/` (`@poyetik/config`) | The strict TypeScript settings (`tsconfig.base.json`) and ESLint configuration (`eslint`) every TypeScript package uses |
| `oracle/` (`@poyetik/oracle`) | The independent conformance oracle: rereads the WAV file the native engine wrote for every case, recomputes its hash and checks the signal against its mathematical definition, with no code shared with the engine |
