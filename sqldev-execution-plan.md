# `sqldev` Execution Plan

Ordered for maximum learning per unit of work — de-risk the hard parts early, ship usable value at every milestone, defer ecosystem work until the core is proven.

---

## M0 — De-risk (the gate)

**Goal:** Prove the two hardest features work on a real schema before committing to the rest. If this fails, scope shrinks.

1. **Spike: TDS connectivity + schema introspection.** Connect via `mssql-tiberius-bridge`, dump `sys.tables` / `sys.columns` / `sys.foreign_keys` / `sys.indexes` into the v0.1 schema graph JSON. No CLI yet — just a binary that prints the graph.
2. **Spike: `diff` on AdventureWorks.** Take AdventureWorks, hand-write 5–10 evolutionary migrations, and verify `sqldev diff` produces correct SQL for tables, columns, PK/FK, indexes, and views. *This is the gate.* If correctness on the common path isn't achievable in this spike, fall back to wrapping DacFx from day one.
3. **Spike: `explain` plan parsing.** Run a query with `SET STATISTICS XML ON`, parse the showplan XML, identify the top 3 anti-patterns (table scan w/ no index, key lookup, implicit conversion). Produce one human-readable suggestion. Proves the headline feature is feasible.

**Exit criteria:** all three spikes work end-to-end in throwaway binaries. Schema graph JSON Schema is drafted and reviewed.

---

## M1 — Foundation (first usable release: `v0.1`)

**Goal:** A CLI a developer would actually install to replace `sqlcmd` for daily work.

1. **Project skeleton.** Cargo workspace, `clap`-based CLI, structured logging, error model, CI (fmt + clippy + test on macOS/Linux/Windows), single-binary releases via GitHub Actions.
2. **Connection layer.** `mssql-tiberius-bridge` wrapper with connection pooling. Auth modes in priority order: SQL auth, Windows integrated, Entra interactive, Entra managed identity, Entra service principal. Connection strings *and* structured `.sqldev.yml` env blocks both supported.
3. **`.sqldev.yml` config.** Multi-environment, env-var interpolation, `protected: true` flag enforcement, schema validation with helpful errors.
4. **`sqldev query` — one-shot mode first.** `--format table|json|csv|ndjson`. Pipe-friendly. Exit codes correct. This unlocks scripting use cases immediately.
5. **`sqldev query` — REPL mode.** `rustyline`-based, history file, `\d`/`\dt`/`\di` meta-commands, syntax highlighting (`syntect`), tab-completion (start with table/column names from a cached schema snapshot).
6. **`sqldev init`.** Reverse-engineer current schema → `0001_baseline.sql`, write `.sqldev.yml`, create `migrations/`, `seeds/`, `models/` directories.
7. **`sqldev migrate up|down|status|create`.** `__sqldev_migrations` tracking table, transactional per-migration apply, `--to <version>`, `--steps N`, `--dry-run`, `--confirm` enforcement on protected envs.
8. **Telemetry opt-in + crash reporting.** Anonymous usage counts so we know which commands matter.

**Ship `v0.1`.** Blog post + Hacker News launch. The pitch: *"`sqlcmd` for humans, with migrations."*

---

## M2 — Intelligence (the differentiating release: `v0.2`)

**Goal:** The features nobody else has. This is where the project earns its name.

1. **`sqldev explain`.** Productionize the M0 spike. Expand the rule catalog to ~15 anti-patterns (table scans, key lookups, implicit conversions, hash spills to tempdb, parameter sniffing tells, missing-index hints, parallelism warnings, RID lookups, sort warnings). Each rule produces a fix suggestion with estimated cost delta. **This is the headline feature — invest disproportionately here.**
2. **`sqldev diff` — common path.** Tables, columns (incl. computed + defaults), PK/FK, unique/check constraints, indexes (incl. filtered), views, schemas, sequences, stored procedures, functions, triggers. Detect-and-warn on out-of-scope objects.
3. **`sqldev diff --output` → migration file.** Generates a numbered migration with up + down sections. Round-trip test: apply → diff → expect empty.
4. **Snapshot mode.** `sqldev snapshot save dev > schema.json` and `sqldev diff --source schema.json --target prod`. Enables offline diffing in CI.
5. **`sqldev seed`.** Type-aware generators (faker integration), YAML seed files, `--related` with topological sort + cycle detection, composite/unique key handling, deterministic mode (`--seed N`) for reproducible test data.
6. **Built-in C# codegen.** Most-requested target for SQL Server shops. EF Core models + Dapper POCOs as two styles. Lives in core, not in a plugin — proves the schema graph is rich enough before externalizing it.

**Ship `v0.2`.** This is the release that gets shared. Demo video should lead with `sqldev explain`.

---

## M3 — Workflow (the daily-driver release: `v0.3`)

**Goal:** Close the inner-dev-loop story so `sqldev` is open in a terminal tab all day.

1. **`sqldev watch`.** Filesystem watcher, debounce, applies sprocs/views/functions on save with inline pass/fail. Smart enough to detect destructive changes and require confirmation.
2. **`sqldev test` — built-in runner.** YAML test files, `BEGIN TRAN`/`ROLLBACK` isolation by default, `snapshot-restore` and `none` opt-ins, parallel execution, JUnit XML output for CI.
3. **`sqldev test --runner tsqlt`.** Discover and execute existing tSQLt test classes; report results in the same format. Wins over teams that already invested in tSQLt.
4. **Better diagnostics.** Every error includes a `--explain-error` hint. Common SQL Server error codes get human-readable translations.
5. **Performance pass.** Cold-start budget < 50ms. Schema introspection cached on disk with mtime-based invalidation.

**Ship `v0.3`.**

---

## M4 — Ecosystem (the leverage release: `v1.0`)

**Goal:** Let other people extend `sqldev` without touching core.

1. **Plugin protocol.** Finalize stdin/stdout JSON contract, freeze schema graph at `v1.0`, publish JSON Schema, write the spec doc.
2. **Plugin SDKs.** Reference SDKs in Rust, Go, TypeScript, Python — each with input parsing helpers, output helpers, and a test harness.
3. **Plugin lifecycle commands.** `sqldev plugin search|install|list|update|create|test`.
4. **Registry.** `github.com/sqldev-plugins/registry` repo, YAML manifest per plugin, per-platform binary URLs + SHA256 pins.
5. **Supply-chain hardening from day one.** Sigstore/cosign signature verification, `--unverified` opt-in for unsigned plugins, hook timeouts (default 30s), fail-closed on protected envs, `--skip-hooks` is logged.
6. **First-party plugins shipped alongside `v1.0`.**
   - `sqldev-codegen-typescript` (Zod + plain interfaces)
   - `sqldev-codegen-python` (Pydantic + SQLAlchemy)
   - `sqldev-codegen-go` (structs + `db:` tags)
   - `sqldev-output-xlsx`
   - `sqldev-hook-lint` (using a curated rule set)
   - `sqldev-migrate-dacpac` (the diff escape hatch)
7. **Migration interop plugins.** `sqldev-migrate-ef`, `sqldev-migrate-flyway` — important for adoption in existing codebases.

**Ship `v1.0`.** Stable plugin contract, semver guarantees, LTS branch.

---

## M5 — Beyond `v1.0` (driven by usage data)

Prioritize from real telemetry and issues. Likely candidates:

- Azure SQL Hyperscale specifics; serverless tier connection-warmup awareness.
- Schema graph extensions for Always Encrypted column metadata, row-level security, temporal tables.
- Query Store integration for `sqldev explain --history` (compare plans over time).
- VS Code extension that drives the CLI (not a re-implementation).
- `sqldev fmt` — opinionated SQL formatter.
- Multi-DB project mode (cautiously — see non-goals).

---

## Cross-Cutting Workstreams

These run in parallel with milestones, not after:

| Stream | Owner cadence |
|---|---|
| **Docs site** (`sqldev.dev`) | Every release. Tutorial → reference → recipes → plugin authoring. |
| **Test infra** | Each milestone needs integration tests against real SQL Server (Linux container) and Azure SQL (CI nightly). |
| **Telemetry review** | Monthly. Drives M5 prioritization. |
| **Security** | Threat model before M4 (plugin execution is the new attack surface). |
| **Community** | Discord + GitHub Discussions opened at M1 launch, not after. |
| **Distribution** | See Distribution Plan below. Channels phased over M1–M4. |

---

## Distribution Plan

Distribution scales with the release. Every milestone unlocks new channels — we don't try to be on every package manager from day one.

### M0 — De-risk
- No public artifacts. Internal builds only via `cargo build --release`.

### M1 — `v0.1` launch (Foundation)
**Goal:** make it trivially installable on a developer laptop.

1. **GitHub Releases** with pre-built tarballs for the seven primary target triples (macOS x86_64/arm64, Linux gnu x86_64/arm64, Linux musl x86_64/arm64, Windows x86_64).
2. **`install.sh` / `install.ps1`** one-liners hosted at `sqldev.dev`, with checksum + cosign signature verification.
3. **Homebrew tap** (`sqldev/tap`) — primary macOS/Linux channel.
4. **WinGet manifest** submitted to `microsoft/winget-pkgs` — primary Windows channel.
5. **`cargo install sqldev`** — works automatically once published to crates.io.
6. **Multi-arch container image** at `ghcr.io/sqldev/sqldev` for CI users.
7. **Signing infrastructure**: cosign keyless via GitHub OIDC, SHA256SUMS published, SBOM (CycloneDX) generated.

### M2 — `v0.2` (Intelligence)
1. **APT repository** at `apt.sqldev.dev` (signed `.deb` for Debian/Ubuntu, amd64 + arm64).
2. **RPM repository** at `rpm.sqldev.dev` (signed `.rpm` for Fedora/RHEL/Rocky).
3. **Scoop bucket** for Windows power users.
4. **macOS notarization** via Apple Developer ID — removes Gatekeeper warning.
5. **`sqldev self update`** command — detects package-manager installs and defers; otherwise atomic in-place replace with signature verification.

### M3 — `v0.3` (Workflow)
1. **Chocolatey package** for enterprise Windows shops.
2. **AUR PKGBUILD** (we maintain source-of-truth; community owns submission).
3. **Snap package** for Ubuntu LTS users (strict confinement).
4. **Windows EV code signing** — required once SmartScreen reputation starts mattering at scale.
5. **`mise` / `asdf` plugin** registered.

### M4 — `v1.0` (Ecosystem)
1. **Promote Homebrew tap → homebrew-core** (requires ~75 stars + maturity).
2. **SLSA Level 3 provenance attestations** on all release artifacts.
3. **LTS branch policy** activated: 12-month security backports for each `1.x` minor.
4. **Plugin distribution** piggy-backs on the same CI: each first-party plugin gets the same release matrix and signing pipeline.

### Cross-cutting principles
- **Single binary, no runtime deps.** Rules out anything that needs Python/Node/JVM at runtime.
- **No telemetry on install.** Ever.
- **Reproducible builds** as a goal post-`v1.0` (best effort during 0.x).
- **GitHub Releases is always the source of truth.** Every other channel is a mirror that may lag by hours; users in a hurry can always grab the tarball directly.

---

## Sequencing Rationale

- **M0 first because the project lives or dies on `diff`.** Every other feature is "nice CLI ergonomics over `sqlcmd`" — easy to copy. `diff` correctness is the moat.
- **`query` ships before `migrate`** in M1 because it's the lowest-friction first install. People try a tool before they trust it with their schema.
- **`explain` lives in M2, not M1**, because it needs the schema graph cache from M1 to suggest indexes intelligently.
- **Codegen lives in core for C# (M2) before going plugin-only (M4).** Forces the schema graph to be rich enough *before* the contract gets frozen for external plugins.
- **Plugin architecture is M4, not M2.** Premature plugin contracts are how projects ossify. Build the things, then extract the contract.
- **`test` is M3, not M2.** It depends on `seed` (M2) for fixture setup and is the feature most likely to creep in scope; it deserves a dedicated milestone.
