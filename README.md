# sqldev

> A developer-friendly SQL Server CLI. *sqlcmd for humans, with migrations.*

[![ci](https://github.com/saurabh500/sqldev/actions/workflows/ci.yml/badge.svg)](https://github.com/saurabh500/sqldev/actions/workflows/ci.yml)
[![license](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-blue.svg)](#license)
[![status](https://img.shields.io/badge/status-pre--release-orange.svg)](CHANGELOG.md)

`sqldev` is a single static binary that lets you query SQL Server, snapshot
its schema, generate migrations, and (eventually) explain bad query plans
— without leaving your terminal.

> **Status: pre-release.** v0.1.x is the walking skeleton. Expect rough
> edges and breaking changes until v1.0. Track progress on the
> [open milestones](https://github.com/saurabh500/sqldev/milestones).

---

## Why another SQL Server CLI?

`sqlcmd` is great at being faithful to T-SQL. It is not great at being
a *daily driver* for application developers. `sqldev` is opinionated about
that workflow:

- **Config files, not flag soup.** A `.sqldev.yml` per repo defines the
  `dev`, `staging`, `prod` env blocks and `protected: true` makes
  destructive ops fail-closed.
- **JSON out of the box.** Every command emits machine-readable output
  (`--format json`, schema-graph JSON, plan JSON) so you can pipe to `jq`.
- **Migrations are first-class.** `sqldev migrate up|down|status|create`
  with transactional apply and `sp_getapplock` locking — *coming in M1.7*.
- **The schema is a graph.** Tables, indexes, constraints, UDDTs, views,
  procs, triggers — one canonical JSON document, used by `diff`,
  codegen, and (later) plugins.

## What works today (v0.1.0)

| Command | Status |
|---|---|
| `sqldev introspect` | ✅ — emits the v0.1 schema-graph JSON. |
| `sqldev query --sql … \| --format text\|json` | ✅ — one-shot, stdin too. |
| `sqldev config show [--all] [--json]` | ✅ — redacted resolved config. |
| `--config <path>` / `--env <name>` global flags | ✅ |
| `.sqldev.yml` with `${VAR}` / `${VAR:-fallback}` | ✅ |
| `protected: true` env enforcement | ✅ |
| `sqldev diff` | 🧪 spike-only ([spike-diff](spike-diff/)) |
| `sqldev explain` | 🧪 spike-only ([spike-explain](spike-explain/)) |
| `sqldev migrate` | ⏭ M1.7 |
| `sqldev init` | ⏭ M1.6 |
| `sqldev query` REPL | ⏭ M1.5 |

The full plan lives in [sqldev-execution-plan.md](sqldev-execution-plan.md);
the proposal motivation is in [sqldev-proposal.md](sqldev-proposal.md).

## Install

> Pre-built binaries are produced by the
> [release-please workflow](.github/workflows/release-please.yml) once the
> first GitHub Release is cut. Until then, build from source.

```bash
git clone https://github.com/saurabh500/sqldev.git
cd sqldev
cargo build --release
./target/release/sqldev --version
```

Targets we ship pre-built tarballs for at release time:

- `x86_64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

## Quick start

```bash
# 1. Drop a .sqldev.yml in your repo (see below).
cat > .sqldev.yml <<'YML'
default_env: dev
envs:
  dev:
    host: localhost
    port: 1433
    database: AdventureWorks2022
    auth: { kind: sql, user: SA, password: ${DEV_SA_PASSWORD} }
    trust_server_certificate: true
YML

# 2. Sanity-check the connection by inspecting the resolved config.
export DEV_SA_PASSWORD='your-password'
sqldev config show

# 3. Run a query.
sqldev query --sql 'SELECT TOP 3 name FROM sys.tables ORDER BY name'

# 4. Snapshot the whole schema.
sqldev introspect | jq '.schemas | length'
```

For a step-by-step walkthrough — including spinning up SQL Server in
Docker and restoring AdventureWorks — see
[**docs/getting-started.md**](docs/getting-started.md).

## Configuration

`.sqldev.yml` is auto-discovered by walking up from the current
directory. Override with `--config <path>`. The full key reference:

```yaml
default_env: dev          # used when --env is omitted

envs:
  dev:
    host: localhost
    port: 1433            # default
    database: app_dev
    auth:
      kind: sql           # only kind in v0.1; entra-* lands in M1.2
      user: sa
      password: ${DEV_SA_PASSWORD}   # ${VAR} or ${VAR:-fallback}
    trust_server_certificate: true   # rejected when protected: true

  prod:
    host: prod.example.com
    database: app
    auth: { kind: sql, user: appuser, password: ${PROD_PASSWORD} }
    protected: true       # blocks --trust-cert; future: requires --confirm <env>
```

CLI flags always override the env block. `SQLDEV_HOST`, `SQLDEV_PORT`,
`SQLDEV_DATABASE`, `SQLDEV_USER`, `SQLDEV_PASSWORD`, `SQLDEV_TRUST_CERT`
are consulted last, before falling back to `localhost:1433` / `SA`.

## Repository layout

```
.
├── crates/
│   ├── sqldev-core/        # shared types: schema graph, errors
│   ├── sqldev-conn/        # Tiberius wrapper, ConnectOptions, AuthOptions
│   ├── sqldev-config/      # .sqldev.yml loader + ${VAR} interpolation
│   ├── sqldev-introspect/  # system-catalog walker (lib)
│   └── sqldev-cli/         # the `sqldev` binary
├── spike-introspect/       # M0 evidence — kept for reference
├── spike-diff/             # M0 evidence — kept for reference
├── spike-explain/          # M0 evidence — kept for reference
├── docs/
│   ├── getting-started.md
│   └── demo-script.md      # for recording in Clipchamp
├── sqldev-proposal.md
├── sqldev-execution-plan.md
└── CHANGELOG.md
```

## Contributing

- All changes land via PR. `main` is protected.
- Use [Conventional Commits](https://www.conventionalcommits.org/) so
  `release-please` can build the changelog automatically.
- See [CONTRIBUTING.md](CONTRIBUTING.md) for the dev loop.

## License

Dual-licensed under either of:

- [MIT license](LICENSE-MIT)
- [Apache License 2.0](LICENSE-APACHE)

at your option.
