# Poyetik

A visual audio patching environment (boxes connected by cables) running in the browser, with a Rust engine (WebAssembly on the web, native elsewhere). A finished patch exports to native apps and plugins, user interface included.

The core promise: **the engine produces exactly the same samples, bit for bit, on every target**, and a strict, public testing policy proves it.

> Status: early development. See the [roadmap](apps/roadmap/) for what is done and what comes next.

## Repository

A monorepo of separate parts:

| Path | Contents |
|---|---|
| `spec/` | Patch format specification |
| `crates/` | The engine: `engine-core` (`no_std`, no `unsafe`), the boxes, the patch reader, the render hash, the command-line renderer, and project automation (`xtask`) |
| `bindings/` | Developer SDKs around the core: `wasm/`, the engine in browsers and Node |
| `apps/` | Applications: the playground and the public roadmap |
| `conformance/` | Reference cases and expected hashes, checked on every target |
| `tooling/` | Shared development tooling: the strict TypeScript policy (`@poyetik/config`) and the independent conformance oracle |
| `GLOSSARY.md` | The words of the project, and the name each takes in the patch format and the code |
| `CLAUDE.md` | Project rules: language, proof, typing, testing, documentation, versioning |
| `CHANGELOG.md` | Notable changes, per version |

## Requirements

macOS or Linux (on Windows, use WSL2). `cargo xtask check` checks every tool first and says how to install what is missing; in short:

- [rustup](https://rustup.rs): the pinned toolchain (`rust-toolchain.toml`) installs itself. The checks also need `nightly-2026-11-01` with `llvm-tools-preview`, `miri` and `rust-src` (branch coverage and Miri only).
- `cargo install --locked cargo-llvm-cov@0.9.1 cargo-mutants@27.1.0 cargo-deny@0.20.2` (the exact versions the checks require).
- Node.js, exactly the release in `.node-version` (the one CI installs; the preflight refuses any other, though the packages' `engines` accept 22.18 or later) and pnpm 10 (`corepack enable`), then `pnpm install` at the root, and `pnpm exec playwright install` in each package with end-to-end tests (`bindings/wasm`, `apps/playground`, `apps/roadmap`).

## Checking

```sh
cargo xtask check          # every check, in order; mutation testing of the changes only (see below)
cargo xtask check --full   # every check and every mutant
cargo xtask list           # what they are (`list --full`: those of `check --full`)
cargo xtask mutants <target>  # every mutant of a crate, a Rust file, a TypeScript package or one of its sources
```

The pre-push hook runs `cargo xtask pre-push`, which refuses uncommitted changes and a pushed commit other than the one checked out, then runs `cargo xtask check` (`--full` when a tag is pushed); CI runs `cargo xtask check --full`. The default check mutation tests only what changed: Rust since the merge base with the branch's upstream (else `origin/main`, else `main`), TypeScript since the last local Stryker run (its incremental file), so a green default check does not prove a green full check; `CLAUDE.md` (CI) says what each proves and when the full check is required.

## License

Not decided yet.

## Versioning

[Semantic Versioning](https://semver.org); every notable change is recorded in [`CHANGELOG.md`](CHANGELOG.md). Commit messages follow [Conventional Commits](https://www.conventionalcommits.org).
