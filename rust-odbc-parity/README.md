# Rust ODBC driver — data-type conversion parity suite

This folder holds two GoogleTest suites, copied out of a larger, private
conformance investigation (`~/odbctest` on this machine), that check whether
Microsoft's Rust ODBC driver (`mssql-odbc`, from the `mssql-rs` repository)
converts SQL data types to C types with the **same correctness guarantees**
as Microsoft's native ODBC Driver 18 for SQL Server (`msodbcsql`).

Both suites are ordinary ODBC 3.x client code: they call `SQLBindCol`,
`SQLGetData`, `SQLFetch`, etc. through the unixODBC driver manager. The exact
same test binary runs unmodified against either driver — only the ODBC
driver shared library and a couple of environment variables change between
runs. This is what makes the comparison meaningful: any behavioral
difference in the results is a genuine driver difference, not a test
artifact.

## Why these two suites specifically

They were the two suites written to close **measured** gaps, not guesses.
The private investigation instrumented `msodbcsql`'s own C++ source with
`gcov`/`gcovr`, ran the existing data-path test suites, and inspected which
conversion routines in `sqlccnvt.cpp` (the SQL-to-C conversion dispatcher)
never executed. Two genuinely untested conversion families fell out of that
analysis, and a suite was written for each:

| Suite | Conversion family | SQL sources | C targets |
|---|---|---|---|
| `numeric_parser_conversion_test.cpp` | Character-to-numeric parsing | `varchar`/`nvarchar` literals (integers, decimals, exponents, edge cases) | `SQL_C_*` integer/float/numeric types |
| `interval_conversion_test.cpp` | Numeric/text-to-interval conversion | `tinyint`/`smallint`/`int`/`bigint`/`decimal`/`numeric` columns, and `INTERVAL '...' <unit>` text literals via `varchar`/`nvarchar` | The six single-field ODBC C interval structs (year, month, day, hour, minute, second) |

Both suites assert on more than the return code: SQLSTATE, exact converted
value/precision/scale, indicator length, and (for the interval suite)
out-of-bounds guard bytes around the target buffer to catch overwrites.

## Results measured so far

These are the actual outcomes from running both suites against msodbcsql
18.6 (retail), a same-revision instrumented `msodbcsql` source build, and
the Rust driver, against a live SQL Server. They are **not exhaustive** —
see "Scope and limitations" below.

**`numeric_parser_conversion_test` (528 parameter combinations):**
Both native builds pass all 528. Rust passes 496 and fails 32, all
classified into five compatibility behaviors — one confirmed missing
conversion target (`SQL_C_NUMERIC` delivery), one confirmed Linux
underflow-handling gap, and three cases that are reference-driver quirks or
existing, deliberate SQLSTATE policy rather than Rust bugs. Every failure is
individually triaged, not just counted.

**`interval_conversion_test` (580 cases: 372 numeric-source + 208
text-literal-source):**
Both native builds pass all 580. Rust passes only the 12 NULL-input cases
and fails the remaining 568 with `SQLSTATE HYC00` — "Target type not yet
implemented" — for every non-NULL exact-numeric or `INTERVAL '...'` literal
conversion to a C interval struct. This is **one missing conversion family**
in the Rust driver, not 568 independent bugs: it never implemented
SQL-to-C-interval conversion at all. [SQL to C: Numeric][sql-to-c-numeric]
documents this as a required exact-numeric-to-C conversion.

Full per-case results, SQLSTATEs, and diagnostic text for every run are kept
in the private `~/odbctest` workspace (`FAILURE_LEDGER.md`, `TEST_PLAN_LEDGER.md`,
and `artifacts/runs/`); this folder intentionally carries only the source
code so it can be shared without any of that campaign's large coverage/log
artifacts.

## Building

Requires CMake 3.15+, a C++17 compiler, and unixODBC development headers
(`unixodbc-dev` on Ubuntu). GoogleTest is fetched automatically by CMake;
no system GoogleTest package or running SQL Server is needed to configure
or build.

```bash
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build -j"$(nproc)"
```

This produces `build/numeric_parser_conversion_test` and
`build/interval_conversion_test`.

## Running against a driver

Both binaries read connection info from environment variables (see
`include/odbc_test_fixture.h` for the full list):

```bash
export ODBC_TEST_DRIVER="ODBC Driver 18 for SQL Server"   # or a registered driver name
export ODBC_TEST_SERVER="127.0.0.1,1433"
export ODBC_TEST_DATABASE="tempdb"
export ODBC_TEST_UID="sa"
export ODBC_TEST_PWD="<password>"
export ODBC_TEST_TRUST_CERT="Yes"
export ODBC_TEST_ENCRYPT="Optional"

./build/numeric_parser_conversion_test
./build/interval_conversion_test
```

To compare two drivers without touching system-wide ODBC registration,
point `ODBC_TEST_DRIVER` at a private `odbcinst.ini` entry and set
`ODBCSYSINI`/`ODBCINSTINI` to select it, e.g.:

```bash
mkdir -p /tmp/odbc-reg
cat > /tmp/odbc-reg/odbcinst.ini <<'EOF'
[Rust ODBC under test]
Driver=/path/to/libmssqlodbc.so
Threading=1
EOF
export ODBCSYSINI=/tmp/odbc-reg
export ODBCINSTINI=odbcinst.ini
export ODBC_TEST_DRIVER="Rust ODBC under test"
```

Set `ODBC_TEST_TARGET=msodbcsql` to skip the handful of cases in the wider
e2e harness that intentionally assert Rust-specific behavior (neither of
these two suites currently defines such a case, but the fixture macro is
inherited unchanged from the parent harness).

Filter to one suite or case with GoogleTest's own flag:

```bash
./build/interval_conversion_test --gtest_filter='Shared/TextIntervalConversionTest.*'
./build/interval_conversion_test --gtest_list_tests
```

## Scope and limitations

- This covers two specific, measured conversion-family gaps. It is **not**
  a general ODBC conformance suite — no connection/auth, catalog, cursor,
  transaction, or bulk-operation coverage is included here.
- "580 cases" and "528 cases" are parameter combinations (source type x
  target type x API x ODBC version), not 1,108 independently hand-designed
  scenarios; each is a real, distinct assertion, but many share structure.
- Native JSON/vector conversion paths (SQL Server 2025) were out of scope
  for this extraction: the SQL Server 2025 container images tested would
  not start on the machine used for this investigation, independent of
  either driver.

## Provenance

Ported unchanged (except for this README) from `mssql-rs/mssql-odbc`'s own
`tests/e2e` GoogleTest harness, where they remain registered in that
project's `CMakeLists.txt` and continue to run as part of its broader
(26+ suite) end-to-end test set. `include/odbc_test_fixture.h` and the
three `lib/odbc_test_*.cpp` files are that harness's shared fixture,
copied here because both suites depend on it. All copied files retain
their original "Copyright (c) Microsoft Corporation" headers.

[sql-to-c-numeric]: https://learn.microsoft.com/en-us/sql/odbc/reference/appendixes/sql-to-c-numeric
