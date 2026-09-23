# Copilot instructions

## Repository purpose

`sqldev` is an ODBC conformance test suite written in C99 for Microsoft ODBC
Driver 18 for SQL Server. It runs through unixODBC against a real SQL Server
instance. The executable in `odbc-conformance/src/conformance.c` exposes one
named test area at a time, and CTest registers each area independently.

## Before changing code

1. Read `odbc-conformance/README.md`, `odbc-conformance/CMakeLists.txt`, and
   the relevant parts of `src/conformance.c`.
2. Check the CI workflow in `.github/workflows/odbc-conformance.yml` when
   changing dependencies, setup, or test behavior.
3. Keep the existing C99, unixODBC, and Microsoft ODBC Driver 18 assumptions
   unless the task explicitly changes them.

## Build and test

For a local end-to-end run, start SQL Server and configure the connection
through `ODBC_CONNECTION_STRING` or the documented `ODBC_DRIVER`,
`ODBC_SERVER`, `ODBC_USER`, `ODBC_PASSWORD`, and `ODBC_DATABASE` environment
variables. Do not add credentials to source files or log output.

```bash
docker compose -f odbc-conformance/compose.yaml up -d
cmake -S odbc-conformance -B build/odbc-conformance -G Ninja \
  -DCMAKE_BUILD_TYPE=Release
cmake --build build/odbc-conformance
ctest --test-dir build/odbc-conformance --output-on-failure
docker compose -f odbc-conformance/compose.yaml down
```

To iterate on one area:

```bash
build/odbc-conformance/odbc-conformance <test-area>
```

Valid areas are `connection`, `statements`, `parameters`, `transactions`,
`metadata`, `diagnostics`, and `unicode`.

## Coding and testing rules

- Use C99 and the existing ODBC helper functions and failure-reporting style.
- Preserve warning-free builds (`-Wall -Wextra -Wpedantic -Werror` on GCC and
  Clang; `/W4 /WX` on MSVC).
- Make every test deterministic and clean up allocated ODBC handles, cursors,
  transactions, and database objects on both success and failure.
- Keep tests independent and use uniquely named temporary objects where
  appropriate.
- When adding a test area, register it in both the `CONFORMANCE_TESTS` CMake
  list and the executable's test table.
- Update the README when commands, prerequisites, environment variables, or
  coverage change.
- Avoid unrelated refactors, new dependencies, generated build artifacts, and
  committed secrets.
