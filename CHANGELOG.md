# Changelog

All notable changes to **sqldev** are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and from `v0.2.0` onward this file is generated automatically by
[release-please](https://github.com/googleapis/release-please) from
Conventional Commits.

## [0.1.1](https://github.com/saurabh500/sqldev/compare/v0.1.0...v0.1.1) (2026-05-11)


### Features

* add sqldev-showplan crate for SHOWPLAN_XML parsing ([#42](https://github.com/saurabh500/sqldev/issues/42)) ([d541bbf](https://github.com/saurabh500/sqldev/commit/d541bbf785f72ac0a67952dab61d9a4f69d3df84))
* **diff:** M2.2 productionize schema-graph differ + `sqldev diff` CLI ([#35](https://github.com/saurabh500/sqldev/issues/35)) ([e42c526](https://github.com/saurabh500/sqldev/commit/e42c526854914519e8d2402e60c9a43fd4c30ea4))
* **diff:** M2.3 `sqldev diff --output` → numbered migration file ([#40](https://github.com/saurabh500/sqldev/issues/40)) ([9d93f64](https://github.com/saurabh500/sqldev/commit/9d93f641067c43e774ca1b5204e8fdb2f8f95509))
* **explain:** M2.1 productionize showplan analyzer (15 rules) ([#34](https://github.com/saurabh500/sqldev/issues/34)) ([55d1bef](https://github.com/saurabh500/sqldev/commit/55d1befb3c4a6557a5f8c1b22c40fba006b008d2))
* **m1.6:** `sqldev init` + live SQL Server 2025 CI ([#28](https://github.com/saurabh500/sqldev/issues/28)) ([5871746](https://github.com/saurabh500/sqldev/commit/58717466cf4189b50c3c704c022eed35a374ee8c))
* **migrate:** M1.7 sqldev migrate up|down|status|create ([#30](https://github.com/saurabh500/sqldev/issues/30)) ([6086228](https://github.com/saurabh500/sqldev/commit/6086228aa50b94b06204866e47b740d139da53f2))
* **query:** M1.5 sqldev query REPL ([#31](https://github.com/saurabh500/sqldev/issues/31)) ([bcb6886](https://github.com/saurabh500/sqldev/commit/bcb6886b96b51ae50aa5421660e0afbaa87f76d7))
* **query:** typed values + table / json / ndjson / csv output formats ([#25](https://github.com/saurabh500/sqldev/issues/25)) ([5d1eae1](https://github.com/saurabh500/sqldev/commit/5d1eae151a2d384dd78e08e94738248111b3dd0c))
* **seed:** M2.5 sqldev seed — type-aware fake data from YAML ([#43](https://github.com/saurabh500/sqldev/issues/43)) ([283c6e6](https://github.com/saurabh500/sqldev/commit/283c6e6d7e11231aaa7fd381c58dbe746fb1e7d9))
* **snapshot:** M2.4 sqldev snapshot save + diff --source/--target ([#41](https://github.com/saurabh500/sqldev/issues/41)) ([3d82007](https://github.com/saurabh500/sqldev/commit/3d820077145bd60ce43ae9d69ecdae7689608519)), closes [#18](https://github.com/saurabh500/sqldev/issues/18)
* **telemetry:** M1.8 opt-in telemetry stub + crash hook ([#32](https://github.com/saurabh500/sqldev/issues/32)) ([1573bf4](https://github.com/saurabh500/sqldev/commit/1573bf4ea8e3782ad656c376e0072cbf09df0b5d))


### Bug Fixes

* **diff:** plumb owning schema for UDDT columns ([#47](https://github.com/saurabh500/sqldev/issues/47)) ([f7ae672](https://github.com/saurabh500/sqldev/commit/f7ae6720cfe7c1e35bedeaaf5f7dde87d3b83cf9)), closes [#37](https://github.com/saurabh500/sqldev/issues/37)
* **release:** switch release-please to simple strategy for Cargo workspace ([#26](https://github.com/saurabh500/sqldev/issues/26)) ([8f4fb88](https://github.com/saurabh500/sqldev/commit/8f4fb885f65222b6a7c32e9cb9095449cc8abd76))


### Documentation

* README, getting-started guide, and Clipchamp demo script ([#24](https://github.com/saurabh500/sqldev/issues/24)) ([a8d3077](https://github.com/saurabh500/sqldev/commit/a8d3077fdda2104e7a7f1a23e2018cee6ca25c88))

## [Unreleased]

### Added
- **`sqldev query` typed output formats:**
  - `--format table` — aligned, human-readable table with
    right-aligned numerics and a visible `NULL` marker.
  - `--format json` is now **typed**: numbers stay numbers, booleans
    stay booleans, NULLs become JSON `null`.
  - `--format ndjson` — newline-delimited typed JSON, one row per line.
  - `--format csv` — RFC 4180 via the `csv` crate.
  - Internal `output::CellValue` enum is open-ended so decimal,
    datetime, UUID, and binary variants plug in later without changing
    formatter signatures.
- `README.md` with status, install, quickstart, config reference, and
  repo layout.
- `docs/getting-started.md` — 10-minute walkthrough that takes you from
  a fresh machine to running queries against AdventureWorks2022 in
  Docker.
- `docs/demo-script.md` and `scripts/demo.sh` — a Clipchamp-ready
  recording plan for a 2-minute "what is sqldev?" demo, plus a runnable
  script that drives the same commands.

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
