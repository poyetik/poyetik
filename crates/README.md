# crates

The **engine core** and the project tooling, as Rust crates of one Cargo workspace.

| Crate | Contents |
|---|---|
| `engine-core` | Box contract, strong types, deterministic execution plan (`no_std`, no `unsafe`) |
| `engine-objects` | The boxes (`osc~`, `dac~`) and their registry (`no_std`, no `unsafe`) |
| `engine-testkit` | Test bench: the laws every registered box must respect |
| `xtask` | Project automation: `cargo xtask check` |

Planned: `engine-hash`, `engine-cli` (roadmap ticket #1).
