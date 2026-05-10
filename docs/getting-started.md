# Getting started with sqldev

This walkthrough takes you from "no SQL Server" to "running queries
against AdventureWorks with `sqldev`" in about 10 minutes.

## 1. Prerequisites

- **Rust toolchain (stable, 1.88+).** Install via
  [rustup](https://rustup.rs):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **Docker** (or any reachable SQL Server instance). The examples below
  use Docker.
- `jq`, `curl`, `unzip` — handy but not required.

## 2. Run SQL Server in Docker

Microsoft's official image runs on macOS, Linux, and Windows.

```bash
docker run -d --name sqldev-sql \
  -e ACCEPT_EULA=Y \
  -e MSSQL_SA_PASSWORD='Dev!Pass2025' \
  -e MSSQL_PID=Developer \
  -p 1433:1433 \
  mcr.microsoft.com/mssql/server:2022-latest

# Wait until ready:
docker exec sqldev-sql /opt/mssql-tools18/bin/sqlcmd \
  -S localhost -U SA -P 'Dev!Pass2025' -C -Q 'SELECT @@VERSION;'
```

## 3. Restore AdventureWorks2022 (optional but recommended)

```bash
curl -L -o /tmp/AdventureWorks2022.bak \
  https://github.com/Microsoft/sql-server-samples/releases/download/adventureworks/AdventureWorks2022.bak

docker cp /tmp/AdventureWorks2022.bak sqldev-sql:/var/opt/mssql/backup/

docker exec sqldev-sql /opt/mssql-tools18/bin/sqlcmd \
  -S localhost -U SA -P 'Dev!Pass2025' -C -Q "
    RESTORE DATABASE AdventureWorks2022
    FROM DISK = '/var/opt/mssql/backup/AdventureWorks2022.bak'
    WITH MOVE 'AdventureWorks2022'     TO '/var/opt/mssql/data/AdventureWorks2022.mdf',
         MOVE 'AdventureWorks2022_log' TO '/var/opt/mssql/data/AdventureWorks2022_log.ldf';"
```

## 4. Build sqldev

```bash
git clone https://github.com/saurabh500/sqldev.git
cd sqldev
cargo build --release
export PATH="$PWD/target/release:$PATH"
sqldev --version
```

## 5. Create a project config

```bash
mkdir -p ~/playground/sqldev-demo && cd ~/playground/sqldev-demo

cat > .sqldev.yml <<'YML'
default_env: dev

envs:
  dev:
    host: localhost
    port: 1433
    database: AdventureWorks2022
    auth:
      kind: sql
      user: SA
      password: ${DEV_SA_PASSWORD}
    trust_server_certificate: true

  prod:
    host: prod.example.com
    database: AdventureWorks
    auth: { kind: sql, user: appuser, password: ${PROD_PASSWORD} }
    protected: true
YML
```

```bash
export DEV_SA_PASSWORD='Dev!Pass2025'
```

> **Tip:** put the `export` in a `.envrc` and use
> [direnv](https://direnv.net/) so it loads automatically when you `cd`
> into the repo.

## 6. Inspect the resolved config

```bash
sqldev config show
```

You should see your `dev` block with `password: '***REDACTED***'`. Pass
`--all` to dump every env, `--json` to get JSON, `--env prod` to switch.

## 7. Run a query

```bash
sqldev query --sql 'SELECT TOP 5 name FROM sys.tables ORDER BY name'

# JSON, ready for jq
sqldev query \
  --sql 'SELECT TOP 5 name, type_desc FROM sys.objects ORDER BY name' \
  --format json | jq

# Pipe SQL via stdin
echo 'SELECT @@VERSION AS v;' | sqldev query --format json | jq -r '.[0].v'
```

## 8. Snapshot the schema

```bash
sqldev introspect > schema.json
jq '.schemas | length' schema.json
jq '.schemas[].tables | length' schema.json | paste -sd+ - | bc
jq '[.schemas[].tables[].foreign_keys[]?] | length' schema.json
```

On AdventureWorks2022 you should see **6** schemas, **71** tables, and
**90** foreign keys.

## 9. Try the protected env

```bash
sqldev --env prod query --sql 'SELECT 1' --trust-cert true
```

This must fail with:

```
Error: resolve connection options
Caused by: env block `prod` is marked `protected: true` but --trust-cert is set; …
```

That's the safety story working as advertised.

## 10. What's next

- Watch [issue #8](https://github.com/saurabh500/sqldev/issues/8) for
  CSV / NDJSON / aligned-table output formats (M1.4 polish).
- Watch [issue #10](https://github.com/saurabh500/sqldev/issues/10) for
  `sqldev init` and [issue #11](https://github.com/saurabh500/sqldev/issues/11)
  for `sqldev migrate`.
- The full v0.1 → v1.0 roadmap is in
  [`sqldev-execution-plan.md`](../sqldev-execution-plan.md).

## Cleanup

```bash
docker rm -f sqldev-sql
rm -rf ~/playground/sqldev-demo
```
