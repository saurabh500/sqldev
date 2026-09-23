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

Each area is a separate CTest test, so failures identify the ODBC surface that
regressed. The tests use `tempdb` and clean up their own objects.

## Run locally on Ubuntu

Install CMake, a C compiler, unixODBC development headers, and
[Microsoft ODBC Driver 18 for SQL Server][driver-install]. Then start SQL
Server and run the suite:

```bash
docker compose -f odbc-conformance/compose.yaml up -d

cmake -S odbc-conformance -B build/odbc-conformance \
  -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build/odbc-conformance

export ODBC_SERVER=127.0.0.1,1433
export ODBC_USER=sa
export ODBC_PASSWORD='Conformance!Pass2026'
ctest --test-dir build/odbc-conformance --output-on-failure
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

Run one area directly while developing:

```bash
build/odbc-conformance/odbc-conformance diagnostics
```

[driver-install]: https://learn.microsoft.com/sql/connect/odbc/linux-mac/installing-the-microsoft-odbc-driver-for-sql-server
