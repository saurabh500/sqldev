# Microsoft SQL Server ODBC conformance tests

This suite exercises Microsoft ODBC Driver 18 for SQL Server through the
unixODBC driver manager against a real SQL Server instance. It covers:

- connection setup and driver/driver-manager information;
- direct statements and result metadata;
- prepared statements and bound parameters;
- commit and rollback behavior;
- table and column metadata;
- SQLSTATE diagnostic records; and
- wide-character parameter and result round trips.

## Datatype coverage

The datatype fixture creates a single session-local table with **39 columns**
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
[ODBC requires that count to be zero][rows-fetched]. The assertion remains
enabled, so CI reports this as a failure rather than silently accepting it.
The datatype rowset tests separately check values, NULL indicators, and counts
on successful fetches.

[rows-fetched]: https://learn.microsoft.com/sql/odbc/reference/syntax/sqlfetch-function#rows-fetched-buffer

The C++17 suite uses GoogleTest fixtures and assertions. Each area is discovered
as a separate CTest test, so failures identify the ODBC surface that regressed.
Each test opens its own connection and releases ODBC handles even after assertion
failures. The tests default to `tempdb`, use session-local tables where possible,
and clean up the uniquely named table used for catalog metadata.

## Run locally on Ubuntu

Install [Microsoft ODBC Driver 18 for SQL Server][driver-install], then install
CMake 3.20 or newer, a C++17 compiler, GoogleTest, and unixODBC development headers
(Ubuntu 22.04 or newer):

```bash
sudo apt-get install -y cmake ninja-build g++ libgtest-dev unixodbc-dev
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
`odbc-conformance`, and manual dispatch. It builds with GoogleTest and unixODBC,
waits for a successful filtered connection test through Microsoft ODBC Driver 18,
then runs all discovered cases against SQL Server 2025. XML reports and the CTest
log are uploaded as the `odbc-conformance-results` artifact, including on failure
when those files exist. Connection failures are failures, not skipped tests.

[driver-install]: https://learn.microsoft.com/sql/connect/odbc/linux-mac/installing-the-microsoft-odbc-driver-for-sql-server
