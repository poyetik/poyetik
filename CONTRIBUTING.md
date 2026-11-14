# Contributing

Thank you for your interest in Poyetik. The project is in early development, and its rules are strict on purpose: read [`CLAUDE.md`](CLAUDE.md), which states them (testing, typing, documentation, commits), before you start.

## Before your first pull request

Contributions are accepted under the [Contributor License Agreement](CLA.md). It lets the project keep its licenses, and change them for later versions, while you keep the copyright of your work. On your first pull request, write the comment the agreement gives; a pull request is not merged before it.

## How a change is accepted

- One logical change per commit, in [Conventional Commits](https://www.conventionalcommits.org), in English; every notable change adds an entry to [`CHANGELOG.md`](CHANGELOG.md).
- `cargo xtask check` passes: run `pnpm install` once at the root, which enables the git hooks that run it before every push.
- Every behavior has its tests, and no test is weakened to make a change pass.

## Licenses

A contribution to `apps/` is under the license of the apps, every other contribution under the open license of the rest of the repository: see [License](README.md#license).
