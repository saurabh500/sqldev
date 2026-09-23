#!/usr/bin/env python3
"""Run the identical CTest inventory through both drivers and compare outcomes."""

import argparse
import csv
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET


DRIVERS = ("msodbcsql18", "mssql-rs")
ROOT = Path(__file__).resolve().parent


def load_exclusions(path, names):
    registry = json.loads(path.read_text())
    if set(registry) != set(DRIVERS):
        raise ValueError("Known-failure registry must have exactly both driver keys")
    result = {driver: {} for driver in DRIVERS}
    for driver, groups in registry.items():
        for item in groups:
            if not re.fullmatch(r"https://github\.com/saurabh500/sqldev/issues/[1-9][0-9]*",
                                item.get("issue", "")):
                raise ValueError(f"Exclusion needs a repository issue: {driver}")
            if not item.get("reason", "").strip():
                raise ValueError(f"Exclusion needs a reason: {driver}")
            if not item.get("tests"):
                raise ValueError(f"Exclusion group is empty: {driver}")
            for name in item["tests"]:
                if name not in names or name in result[driver]:
                    raise ValueError(f"Stale or duplicate exclusion for {driver}: {name}")
                result[driver][name] = {"issue": item["issue"], "reason": item["reason"]}
        cases = result[driver]
        if "OdbcConformance.Connection" in cases:
            raise ValueError("Connection readiness must never be quarantined")
        if len(cases) == len(names):
            raise ValueError(f"All tests excluded for {driver}")
    return result


def read_results(path, expected):
    results = {}
    for case in ET.parse(path).getroot().iter("testcase"):
        name = case.attrib["name"]
        if name in results or name not in expected:
            raise ValueError(f"Duplicate or unexpected result: {name}")
        if case.find("failure") is not None or case.find("error") is not None:
            status = "failed"
        elif case.find("skipped") is not None or case.attrib.get("status") == "notrun":
            status = "skipped"
        else:
            status = "passed"
        results[name] = {"status": status, "time": case.attrib.get("time", "0")}
    missing = set(expected) - results.keys()
    if missing:
        raise ValueError(f"Missing results: {sorted(missing)}")
    return results


def selection_indices(tests, omitted):
    # Exact indices avoid CTest's regex size limit for large parameterized inventories.
    return "0,0,1," + ",".join(str(index) for index, test in enumerate(tests, 1)
                              if test["name"] not in omitted)


def driver_environment(driver, library):
    overrides = ("ODBC_CONNECTION_STRING", "ODBC_TEST_CONNSTR", "ODBC_TEST_DSN")
    if any(os.environ.get(key) for key in overrides):
        raise ValueError("Comparison requires ODBC_SERVER/USER/PASSWORD settings, not DSN/connection-string overrides")
    env = os.environ.copy()
    # Both suites must use exactly the same server and credentials.
    for legacy, canonical, fallback in (
        ("ODBC_TEST_SERVER", "ODBC_SERVER", "127.0.0.1,1433"),
        ("ODBC_TEST_DATABASE", "ODBC_DATABASE", "tempdb"),
        ("ODBC_TEST_UID", "ODBC_USER", "sa"),
        ("ODBC_TEST_PWD", "ODBC_PASSWORD", ""),
    ):
        env[legacy] = env.get(canonical, fallback)
    env["ODBC_DRIVER"] = library
    env["ODBC_TEST_DRIVER"] = library
    env["ODBC_TEST_TARGET"] = driver
    env["ODBC_TEST_ENCRYPT"] = "yes"
    env["ODBC_TEST_TRUST_CERT"] = "yes"
    return env


def run_driver(driver, library, build, output, tests, exclusions, include_known):
    env = driver_environment(driver, library)
    directory = output / driver
    directory.mkdir(parents=True, exist_ok=True)
    names = {test["name"] for test in tests}
    omitted = {} if include_known else exclusions
    expected = names - omitted.keys()
    report = directory / "ctest.xml"
    if report.exists():
        report.unlink()
    for test in tests:
        for argument in test["command"]:
            if argument.startswith("--gtest_output=xml:"):
                Path(argument.removeprefix("--gtest_output=xml:")).unlink(missing_ok=True)
    command = ["ctest", "--test-dir", str(build), "--output-on-failure",
               "--no-tests=error", "--output-junit", str(report)]
    if omitted:
        command += ["-I", selection_indices(tests, omitted)]
    with (directory / "ctest.log").open("w") as log:
        result = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT,
                                timeout=1200, check=False)
    if not report.exists():
        raise RuntimeError(f"{driver}: CTest produced no XML (exit {result.returncode}); see {directory / 'ctest.log'}")
    results = read_results(report, expected)
    for name, issue in omitted.items():
        results[name] = {"status": "disabled", **issue}
    # Preserve GoogleTest properties/diagnostics before the other driver runs.
    xml_dir = directory / "gtest"
    xml_dir.mkdir(exist_ok=True)
    for test in tests:
        if test["name"] not in expected:
            continue
        for argument in test["command"]:
            if argument.startswith("--gtest_output=xml:"):
                source = Path(argument.removeprefix("--gtest_output=xml:"))
                if source.exists():
                    shutil.copyfile(source, xml_dir / source.name)
    failed = [name for name, case in results.items() if case["status"] in ("failed", "skipped")]
    print(f"{driver}: {len(expected) - len(failed)} passed, {len(failed)} failed/skipped, "
          f"{len(omitted)} disabled", flush=True)
    for name in failed:
        print(f"  {name}", flush=True)
    return {"library": library, "exit_code": result.returncode, "tests": results}


def audit_results(runs, exclusions):
    audit = {"recoveries": [], "expected_failures": [], "unexpected_failures": [],
             "infrastructure_errors": []}
    for driver in DRIVERS:
        run = runs[driver]
        if "error" in run:
            audit["infrastructure_errors"].append(f"{driver}: {run['error']}")
            continue
        cases = run["tests"]
        failed = any(case["status"] in ("failed", "skipped") for case in cases.values())
        if run["exit_code"] not in (0, 8) or (run["exit_code"] != 0 and not failed):
            audit["infrastructure_errors"].append(
                f"{driver}: unexpected CTest exit code {run['exit_code']}")
        for name, case in cases.items():
            known = driver == "mssql-rs" and name in exclusions[driver]
            if known and case["status"] == "passed":
                reference = runs["msodbcsql18"].get("tests", {}).get(name, {})
                audit["recoveries"].append({
                    "test": name, **exclusions[driver][name],
                    "reference_status": reference.get("status", "missing"),
                })
            elif known and case["status"] == "failed":
                audit["expected_failures"].append(name)
            elif case["status"] not in ("passed", "disabled"):
                audit["unexpected_failures"].append(
                    {"driver": driver, "test": name, "status": case["status"]})
    return audit


def write_comparison(output, runs, source, audit=None):
    names = sorted(set().union(*(run.get("tests", {}) for run in runs.values())))
    summary = ["# ODBC side-by-side comparison", "",
               f"mssql-rs source: `{source['revision']}`", "",
               "| Driver | Passed | Failed | Skipped | Disabled |",
               "| --- | ---: | ---: | ---: | ---: |"]
    for driver in DRIVERS:
        tests = runs[driver].get("tests", {})
        counts = {status: sum(test["status"] == status for test in tests.values())
                  for status in ("passed", "failed", "skipped", "disabled")}
        summary.append(f"| {driver} | " + " | ".join(str(n) for n in counts.values()) + " |")
        if "error" in runs[driver]:
            summary.extend(["", f"**{driver} infrastructure failure:** {runs[driver]['error']}"])
    if audit is not None:
        summary.extend(["", "## Upstream-main quarantine audit", "",
                        f"- Still failing as tracked: {len(audit['expected_failures'])}",
                        f"- Newly passing quarantined Rust cases: {len(audit['recoveries'])}",
                        f"- Unexpected failures/skips: {len(audit['unexpected_failures'])}",
                        f"- Infrastructure errors: {len(audit['infrastructure_errors'])}",
                        "", "New passes are candidate fixes, not automatic approval to remove "
                        "quarantines. Exact names and reference outcomes are in "
                        "`comparison.json` under `audit.recoveries`."])
        groups = {}
        for case in audit["recoveries"]:
            groups[case["issue"]] = groups.get(case["issue"], 0) + 1
        for issue, count in sorted(groups.items()):
            summary.append(f"- {issue}: {count} newly passing cases")
    with (output / "comparison.csv").open("w", newline="") as stream:
        writer = csv.writer(stream)
        writer.writerow(["test", *DRIVERS, "msodbcsql18_issue", "mssql_rs_issue"])
        for name in names:
            values = [runs[driver].get("tests", {}).get(name, {"status": "missing"}) for driver in DRIVERS]
            writer.writerow([name, *(v["status"] for v in values),
                             *(v.get("issue", "") for v in values)])
    summary.extend(["", "## Disabled cases", ""])
    for driver in DRIVERS:
        issues = {}
        for name, case in runs[driver].get("tests", {}).items():
            if case["status"] == "disabled":
                group = issues.setdefault(case["issue"], {"count": 0, "reason": case["reason"]})
                group["count"] += 1
        for issue, group in issues.items():
            summary.append(f"- **{driver}**: {group['count']} exact cases — {issue}: {group['reason']}")
    summary.extend(["", "Full per-test comparison: `comparison.csv`. "
                    "Per-driver CTest and GoogleTest XML/logs are included in the artifact."])
    (output / "summary.md").write_text("\n".join(summary) + "\n")
    report = {"source": source, "drivers": runs}
    if audit is not None:
        report["audit"] = audit
    (output / "comparison.json").write_text(json.dumps(report, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-dir", type=Path, required=True)
    parser.add_argument("--rust-driver", type=Path, required=True)
    parser.add_argument("--msodbcsql-driver", default="ODBC Driver 18 for SQL Server")
    parser.add_argument("--output-dir", type=Path, required=True)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--include-known-failures", action="store_true")
    mode.add_argument("--audit-known-failures", action="store_true",
                      help="Run quarantined Rust cases and classify recoveries against the registry")
    parser.add_argument("--source-metadata", type=Path, default=ROOT / "driver-source.json",
                        help="Metadata recording the actual checked-out Rust revision")
    args = parser.parse_args()
    build = args.build_dir.resolve()
    output = args.output_dir.resolve()
    if not args.rust_driver.is_file():
        parser.error("Rust driver library does not exist; build the pinned source first")
    discovery = subprocess.run(["ctest", "--test-dir", str(build), "--show-only=json-v1"],
                               text=True, capture_output=True, check=True)
    tests = json.loads(discovery.stdout)["tests"]
    names = {test["name"] for test in tests}
    if not names or len(names) != len(tests):
        parser.error("CTest inventory is empty or has duplicate test names")
    for test in tests:
        if any(p["name"] == "DISABLED" and p["value"] for p in test["properties"]):
            parser.error("Use driver-specific known-failures.json, not globally disabled cases")
    registry = load_exclusions(ROOT / "known-failures.json", names)
    source = json.loads(args.source_metadata.read_text())
    if args.audit_known_failures and (
            source.get("ref") != "main" or
            source.get("repository") != "https://github.com/microsoft/mssql-rs" or
            not re.fullmatch(r"[0-9a-f]{40}", source.get("revision", ""))):
        parser.error("Audit mode requires resolved mssql-rs main source metadata")
    output.mkdir(parents=True, exist_ok=True)
    runs = {}
    for driver, library in zip(DRIVERS, (args.msodbcsql_driver, str(args.rust_driver.resolve()))):
        try:
            runs[driver] = run_driver(driver, library, build, output, tests, registry[driver],
                                      args.include_known_failures or
                                      (args.audit_known_failures and driver == "mssql-rs"))
        except (RuntimeError, ValueError, OSError, ET.ParseError, subprocess.TimeoutExpired) as error:
            print(f"{driver}: {error}", file=sys.stderr, flush=True)
            runs[driver] = {"exit_code": 2, "error": str(error)}
    audit = audit_results(runs, registry) if args.audit_known_failures else None
    write_comparison(output, runs, source, audit)
    if audit is not None:
        return int(bool(audit["infrastructure_errors"] or audit["unexpected_failures"]))
    return int(any(run["exit_code"] != 0 or
                   any(case["status"] != "passed" and case["status"] != "disabled"
                       for case in run.get("tests", {}).values())
                   for run in runs.values()))


if __name__ == "__main__":
    sys.exit(main())
