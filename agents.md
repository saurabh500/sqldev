# Repository agent guidance

## Scope and project layout

This repository contains a C++17 GoogleTest ODBC conformance suite for Microsoft ODBC
Driver 18 for SQL Server:

- `odbc-conformance/src/conformance.cpp` contains fixtures and test cases.
- `odbc-conformance/src/datatype_samples.h` defines datatype fixtures and expected values.
- `odbc-conformance/CMakeLists.txt` defines the target and GoogleTest discovery for CTest.
- `odbc-conformance/compose.yaml` starts the local SQL Server dependency.
- `odbc-conformance/README.md` documents local setup and connection overrides.
- `.github/workflows/odbc-conformance.yml` is the CI workflow.

Keep changes focused on the conformance suite. Do not add application
frameworks or a second test harness without a clear requirement.

## Build and test

Use the documented Ubuntu workflow:

```bash
docker compose -f odbc-conformance/compose.yaml up -d
cmake -S odbc-conformance -B build/odbc-conformance -G Ninja \
  -DCMAKE_BUILD_TYPE=Release
cmake --build build/odbc-conformance
ctest --test-dir build/odbc-conformance --output-on-failure
docker compose -f odbc-conformance/compose.yaml down
```

The build requires CMake 3.20+, Ninja, a C++17 compiler, GoogleTest, unixODBC
development headers, Microsoft ODBC Driver 18 (including msodbcsql.h), Docker,
and a reachable SQL Server 2025 for the complete datatype suite.
Use `ODBC_CONNECTION_STRING` or the documented `ODBC_*` variables for local
configuration; never hard-code real credentials.

CTest discovers GoogleTest cases and runs them serially, including connection,
statements, parameters, transactions, metadata, diagnostics, Unicode, and
datatype retrieval. A single case can be run directly with:

```bash
build/odbc-conformance/odbc-conformance --gtest_filter=OdbcConformance.Diagnostics
```

## Implementation conventions

- Keep test code C++17 and compatible with unixODBC.
- Treat compiler warnings as errors; preserve the existing `-Wall -Wextra
  -Wpedantic -Werror` and MSVC warning settings.
- Use the existing diagnostic and assertion helpers so failures identify the
  ODBC call and diagnostic records involved.
- Each test must clean up statements, transactions, and temporary database
  objects, including failure paths.
- Do not print connection strings, passwords, or other sensitive configuration.
- Prefer focused changes and update `odbc-conformance/README.md` when setup or
  supported test areas change.

## CI expectations

The workflow installs unixODBC and Microsoft ODBC Driver 18, builds with CMake
and Ninja, waits for SQL Server through the ODBC connection itself, then runs
CTest. Changes to the test executable should be validated with the same
build/test commands where the required services are available.
GoogleTest XML reports and CTest logs are uploaded even on failures. Preserve
real conformance failures instead of weakening assertions to make CI pass.
