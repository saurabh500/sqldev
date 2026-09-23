# Copilot instructions

## Repository purpose

`sqldev` is an ODBC conformance test suite using C++17 GoogleTest for Microsoft ODBC
Driver 18 for SQL Server. It runs through unixODBC against a real SQL Server
2025 instance. The executable in `odbc-conformance/src/conformance.cpp` uses
GoogleTest fixtures and CTest discovers each case independently.

## Before changing code

1. Read `odbc-conformance/README.md`, `odbc-conformance/CMakeLists.txt`, and
   the relevant parts of `src/conformance.cpp` and `src/datatype_samples.h`.
2. Check the CI workflow in `.github/workflows/odbc-conformance.yml` when
   changing dependencies, setup, or test behavior.
3. Keep the existing C++17/GoogleTest, unixODBC, and Microsoft ODBC Driver 18 assumptions
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
build/odbc-conformance/odbc-conformance --gtest_filter=OdbcConformance.Diagnostics
```

Use `--gtest_list_tests` to list cases without connecting. The complete datatype
suite requires SQL Server 2025 (JSON and vector included). Install `libgtest-dev`
and the Driver 18 headers along with the documented build dependencies.

## Coding and testing rules

- Use C++17, GoogleTest assertions, and the existing ODBC diagnostic helpers.
- Preserve warning-free builds (`-Wall -Wextra -Wpedantic -Werror` on GCC and
  Clang; `/W4 /WX` on MSVC).
- Make every test deterministic and clean up allocated ODBC handles, cursors,
  transactions, and database objects on both success and failure.
- Keep tests independent and use uniquely named temporary objects where
  appropriate.
- Add `TEST_F` or parameterized cases; `gtest_discover_tests` registers them
  automatically. Register additional source files explicitly in CMake.
- Preserve real conformance failures and document the specification/observed
  behavior; do not silently skip or weaken assertions to make CI pass.
- Update the README when commands, prerequisites, environment variables, or
  coverage change.
- Avoid unrelated refactors, new dependencies, generated build artifacts, and
  committed secrets.
