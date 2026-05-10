# `sqldev` — A Developer-First SQL Server CLI

## Vision

A single, cohesive CLI tool that gives SQL Server *developers* (not DBAs) the workflow they deserve — schema-as-code, smart querying, human-readable query plans, test data generation, codegen, and live reload. Git-friendly, pipe-friendly, zero GUI required.

## The Problem

| Tool | Pain |
|------|------|
| `sqlcmd` | Bare-bones, no autocomplete, no schema awareness |
| SSMS | Windows-only GUI monolith, can't use in CI or over SSH |
| Azure Data Studio | Electron bloat, slow, still GUI-dependent |
| Redgate tools | Expensive, proprietary, GUI-first |

None of these tools think like a developer. They think like a DBA or a query runner. Developers need migrations, codegen, diffing, testing, and fast feedback loops — not a 2GB IDE to run a SELECT.

## Headline Demo

The one-liner that sells the project:

```bash
$ sqldev explain "SELECT * FROM Orders WHERE OrderDate > '2024-01-01'"

⚠ Table Scan on [Orders] (2.1M rows)
  → No index on OrderDate. Suggested fix:
      CREATE INDEX IX_Orders_OrderDate ON Orders(OrderDate);
  → Estimated cost would drop from 12.4 → 0.3

⏱ 842ms · 28,491 logical reads · 3,201 physical reads
```

No GUI, no plan-XML squinting, no DBA pager. That's the wedge.

## Core Commands

### `sqldev init`
Scaffolds a project folder with versioned `.sql` migration files. Connects to an existing database and reverse-engineers the current schema into a baseline migration.

```bash
sqldev init --connection "Server=localhost;User=SA;Password=..."
# Creates:
#   .sqldev.yml
#   migrations/
#   └── 0001_baseline.sql
#   seeds/
#   models/
```

### `sqldev migrate up|down|status`
Applies or rolls back migrations. Tracks state in a `__sqldev_migrations` table.

```bash
sqldev migrate up                    # apply all pending
sqldev migrate up --to 0005          # apply up to 0005
sqldev migrate down --steps 1        # rollback last migration
sqldev migrate status                # show pending/applied
sqldev migrate create add_orders     # scaffold new migration file
```

### `sqldev diff`
Compares two sources (connections, migration folders, or snapshots) and generates migration SQL.

```bash
sqldev diff --source dev --target staging          # two connections
sqldev diff --source ./migrations --target prod    # local files vs live DB
sqldev diff --source dev --target staging --output migrations/0006_sync.sql
```

**Scope (Phase 2):** tables, columns, primary/foreign keys, unique constraints, check constraints, indexes (including filtered), views, stored procedures, functions, triggers, schemas, and sequences. Computed columns and default constraints are supported.

**Out of scope for Phase 2** (tracked as future work): temporal tables, memory-optimized tables, partition schemes/functions, CLR types, FILESTREAM, Always Encrypted column metadata, Service Broker objects, full-text indexes, and replication artifacts. For these, `sqldev diff` will *detect and warn* rather than emit incorrect SQL.

**Implementation note:** for the long tail of edge cases, `sqldev diff` may delegate to `DacFx`/`sqlpackage` under the hood (via an optional plugin) rather than reimplement two decades of Redgate/SSDT learnings. The core ships with a hand-rolled differ for the common path; the dacpac plugin is the escape hatch.

### `sqldev query`
Smart interactive REPL with autocomplete, syntax highlighting, and multiple output formats.

```bash
sqldev query                                         # interactive REPL
sqldev query "SELECT * FROM Users" --format json     # one-shot, JSON output
sqldev query "SELECT * FROM Users" --format csv | xsv select Name,Email
sqldev query -f my_report.sql --format table         # run from file
```

REPL features:
- Tab-completion on table names, column names, schemas, sproc names
- Syntax highlighting
- Query history (up arrow, Ctrl+R search)
- `\d Users` to describe a table
- `\dt` to list tables
- `\di` to list indexes
- Multi-line editing

### `sqldev explain`
Runs a query and translates the execution plan into human-readable insights.

```bash
sqldev explain "SELECT * FROM Orders WHERE OrderDate > '2024-01-01'"
```

Output:
```
⚠ Table Scan on [Orders] (2.1M rows)
  → No index on OrderDate. Consider:
    CREATE INDEX IX_Orders_OrderDate ON Orders(OrderDate);

⏱ Estimated cost: 12.4 | Actual time: 842ms
📊 Logical reads: 28,491 | Physical reads: 3,201

Suggested fix applied would reduce estimated cost to 0.3
```

### `sqldev seed`
Generate realistic test data based on column types and constraints.

```bash
sqldev seed --table Users --rows 1000              # auto-detect types
sqldev seed --file seeds/users.yml                 # from YAML definition
sqldev seed --table Orders --rows 5000 --related   # respect FK relationships
```

**`--related` semantics.** The seeder builds a dependency graph from foreign keys and inserts in topological order. Self-referential FKs are handled by inserting nullable parents first, then back-filling. Cycles across tables are reported and require an explicit `--break-cycle <fk>` flag. Composite keys and multi-column unique constraints are honored via per-table generators that maintain a uniqueness set in memory; for very large row counts, this falls back to a deterministic hash-based generator.

Seed file example:
```yaml
# seeds/users.yml
table: Users
rows: 500
columns:
  Name: { faker: name }
  Email: { faker: email, unique: true }
  CreatedAt: { faker: date_recent, days: 90 }
  Role: { enum: [admin, user, viewer], weights: [5, 80, 15] }
```

### `sqldev codegen`
Generate application models from database schema.

```bash
sqldev codegen --lang csharp --namespace MyApp.Models --output ./models/
sqldev codegen --lang typescript --style zod
sqldev codegen --lang python --style pydantic
```

### `sqldev watch`
Live reload for stored procedures, views, and functions during development.

```bash
sqldev watch ./sql/                  # watch folder, apply on save
sqldev watch ./sql/ --filter "*.sproc.sql"
```

On file save → applies to local SQL Server → shows success/error inline. Like `nodemon` for SQL.

### `sqldev test`
Unit test your SQL with isolation between cases.

```bash
sqldev test                          # run all tests
sqldev test --filter "order*"        # filter by name
sqldev test --verbose                # show query output
```

**Isolation strategy.** Each test runs inside `BEGIN TRAN ... ROLLBACK` by default. This works for the common case (DML, proc calls that don't manage their own transactions) but has known limits: DDL inside `tempdb`, distributed transactions, procs that `COMMIT` internally, and anything touching `SAVEPOINT` semantics may leak state. For these, tests can opt into `isolation: snapshot-restore` (slower, uses a per-test database snapshot) or `isolation: none` (developer manages cleanup).

**tSQLt interop.** `sqldev test` is not trying to replace [tSQLt](https://tsqlt.org/). Teams that already have tSQLt suites can run them via `sqldev test --runner tsqlt`; `sqldev` handles discovery, parallelism, and reporting while tSQLt handles fakes/mocks/spies.

Test file example:
```yaml
# tests/test_orders.yml
- name: "inserting an order calculates total"
  setup: |
    INSERT INTO Products (Id, Price) VALUES (1, 29.99);
  run: |
    EXEC CreateOrder @UserId=1, @ProductId=1, @Qty=3;
  assert:
    - query: "SELECT Total FROM Orders WHERE UserId = 1"
      equals: 89.97

- name: "cannot insert duplicate email"
  run: |
    INSERT INTO Users (Email) VALUES ('dupe@test.com');
    INSERT INTO Users (Email) VALUES ('dupe@test.com');
  expect_error: "UNIQUE constraint"
```

## Plugin Architecture

### Design Philosophy

`sqldev` does two things: *talks to SQL Server* (fixed — TDS protocol) and *transforms what it gets back* (infinitely varied). The core owns the first part. Plugins own the second.

The core should be fully useful with zero plugins. Plugins extend, they don't enable.

### Plugin Contract

Plugins are standalone executables that speak JSON over stdin/stdout:

```
┌──────────┐     JSON stdin      ┌──────────────┐
│  sqldev   │ ──────────────────▶ │    plugin     │
│  core     │ ◀────────────────── │  (any lang)   │
└──────────┘     JSON stdout      └──────────────┘
```

This means plugins can be written in any language — Go, Rust, Python, Node, even bash. No shared library ABI headaches. Same model as Git, Docker, and kubectl.

### Plugin Categories

#### Codegen Plugins (`sqldev-codegen-*`)

Each receives a schema graph (tables, columns, types, relationships, constraints) as structured JSON and emits source files.

| Plugin | Output |
|--------|--------|
| `sqldev-codegen-csharp` | EF Core models, Dapper POCOs, raw ADO.NET |
| `sqldev-codegen-typescript` | Zod schemas, plain interfaces, Prisma-style types |
| `sqldev-codegen-python` | SQLAlchemy models, Pydantic schemas |
| `sqldev-codegen-go` | Structs with `db:` tags |
| `sqldev-codegen-openapi` | OpenAPI schema from table definitions |

#### Migration Plugins (`sqldev-migrate-*`)

Interop with existing migration ecosystems:

| Plugin | Purpose |
|--------|---------|
| `sqldev-migrate-dacpac` | Read/write SSDT dacpac format |
| `sqldev-migrate-ef` | EF Core migration compatibility |
| `sqldev-migrate-flyway` | Flyway naming conventions |

#### Output Plugins (`sqldev-output-*`)

Control how query results render:

| Plugin | Format |
|--------|--------|
| `sqldev-output-table` | Pretty terminal tables (built-in) |
| `sqldev-output-json` | JSON / NDJSON (built-in) |
| `sqldev-output-csv` | CSV / TSV (built-in) |
| `sqldev-output-xlsx` | Excel export |
| `sqldev-output-parquet` | Parquet for data pipelines |
| `sqldev-output-html` | Styled HTML tables |

#### Hook Plugins (`sqldev-hook-*`)

Lifecycle hooks that fire on events:

| Plugin | Trigger |
|--------|---------|
| `sqldev-hook-lint` | Pre-migrate: lint SQL for anti-patterns |
| `sqldev-hook-notify` | Post-migrate: notify Slack/Teams |
| `sqldev-hook-audit` | Post-migrate: log who/what/where |
| `sqldev-hook-backup` | Pre-migrate: snapshot before destructive changes |

### Plugin Configuration

```yaml
# .sqldev.yml
plugins:
  codegen:
    - name: csharp
      bin: sqldev-codegen-csharp
      config:
        namespace: MyApp.Models
        style: ef-core
        nullable: true

  hooks:
    pre-migrate:
      - name: lint
        bin: sqldev-hook-lint
        config:
          rules: [no-select-star, require-pk, no-implicit-conversions]
      - name: backup
        bin: sqldev-hook-backup
        config:
          target: s3://my-backups/

    post-migrate:
      - name: notify
        bin: sqldev-hook-notify
        config:
          slack_webhook: ${SLACK_WEBHOOK_URL}
```

### Plugin Management

```bash
sqldev plugin search codegen       # search the registry
sqldev plugin install csharp       # install from registry
sqldev plugin list                 # show installed plugins
sqldev plugin update               # update all plugins
sqldev plugin create my-plugin     # scaffold a new plugin project
```

### Plugin SDK & Authoring

```bash
sqldev plugin create my-formatter
# Creates:
#   sqldev-output-my-formatter/
#   ├── plugin.yml          # metadata: name, version, category, supported actions
#   ├── schema.json         # input contract (JSON Schema)
#   ├── main.go             # entry point (or main.rs, index.ts, etc.)
#   └── testdata/           # sample inputs for testing
```

Ship `sqldev-plugin-sdk` libraries for Go, Rust, TypeScript, and Python with:
- Input parsing helpers (schema graph, query results, migration metadata)
- Output formatting helpers
- Test harness (`sqldev plugin test` runs testdata through the plugin)

### Plugin Registry

GitHub-based index (like Homebrew taps). Low infrastructure to start, scales naturally.

```
github.com/sqldev-plugins/registry
├── plugins/
│   ├── codegen-csharp.yml
│   ├── codegen-typescript.yml
│   ├── output-xlsx.yml
│   └── hook-lint.yml
```

Each entry points to the plugin's repo, version, and binary release assets.

**Supply-chain hygiene (day one, not later):**
- Every registry entry pins a SHA256 checksum per platform binary. `sqldev plugin install` verifies before executing.
- Plugins are signed with [Sigstore](https://www.sigstore.dev/) / cosign; signatures are verified against the publisher identity recorded in the registry entry.
- `sqldev plugin install` requires `--unverified` to install anything that fails verification.
- Hook plugins run with a configurable timeout (default 30s) and fail-closed by default on prod environments. `--skip-hooks` is an explicit escape hatch and is logged.

### Schema Graph Contract (Core Abstraction)

The schema graph is the central data structure that plugins consume. It's the foundation everything builds on.

The schema is versioned starting at `0.1` and follows semver. Breaking changes bump the major version; plugins declare a compatible range in their `plugin.yml`. The contract is published as a formal JSON Schema in the `sqldev` repo so plugin authors can generate types in their language of choice.

```json
{
  "version": "0.1",
  "database": "MyAppDB",
  "schemas": [
    {
      "name": "dbo",
      "tables": [
        {
          "name": "Users",
          "columns": [
            {
              "name": "Id",
              "type": "int",
              "nullable": false,
              "identity": true,
              "primaryKey": true
            },
            {
              "name": "Email",
              "type": "nvarchar(255)",
              "nullable": false,
              "unique": true
            },
            {
              "name": "CreatedAt",
              "type": "datetime2",
              "nullable": false,
              "default": "GETUTCDATE()"
            }
          ],
          "indexes": [...],
          "foreignKeys": [...],
          "triggers": [...]
        }
      ],
      "views": [...],
      "procedures": [...],
      "functions": [...]
    }
  ]
}
```

## Project Configuration

```yaml
# .sqldev.yml
project: my-app

environments:
  local:
    # Inline passwords are supported but discouraged. Prefer env vars,
    # integrated auth, or Microsoft Entra ID (see below).
    connection: "Server=localhost,1433;User=SA;Password=${SA_PASSWORD}"

  dev:
    # Microsoft Entra ID (interactive) — recommended for developer machines.
    server: dev-sql.database.windows.net
    database: MyApp
    auth: entra-interactive

  staging:
    # Microsoft Entra ID via managed identity — for CI runners on Azure.
    server: staging-sql.database.windows.net
    database: MyApp
    auth: entra-managed-identity

  prod:
    server: prod-sql.database.windows.net
    database: MyApp
    auth: entra-service-principal
    tenant_id: ${AZURE_TENANT_ID}
    client_id: ${AZURE_CLIENT_ID}
    client_secret: ${AZURE_CLIENT_SECRET}
    protected: true   # requires --confirm flag for migrate/seed

default_environment: local

migrations:
  directory: ./migrations
  table: __sqldev_migrations

seeds:
  directory: ./seeds

codegen:
  output: ./models

plugins:
  # ... (see Plugin Configuration above)
```

## Technical Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Language | Rust | Single binary, fast startup, strong type system. TDS access via [`mssql-tiberius-bridge`](https://crates.io/crates/mssql-tiberius-bridge) — a tiberius-compatible API on top of Microsoft's official [`mssql-tds`](https://github.com/microsoft/mssql-rs) engine. Picks up the familiar tiberius surface (`Config`/`Client`/`AuthMethod`, deadpool pooling) while riding Microsoft's supported driver long-term. (Go via `go-mssqldb` is the alternative; the call goes to the team's familiarity, not the driver.) |
| Plugin model | Executable over stdin/stdout | Language-agnostic, easy to test, proven (Git/Docker model) |
| Config format | YAML | Developer-friendly, supports env var interpolation |
| Migration format | Plain SQL files | Diffable in PRs, no vendor lock-in, works with any editor |
| Schema tracking | JSON snapshot | Enables offline diffing without DB connection |

## Distribution

`sqldev` is a single static binary. Distribution must be frictionless on every platform a SQL Server developer might use — that's macOS (Intel + Apple Silicon), Linux (x86_64 + arm64, glibc + musl), and Windows (x86_64 + arm64).

### Channels

| Platform | Channel | Notes |
|---|---|---|
| **macOS / Linux** | Homebrew (`brew install sqldev`) | Primary channel. Custom tap (`sqldev/tap`) at launch; promote to homebrew-core after `v1.0` and ~75 stars. Bottles for both arches. |
| **Windows** | WinGet (`winget install sqldev`) | Primary channel. Manifest in `microsoft/winget-pkgs`. |
| **Windows** | Scoop (`scoop install sqldev`) | Power-user channel. Custom bucket initially. |
| **Windows** | Chocolatey (`choco install sqldev`) | Enterprise channel. Lower priority than WinGet. |
| **Linux** | `apt` via APT repository | Hosted at `apt.sqldev.dev`. Signed `.deb` packages for Debian/Ubuntu (amd64 + arm64). |
| **Linux** | `dnf` / `yum` via RPM repository | Hosted at `rpm.sqldev.dev`. Signed `.rpm` for Fedora/RHEL/Rocky. |
| **Linux** | AUR (`yay -S sqldev`) | Community-maintained PKGBUILD; we ship the source-of-truth. |
| **Linux** | Snap (`snap install sqldev`) | Strict confinement; useful for Ubuntu LTS users. Lower priority. |
| **Cross-platform** | `cargo install sqldev` | For Rust developers; builds from source. |
| **Cross-platform** | `mise` / `asdf` plugin | For polyglot dev-tool managers. Community plugin, registry pointer maintained by us. |
| **Containers** | `ghcr.io/sqldev/sqldev` | Multi-arch image for CI pipelines. `sqldev:latest`, `sqldev:0.2`, `sqldev:0.2.1`. |
| **Direct** | GitHub Releases | Always the source of truth. Pre-built tarballs/zips per `target-triple`, plus checksums and signatures. |
| **One-liner** | `curl -fsSL https://sqldev.dev/install.sh \| sh` | Detects OS/arch, fetches the right tarball from GitHub Releases, verifies checksum + signature, installs to `~/.local/bin` (or `/usr/local/bin` if root). PowerShell equivalent at `https://sqldev.dev/install.ps1`. |

### Release artifacts (per version)

For every tagged release, GitHub Actions produces:

- `sqldev-<version>-x86_64-apple-darwin.tar.gz`
- `sqldev-<version>-aarch64-apple-darwin.tar.gz`
- `sqldev-<version>-x86_64-unknown-linux-gnu.tar.gz`
- `sqldev-<version>-aarch64-unknown-linux-gnu.tar.gz`
- `sqldev-<version>-x86_64-unknown-linux-musl.tar.gz` (static, for Alpine/distroless)
- `sqldev-<version>-aarch64-unknown-linux-musl.tar.gz`
- `sqldev-<version>-x86_64-pc-windows-msvc.zip`
- `sqldev-<version>-aarch64-pc-windows-msvc.zip`
- `.deb` and `.rpm` packages per arch
- `SHA256SUMS` + `SHA256SUMS.sig` (cosign keyless signature)
- SBOM (CycloneDX) per artifact
- Provenance attestation (SLSA level 3)

### Signing & supply chain

- All binaries signed with Sigstore/cosign keyless signing tied to the GitHub Actions workflow identity. `sqldev --verify-self` verifies the running binary's signature against the public Rekor log.
- macOS binaries notarized via Apple Developer ID; gatekeeper-friendly out of the box.
- Windows binaries signed with an EV code-signing certificate (post-`v0.3` once SmartScreen reputation matters); pre-`v0.3` we accept the Defender warning and document it.
- APT and RPM repositories signed with a project GPG key, rotated annually, published at `https://sqldev.dev/keys/`.

### Auto-update

`sqldev` ships with `sqldev self update`:

- Checks GitHub Releases (rate-limit aware) at most once per 24h, opt-out via `SQLDEV_NO_UPDATE_CHECK=1` and disabled by default in CI environments (detected via standard env vars).
- Downloads the matching artifact for the current target triple.
- Verifies SHA256 + cosign signature before swap.
- Atomic replace via temp-file + rename (Windows: `MoveFileEx` with `MOVEFILE_REPLACE_EXISTING`).
- A non-blocking notice in the status line when a new version is available; never auto-installs.

For users installed via a package manager (brew, apt, winget, etc.), `sqldev self update` detects this and prints the appropriate `brew upgrade sqldev` / `apt upgrade sqldev` / `winget upgrade sqldev` hint instead of self-replacing — avoiding fights with package-manager-owned files.

### Versioning policy

- Semver. `0.x` is allowed to break; from `1.0.0` onward, breaking changes require a major bump and a deprecation cycle of at least one minor release.
- The schema-graph contract and plugin protocol are versioned independently from the CLI (see Schema Graph Contract section).
- LTS branches: every `1.x.0` is supported with security/critical fixes for 12 months after the next minor.

### Telemetry on install

Zero. Install scripts and package manifests do not phone home. Any telemetry is in-binary, opt-in, and clearly disclosed on first run.

## What Exists Today (Competitive Landscape)

| Tool | Covers | Misses |
|------|--------|--------|
| `sqlcmd` | Query execution | Everything else |
| `dbmate` | Migrations | Codegen, diffing, REPL, explain, seeds |
| `sqitch` | Migrations | Clunky UX, no codegen/REPL |
| `pgcli`/`litecli` | Smart REPL | Postgres/SQLite only, no migrations |
| `atlas` | Schema-as-code, declarative HCL | SQL Server is supported but not first-class; no REPL, explain, seed, or test |
| `schemacrawler` | Schema introspection | Java, no migrations/codegen/REPL |
| Redgate tools | Schema diff, migrations | Expensive, GUI-first, proprietary |

Nobody has built the full developer experience for SQL Server as a single, cohesive CLI.

## Phased Roadmap

### Phase 0 — De-risk
Before committing to the full roadmap, prototype `sqldev diff` against a real-world schema (e.g., AdventureWorks evolved over 5–10 migrations). If diff works on the common path, the rest is execution. If it doesn't, the project is "another migration tool with a REPL" and the scope should shrink.

### Phase 1 — Foundation
- `sqldev init`, `sqldev migrate`, `sqldev query` (REPL + one-shot)
- `.sqldev.yml` config with multi-environment support
- Microsoft Entra ID auth (interactive, managed identity, service principal) + SQL auth
- Built-in output formats: table, JSON, CSV

### Phase 2 — Intelligence
- `sqldev diff` (schema diffing + migration generation, scoped object types)
- `sqldev explain` (human-readable query plans) — *the headline feature*
- `sqldev seed` (test data generation)
- Built-in C# codegen (most-requested target for SQL Server shops)

### Phase 3 — Ecosystem
- Plugin architecture (contract, SDK, registry, signing)
- `sqldev codegen` for additional languages (via plugins)
- `sqldev watch` and `sqldev test`
- Hook system for CI/CD integration
- DacFx/sqlpackage plugin for diff edge cases

## Non-Goals

Explicitly out of scope, at least for v1:

- **DBA tooling.** No backup/restore orchestration, no Always On configuration, no SQL Agent job management, no replication setup. Use SSMS or `dbatools` for those.
- **Query authoring IDE.** No IntelliSense-grade language server, no graphical query designer. The REPL has tab-completion; that's it.
- **Cross-database/cross-server scripts.** Single-database scope per project. Multi-tenant fan-out is a future plugin.
- **Linked servers, Service Broker, CDC, Change Tracking.** These are DBA-domain; `sqldev diff` will detect and warn.
- **Performance tuning beyond `explain`.** No wait-stat analysis, no missing-index DMV scraping, no Query Store integration in v1.
- **GUI.** Not now, not later. If you want a GUI, this is the wrong tool.
- **Engines other than SQL Server / Azure SQL.** No Postgres, no MySQL, no Snowflake. The TDS-shaped problem space is the moat.

## Target Users

- Backend developers who work with SQL Server daily
- Teams doing microservices with SQL Server backends
- Anyone who wants Git-based database workflow without buying Redgate
- Developers on macOS/Linux who can't use SSMS
- CI/CD pipelines that need database migration automation

---

*Built for developers who think in code, not clicks.*
