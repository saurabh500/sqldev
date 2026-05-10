# Contributing to sqldev

Thanks for your interest! This project is in early pre-release; the
contracts (CLI flags, `.sqldev.yml` keys, schema-graph JSON) will keep
changing until v1.0.

## Development

Prerequisites:

- Rust toolchain (stable, MSRV is `1.88`).
- A SQL Server you can talk to. The dev container in
  [docs/getting-started.md](docs/getting-started.md) is fine.

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

CI runs the same four commands on macOS / Linux / Windows.

## Commit conventions

We use [Conventional Commits](https://www.conventionalcommits.org/) so
[release-please](https://github.com/googleapis/release-please) can
generate the `CHANGELOG.md` and tag releases automatically.

| Prefix | Use for | Bumps |
|---|---|---|
| `feat:` | A user-visible new capability. | minor |
| `fix:` | A bug fix. | patch |
| `perf:` | Performance improvement. | patch |
| `refactor:` | Internal change with no user impact. | patch |
| `docs:` | Documentation only. | patch |
| `ci:` / `build:` / `test:` / `chore:` | Hidden from changelog. | none |

A `!` after the type (e.g. `feat!:`) or a `BREAKING CHANGE:` footer
forces a major bump (or `0.x → 0.(x+1)` while pre-1.0).

## PR workflow

`main` is protected: every change lands through a pull request that
passes `ci.yml`. Stack PRs when one feature depends on another;
prefer squash merges so the history mirrors the changelog.
