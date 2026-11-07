# poyetik

A visual audio patching environment (boxes connected by cables) running in the browser, with a Rust engine (WebAssembly on the web, native elsewhere). A finished patch exports to native apps and plugins, user interface included.

The core promise: **the engine produces exactly the same samples, bit for bit, on every target**, and a strict, public testing policy proves it.

> Status: early development. See the [roadmap](apps/roadmap/) for what is done and what comes next.

## Repository

A monorepo of separate parts (see [`doc/architecture.md`](doc/architecture.md)):

| Path | Contents |
|---|---|
| `spec/` | Patch format specification (to come) |
| `crates/` | The engine core (`engine-core`, `no_std`, no `unsafe`) and project automation (`xtask`) |
| `bindings/` | Developer SDKs around the core (to come) |
| `apps/` | Applications: the public roadmap today |
| `conformance/` | Reference cases and expected hashes (to come) |
| `doc/` | Architecture, dependency register and decisions log |
| `CLAUDE.md` | Project rules: language, proof, typing, testing, documentation, versioning |
| `CHANGELOG.md` | Notable changes, per version |

## Requirements

- [rustup](https://rustup.rs): the pinned toolchain (`rust-toolchain.toml`) installs itself; branch coverage also needs `nightly-2026-11-01` with `llvm-tools-preview`.
- `cargo install --locked cargo-llvm-cov cargo-mutants cargo-deny`
- Node.js 22 and pnpm 10, then `pnpm install` at the root and `pnpm --dir apps/roadmap exec playwright install`.

## Checking

```sh
cargo xtask check
```

Runs every quality check: formatting, lints, documentation, tests, portability builds, 100% coverage, mutation testing, supply chain, versions and changelog, then the roadmap checks. The pre-push hook and CI run exactly this command.

## Versioning

[Semantic Versioning](https://semver.org); every notable change is recorded in [`CHANGELOG.md`](CHANGELOG.md). Commit messages follow [Conventional Commits](https://www.conventionalcommits.org).
