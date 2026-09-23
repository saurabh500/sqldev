# Microsoft SQL Server ODBC conformance tests

This suite compares Microsoft ODBC Driver 18 for SQL Server with the Rust ODBC
driver from [microsoft/mssql-rs](https://github.com/microsoft/mssql-rs), through
the same unixODBC driver manager against SQL Server 2025. It covers:

- connection setup and driver/driver-manager information;
- direct statements and result metadata;
- prepared statements and bound parameters;
- commit and rollback behavior;
- table and column metadata;
- SQLSTATE diagnostic records; and
- wide-character parameter and result round trips.

## Imported conversion tests

The two suites under `rust-odbc-parity/tests/` are compiled into the same
GoogleTest executable and discovered automatically by CTest and CI. No Rust
compiler or separate driver build is required:

- `interval_conversion_test.cpp`: exact numeric and character-to-interval
  conversions, interval fields, fractional truncation, range errors, SQLSTATEs,
  NULLs, buffer guards, and statement recovery.
- `numeric_parser_conversion_test.cpp`: varchar/nvarchar-to-numeric conversions,
  signed/unsigned boundaries, exponent parsing, malformed inputs, precision,
  SQLSTATEs, buffer guards, and recovery after conversion errors.

Together they add **1,108 parameterized cases**, exercising `SQLGetData` and
`SQLBindCol` under ODBC 3.0 and 3.8. Their assertions are preserved; adaptations
are limited to build integration, connection configuration, and a login timeout.
Their existing `ODBC_TEST_*` settings take precedence when supplied, falling back
to the corresponding repository `ODBC_*` settings so CI uses the same SQL Server.
`ODBC_TEST_CONNSTR` overrides `ODBC_CONNECTION_STRING`; otherwise the mapped pairs
are SERVER/SERVER, DATABASE/DATABASE, UID/USER, PWD/PASSWORD, and DRIVER/DRIVER.

```bash
ctest --test-dir build/odbc-conformance -R 'IntervalConversionTest' --output-on-failure
ctest --test-dir build/odbc-conformance -R 'NumericParserConversionTest' --output-on-failure
```

## Datatype coverage

### NULL output-buffer preservation

`OutputBufferPreservation` adds **18 independently discovered cases** for NULL
`binary(3)`, `varbinary(8)`, and `varbinary(max)`, retrieved as `SQL_C_CHAR`,
`SQL_C_WCHAR`, or `SQL_C_BINARY` through both `SQLGetData` and
`SQLBindCol`/`SQLFetch`. Each case starts with a 32-byte destination and surrounding
guard bytes filled with `0x7E`, then checks `SQL_SUCCESS`, `SQL_NULL_DATA`, an
entirely unchanged destination, and unchanged buffer/indicator guards.

These are **Driver 18 buffer-preservation parity assertions**, not a claim that
every byte-preservation rule is mandated by the ODBC specification. They capture
[microsoft/mssql-rs#555](https://github.com/microsoft/mssql-rs/issues/555), where a
NULL binary value causes an unwanted character terminator to be written inside
the destination. Return-code, indicator-only, and out-of-bounds-only assertions
would miss this behavior. Binary targets and bound delivery have independent
cases, so confirmed failures can be excluded without hiding passing paths.
Driver 18 passes all 18 cases. The pinned Rust driver fails the 12 character and
wide-character cases across both APIs; those exact cases are quarantined under
[#63](https://github.com/saurabh500/sqldev/issues/63). Its six binary-target cases
remain enabled.

The focused fixture uses a simple SELECT rather than the full datatype table;
it also runs against SQL Server 2022. Run all its assertions, including any
driver-specific quarantines, directly:

```bash
build/odbc-conformance/odbc-conformance \
  --gtest_filter='Nulls/OutputBufferPreservation.*'
```

### Datatype retrieval matrix

The datatype fixture creates a single session-local table with **39 datatype
columns** plus a row-order key,
covering SQL Server 2025 built-in column types and important storage variants:

| Family | Columns |
| --- | --- |
| Exact numeric | bigint, bit, tinyint, smallint, int, decimal(38,4), numeric(18,5), money, smallmoney |
| Approximate numeric | float(53), real |
| Date/time | date, datetime, datetime2(7), smalldatetime, time(7), datetimeoffset(7) |
| Character | char, varchar, text, nchar, nvarchar, ntext, varchar(max), nvarchar(max), sysname |
| Binary | binary, varbinary, image, varbinary(max), rowversion |
| Other | uniqueidentifier, sql_variant, xml, hierarchyid, geometry, geography, json, vector(3) |

It inserts populated, NULL, and repeated-value rows, then verifies the values
through all of these retrieval paths:

- `SQLGetData` with typed C buffers, including exact `SQL_NUMERIC_STRUCT`
  precision/scale and Microsoft time/datetimeoffset structures;
- `SQLGetData` conversions to `SQL_C_CHAR` and `SQL_C_WCHAR`;
- simultaneous `SQLBindCol` bindings followed by `SQLFetch`;
- column-wise and row-wise native array binding with `SQLFetchScroll`, including
  a partial final rowset and row-status/rows-fetched indicators;
- chunked `SQLGetData` for long character, wide-character, and binary values,
  checking `01004` truncation diagnostics, terminators, lengths, and reconstructed
  payloads.

Assertions compare fixed expected values, not just successful return codes.
Native retrieval and both rowset layouts are independently parameterized by
column, so quarantining one driver/type combination does not hide the rest.
Each assertion includes the column and row context. Generated rowversion values
are snapshotted as binary(8) and compared across retrieval paths; a separate
`TimestampAlias` case verifies the timestamp synonym and changes after updates
(SQL Server permits only one rowversion/timestamp column per table).

The complete table **requires SQL Server 2025**; missing JSON/vector support is
a test failure, not a skip. JSON and vector use Driver 18's default textual
representations (`vectorTypeSupport=off`), while hierarchyid and spatial types
use their serialized binary/hex representations. Native vector negotiation and
preview-only `vector(..., float16)` are not covered. `sql_variant` uses an integer
payload here; exhaustive variant subtypes and all precision/length combinations
are not claimed. `cursor` and `table` cannot be column types, and custom
user-defined types are outside this built-in column suite.

### Observed conformance failure

`OdbcConformance.FetchScrollEndOfDataRowsFetched` isolates an observed failure
with Driver 18/unixODBC: after a partial final rowset, `SQLFetchScroll` returns
`SQL_NO_DATA` but leaves `SQL_ATTR_ROWS_FETCHED_PTR` at `1`.
[ODBC requires that count to be zero][rows-fetched]. The test is disabled by
request for Driver 18 while [issue #52](https://github.com/saurabh500/sqldev/issues/52)
tracks investigation. `known-failures.json` excludes it only for that driver in
the comparison run; its assertion remains unchanged and it runs for mssql-rs.
The datatype rowset tests separately check values, NULL indicators, and counts
on successful fetches.

Run the disabled regression explicitly when investigating:

```bash
build/odbc-conformance/odbc-conformance \
  --gtest_filter=OdbcConformance.FetchScrollEndOfDataRowsFetched
```

[rows-fetched]: https://learn.microsoft.com/sql/odbc/reference/syntax/sqlfetch-function#rows-fetched-buffer

The C++17 suite uses GoogleTest fixtures and assertions. Each area is discovered
as a separate CTest test, so failures identify the ODBC surface that regressed.
Each test opens its own connection and releases ODBC handles even after assertion
failures. The tests default to `tempdb`, use session-local tables where possible,
and clean up the uniquely named table used for catalog metadata.

## Run locally on Ubuntu

Install [Microsoft ODBC Driver 18 for SQL Server][driver-install], then install
CMake 3.21 or newer, a C++17 compiler, GoogleTest, and unixODBC development headers
(Ubuntu 22.04 or newer):

```bash
sudo apt-get install -y cmake ninja-build g++ libgtest-dev unixodbc-dev libssl-dev pkg-config
```

GoogleTest is supplied by the system package; configuring the build does not
download dependencies or require a running SQL Server. Start SQL Server 2025
and run the suite:

```bash
docker compose -f odbc-conformance/compose.yaml up -d

cmake -S odbc-conformance -B build/odbc-conformance \
  -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build/odbc-conformance

export ODBC_SERVER=127.0.0.1,1433
export ODBC_USER=sa
export ODBC_PASSWORD='Conformance!Pass2026'
# Run this until SQL Server accepts a login, then run the complete suite.
build/odbc-conformance/odbc-conformance --gtest_filter=OdbcConformance.Connection
ctest --test-dir build/odbc-conformance --output-on-failure --no-tests=error
```

Direct CTest/GoogleTest runs deliberately include every assertion, including
known failures. Use the comparison runner below for issue-linked, driver-specific
exclusions.

If port 1433 is already occupied, set `ODBC_SQLSERVER_PORT` when starting
Compose and use the same port in `ODBC_SERVER`.

Stop the local server with:

```bash
docker compose -f odbc-conformance/compose.yaml down
```

The defaults are `ODBC Driver 18 for SQL Server`, `127.0.0.1,1433`, `sa`, and
`tempdb`. Override them with `ODBC_DRIVER`, `ODBC_SERVER`, `ODBC_USER`,
`ODBC_PASSWORD`, and `ODBC_DATABASE`. Alternatively, set
`ODBC_CONNECTION_STRING` to supply the complete DSN-less connection string.
The connection string is never printed.
The datatype tests also use `msodbcsql.h`, installed with Driver 18 under
`/opt/microsoft/msodbcsql18/include`. For a nonstandard installation, configure
CMake with `-DMSODBCSQL_INCLUDE_DIR=/path/to/driver/include`.

Run one area directly while developing:

```bash
build/odbc-conformance/odbc-conformance --gtest_filter=OdbcConformance.Diagnostics
```

List cases without connecting, or run a selected case through CTest:

```bash
build/odbc-conformance/odbc-conformance --gtest_list_tests
ctest --test-dir build/odbc-conformance -R '^OdbcConformance.Unicode$' --output-on-failure
ctest --test-dir build/odbc-conformance -R 'DataType' --output-on-failure
```

CTest writes one GoogleTest XML report per case under
`build/odbc-conformance/test-results/`. Direct invocations can generate reports
with `--gtest_output=xml:results.xml`.

## GitHub Actions

The `ODBC conformance` workflow runs on pull requests, pushes to `main` and
`odbc-conformance`, manual dispatch, and every three hours. Its required check remains named
`unixODBC + Microsoft ODBC Driver 18` to match branch protection; despite the
historical check name, it exercises both drivers against SQL Server 2025.
It installs Driver 18, checks out the Rust driver's pinned
commit from `driver-source.json`, builds `mssqlodbc` with the specified Rust
toolchain, and builds the GoogleTest executable once. PR/push runs and default
manual runs stay pinned; scheduled audits use upstream `main` as described below.
Every CI report records the actual checked-out Rust commit, not just a branch name.

After a successful ODBC readiness check, `compare.py` runs the identical CTest
inventory separately against both drivers. Each driver gets its own connection
settings, logs, CTest JUnit report, and GoogleTest XML reports. Failures on the
first driver do not prevent execution against the second.

The `odbc-conformance-comparison` artifact contains `comparison.csv` (one row per
test with both driver outcomes and issue links), `comparison.json`, `summary.md`,
per-driver XML/logs, and the resolved upstream Cargo lockfile. The job summary
shows pass/failure/disabled counts and issue groups. Unexpected failures, skipped
cases, missing results, and infrastructure errors fail CI.

### Three-hourly upstream-main audit

The cron `17 */3 * * *` requests runs at 00:17, 03:17, 06:17, and so on **UTC**.
GitHub schedules run only from the repository's default branch (`odbc-conformance`)
after this workflow is merged there, and execution may be delayed by GitHub.

Scheduled runs build **`microsoft/mssql-rs/main`**, resolved to an immutable SHA in
the report, using the configured Rust toolchain. They run the same discovered
inventory against both drivers, but also **execute every quarantined Rust case**.
Driver 18 retains its reviewed exclusions.

Audit reports preserve raw pass/fail results and distinguish:

- Known Rust cases that still fail: expected audit failures, not a broken audit.
- Known Rust cases that now pass: candidate fixes requiring registry/pin review.
- Newly failing tests, skips, missing results, and infrastructure errors: fail CI.

A separate fresh-runner job with `issues: write` creates an `odbc`-labeled sqldev
issue for each original tracking issue with newly passing cases. It includes the
resolved upstream SHA, workflow/artifact link, exact test names (up to 50 in the
body, with the full list in `comparison.json`), and Driver 18 outcomes. It does
not claim that a single passing run proves a stable upstream fix.

One open notification per original issue is updated rather than duplicated every
three hours. Closing a notification suppresses that exact set of recovered cases
on later runs; a different recovered set can generate a new notification. A
managed section is updated without overwriting surrounding human-written notes.
The notification job does not execute driver code, and the build/test job remains
read-only. PR/push/default manual runs cannot file recovery issues. Neither
`driver-source.json` nor `known-failures.json` is modified automatically.

To trigger the same audit explicitly (including issue reporting):

```bash
gh workflow run odbc-conformance.yml --ref odbc-conformance -f upstream_main=true
```

For a local audit after building upstream main, supply a JSON metadata file with
`repository`, `ref: "main"`, the resolved 40-character `revision`, and
`rust_toolchain`, then add `--audit-known-failures --source-metadata PATH` to the
comparison command below. `--include-known-failures` remains a separate diagnostic
mode that runs exclusions for both drivers and fails on any failing assertion.

Preview recovery notifications without changing GitHub:

```bash
python3 odbc-conformance/report_recoveries.py \
  --report build/comparison/comparison.json \
  --run-url https://github.com/saurabh500/sqldev/actions/runs/RUN_ID \
  --dry-run
```

## Side-by-side comparison locally

Install Rust through rustup as well as the prerequisites above. From the
repository root, build the pinned upstream driver without changing system ODBC
registration:

```bash
revision=$(python3 -c 'import json; print(json.load(open("odbc-conformance/driver-source.json"))["revision"])')
toolchain=$(python3 -c 'import json; print(json.load(open("odbc-conformance/driver-source.json"))["rust_toolchain"])')
git clone https://github.com/microsoft/mssql-rs.git build/external/mssql-rs
git -C build/external/mssql-rs checkout "$revision"
rustup toolchain install "$toolchain" --profile minimal
(
  cd build/external/mssql-rs
  export RUSTUP_TOOLCHAIN="$toolchain"
  cargo build -p mssqlodbc --release
  bash mssql-odbc/scripts/finalize-artifact.sh release
)
python3 odbc-conformance/compare.py \
  --build-dir build/odbc-conformance \
  --rust-driver build/external/mssql-rs/target/release/mssqlodbc.so \
  --output-dir build/comparison
```

Use the same `ODBC_SERVER`, `ODBC_DATABASE`, `ODBC_USER`, and `ODBC_PASSWORD`
settings as above. Comparison mode intentionally rejects DSN/full-connection-string
overrides so neither driver can silently be replaced. It synchronizes the imported
suites' credentials and TLS settings with the main suite. Driver 18 can be selected
by another registered name/path using `--msodbcsql-driver`.

To run the executable directly against Rust, set `ODBC_DRIVER` to the absolute
library path and `ODBC_TEST_TARGET=mssql-rs`. No Rust driver registration is needed:
unixODBC can load the library path supplied in `DRIVER={...}`.

## Issue-linked conformance failures

`known-failures.json` stores **exact CTest names per driver**, grouped by issue.
It cannot exclude the connection test, unknown test names, or the whole inventory,
and every entry requires a reason and a repository issue. No wildcards are stored:
unaffected parameter combinations continue to run. A disabled case is reported
as `disabled`, never as passed.

The full inventory contains **1,256 cases**. Current exact exclusions are:

| Driver | Disabled cases | Follow-up |
| --- | ---: | --- |
| Driver 18 | 1 | End-of-data rows-fetched count (#52) |
| Driver 18 | 2 | Inconsistent reference parsing cases, SQLBindCol under ODBC 3.0/3.8 (#59) |
| mssql-rs | 568 | Interval conversion family (#53) |
| mssql-rs | 12 | Numeric struct retrieval, including column-wise binding (#54) |
| mssql-rs | 8 | Numeric exponent underflow (#55) |
| mssql-rs | 16 | Reference-driver compatibility decisions (#56, #57, #58) |
| mssql-rs | 39 | Row-wise array binding (#60) |
| mssql-rs | 4 | Descriptor-based numeric retrieval (#61) |
| mssql-rs | 4 | Binary temporal structure retrieval (#62) |
| mssql-rs | 12 | NULL binary character-buffer preservation, both retrieval APIs (#63; upstream microsoft/mssql-rs#555) |

Compatibility differences and the inconsistent reference case are not
automatically classified as driver bugs. Unaffected parameter combinations
continue to run, even when other cases from the same source file are excluded.
For #59, both bound-column variants have now failed intermittently in SQL Server
2025 CI; the `SQLGetData` variants remain enabled. The reference assertions are
unchanged and can still be run directly or with `--include-known-failures`.

For newly observed failures, inspect the driver-specific XML/logs first to
distinguish test/infrastructure problems from conformance or parity failures.
Then create an `odbc`-labeled issue and record only the selected failed names:

```bash
python3 odbc-conformance/quarantine.py \
  --report build/comparison/comparison.json \
  --driver mssql-rs \
  --match '^ExactTestName$' \
  --reason 'Observed behavior and expected contract after investigation' \
  --create-issue 'ODBC mssql-rs: concise failure description'
```

Use `--issue NUMBER` instead to attach a reviewed group to an existing open issue.
The command requires a passing connection test, refuses infrastructure failures,
and changes the registry only after GitHub returns a valid issue URL. Review and
commit the resulting exact-name list in the PR. Normal comparison CI has read-only
repository permissions. Only the separate scheduled/explicit upstream-audit
notification job can file recovery issues; it never modifies the source pin or
quarantine registry.

Use `compare.py --include-known-failures` with the other comparison arguments to
retest all excluded cases. Remove resolved entries and close their issues after
confirming the fix. Assertion expectations must not be weakened just to match a
driver's incorrect behavior.

[driver-install]: https://learn.microsoft.com/sql/connect/odbc/linux-mac/installing-the-microsoft-odbc-driver-for-sql-server
