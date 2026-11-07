# crates

The **engine core** and the project tooling, as Rust crates of one Cargo workspace.

| Crate | Contents |
|---|---|
| `engine-core` | Box contract, strong types, deterministic execution plan (`no_std`, no `unsafe`) |
| `xtask` | Project automation: `cargo xtask check` |

Planned: `engine-objects` (the boxes), `engine-testkit` (shared laws), `engine-hash`, `engine-cli` (roadmap ticket #1).
