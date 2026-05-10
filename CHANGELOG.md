# Changelog

All notable changes to **sqldev** are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and from `v0.2.0` onward this file is generated automatically by
[release-please](https://github.com/googleapis/release-please) from
Conventional Commits.

## [Unreleased]

### Added
- Pre-release work towards M1.4 (typed JSON, CSV, NDJSON, aligned text
  table) and other M1 milestones. See open issues at
  https://github.com/saurabh500/sqldev/issues.

## [0.1.0] — 2026-05-10

The "first walking skeleton" pre-release. Validated end-to-end against
SQL Server 2025 + AdventureWorks2022.

### Added
- **M0 spikes** (kept in-tree as `spike-introspect/`, `spike-diff/`,
  `spike-explain/`):
  - `spike-introspect`: walks the system catalog into a v0.1 schema graph
    JSON document (6 schemas / 71 tables / 486 columns / 90 FKs / 101
    indexes / 6 UDDTs / 10 triggers on AdventureWorks2022).
  - `spike-diff`: round-trips the schema graph through generated DDL with
    8/8 reference migrations passing.
  - `spike-explain`: parses SQL Server XML query plans and detects 3
    anti-patterns end-to-end (clustered-index-scan-with-residual,
    key-lookup-on-non-covering-index, implicit-conversion-on-column).
- **Cargo workspace skeleton (M1.1)** with crates:
  - `sqldev-core`: shared schema-graph types and error model.
  - `sqldev-conn`: thin Tiberius wrapper, SQL auth.
  - `sqldev-introspect`: M0 catalog walk reused as a library.
  - `sqldev-cli`: the `sqldev` binary.
- **`sqldev introspect`**: emits the v0.1 schema graph JSON.
- **`sqldev query`**: one-shot T-SQL over `--sql` or stdin, with
  `--format text|json`.
- **`.sqldev.yml` config (M1.3)**:
  - New `sqldev-config` crate with typed model
    (`deny_unknown_fields`), upward `.sqldev.yml` discovery, and
    `${VAR}` / `${VAR:-fallback}` interpolation.
  - Top-level `--config <path>` and `--env <name>` global flags.
  - Connection-flag resolver: CLI > env block > `SQLDEV_*` > default.
  - `protected: true` env blocks reject `--trust-cert`.
  - New `sqldev config show [--all] [--json]` with password redaction.
- **CI workflow**: `cargo fmt --check`, `cargo clippy -D warnings`, and
  `cargo test --workspace --locked` on macOS, Linux, and Windows runners.
- **Release tooling**: release-please-driven GitHub Releases with
  pre-built tarballs for `x86_64-unknown-linux-gnu`, `x86_64-apple-darwin`,
  `aarch64-apple-darwin`, and `x86_64-pc-windows-msvc`.

### Notes
- v0.1.0 is **pre-release / preview**. The CLI surface, the schema-graph
  JSON shape, and the `.sqldev.yml` keys are all subject to change before
  v1.0. We will bump the schema-graph `version` field on every breaking
  change.

[Unreleased]: https://github.com/saurabh500/sqldev/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/saurabh500/sqldev/releases/tag/v0.1.0
