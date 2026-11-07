# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `engine-core`: the box contract (`Object`, `ObjectSpec`), strong types (`SampleRate`, fixed 64-frame `Block`, `NodeId`) and the deterministic execution plan (validation, topological sort, cycle detection, allocation-free processing).
- `xtask`: `cargo xtask check`, the single quality gate run by the pre-push hook and CI.
- Public roadmap (`roadmap/`): kanban stored in `board.json`, driven by a CLI, rendered as a static page with collapsible steps.
- Project rules in `CLAUDE.md` and architecture in `doc/architecture.md`.
