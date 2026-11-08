# poyetik

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
| `tooling/` | Shared development configuration (`@poyetik/config`) |
| `CLAUDE.md` | Project rules: language, proof, typing, testing, documentation, versioning |
| `CHANGELOG.md` | Notable changes, per version |

## Requirements

macOS or Linux (on Windows, use WSL2). `cargo xtask check` checks every tool first and says how to install what is missing; in short:

- [rustup](https://rustup.rs): the pinned toolchain (`rust-toolchain.toml`) installs itself. The checks also need `nightly-2026-11-01` with `llvm-tools-preview`, `miri` and `rust-src` (branch coverage and Miri only).
- `cargo install --locked cargo-llvm-cov cargo-mutants cargo-deny`
- Node.js 22.18 or later and pnpm 10 (`corepack enable`), then `pnpm install` at the root, and `pnpm exec playwright install` in each package with end-to-end tests (`bindings/wasm`, `apps/playground`, `apps/roadmap`).

## Checking

```sh
cargo xtask check   # every check, in order
cargo xtask list    # what they are
```

The pre-push hook and CI run exactly `cargo xtask check`.

## License

Not decided yet.

## Versioning

[Semantic Versioning](https://semver.org); every notable change is recorded in [`CHANGELOG.md`](CHANGELOG.md). Commit messages follow [Conventional Commits](https://www.conventionalcommits.org).
