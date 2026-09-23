import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import compare


class ComparisonTests(unittest.TestCase):
    def audit_fixture(self, status="failed"):
        runs = {
            driver: {"exit_code": 0, "tests": {
                "OdbcConformance.Connection": {"status": "passed"},
                "Suite.Known": {"status": "passed"},
                "Suite.Other": {"status": "passed"},
            }} for driver in compare.DRIVERS
        }
        runs["mssql-rs"]["tests"]["Suite.Known"]["status"] = status
        if status == "failed":
            runs["mssql-rs"]["exit_code"] = 8
        registry = {"msodbcsql18": {}, "mssql-rs": {
            "Suite.Known": {"issue": "https://github.com/saurabh500/sqldev/issues/53",
                            "reason": "Known gap"}
        }}
        return runs, registry

    def test_audit_keeps_known_failures_but_reports_recoveries(self):
        runs, registry = self.audit_fixture()
        audit = compare.audit_results(runs, registry)
        self.assertEqual(["Suite.Known"], audit["expected_failures"])
        self.assertEqual([], audit["unexpected_failures"])
        self.assertEqual([], audit["infrastructure_errors"])
        runs["mssql-rs"]["tests"]["Suite.Known"]["status"] = "passed"
        runs["mssql-rs"]["exit_code"] = 0
        recovered = compare.audit_results(runs, registry)["recoveries"]
        self.assertEqual("Suite.Known", recovered[0]["test"])
        self.assertEqual("passed", recovered[0]["reference_status"])

    def test_audit_skips_and_new_failures_are_not_expected(self):
        runs, registry = self.audit_fixture("skipped")
        runs["mssql-rs"]["tests"]["Suite.Other"]["status"] = "failed"
        runs["msodbcsql18"]["tests"]["Suite.Other"]["status"] = "failed"
        audit = compare.audit_results(runs, registry)
        self.assertEqual(3, len(audit["unexpected_failures"]))
        self.assertEqual([], audit["expected_failures"])

    def test_audit_preserves_disabled_reference_outcome(self):
        runs, registry = self.audit_fixture("passed")
        runs["msodbcsql18"]["tests"]["Suite.Known"]["status"] = "disabled"
        audit = compare.audit_results(runs, registry)
        self.assertEqual("disabled", audit["recoveries"][0]["reference_status"])

    def test_audit_detects_infrastructure_errors(self):
        runs, registry = self.audit_fixture()
        runs["mssql-rs"] = {"exit_code": 2, "error": "Missing XML"}
        self.assertEqual(1, len(compare.audit_results(runs, registry)["infrastructure_errors"]))
        for code in (8, 137):
            runs, registry = self.audit_fixture("passed")
            runs["mssql-rs"]["exit_code"] = code
            self.assertEqual(1, len(compare.audit_results(runs, registry)["infrastructure_errors"]))

    def test_audit_main_runs_only_rust_quarantines_and_records_actual_revision(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "driver.so").touch()
            source = {"repository": "https://github.com/microsoft/mssql-rs",
                      "revision": "a" * 40, "ref": "main"}
            (root / "driver-source.json").write_text(json.dumps(source))
            (root / "known-failures.json").write_text(json.dumps({
                "msodbcsql18": [], "mssql-rs": [{
                    "issue": "https://github.com/saurabh500/sqldev/issues/53",
                    "reason": "Known gap", "tests": ["Suite.Known"],
                }]
            }))
            runs, _ = self.audit_fixture()
            discovery = {"tests": [{"name": name, "properties": []}
                                   for name in runs["mssql-rs"]["tests"]]}
            argv = ["compare.py", "--build-dir", directory, "--rust-driver",
                    str(root / "driver.so"), "--output-dir", str(root / "out"),
                    "--audit-known-failures"]
            with patch.object(compare, "ROOT", root), patch("sys.argv", argv), \
                    patch.object(compare.subprocess, "run", return_value=subprocess.CompletedProcess(
                        [], 0, json.dumps(discovery))), \
                    patch.object(compare, "run_driver",
                                 side_effect=lambda driver, *args: runs[driver]) as runner:
                self.assertEqual(0, compare.main())
            self.assertEqual([False, True], [call.args[-1] for call in runner.call_args_list])
            result = json.loads((root / "out/comparison.json").read_text())
            self.assertEqual(source, result["source"])
            self.assertEqual(["Suite.Known"], result["audit"]["expected_failures"])
            self.assertEqual(source, json.loads((root / "driver-source.json").read_text()))

    def test_selection_excludes_only_exact_case(self):
        tests = [{"name": name} for name in ("Suite.Case/Path", "SuiteXCase/Path",
                                            "Suite.Case/PathExtra")]
        self.assertEqual("0,0,1,2,3", compare.selection_indices(tests, {"Suite.Case/Path"}))

    def test_registry_requires_issue_and_live_name(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "known.json"
            item = {"issue": "https://github.com/saurabh500/sqldev/issues/52", "reason": "Tracked",
                    "tests": ["Suite.Case"]}
            registry = {"msodbcsql18": [item], "mssql-rs": []}
            path.write_text(json.dumps(registry))
            self.assertEqual(item["issue"], compare.load_exclusions(
                path, {"Suite.Case", "Suite.Other"})["msodbcsql18"]["Suite.Case"]["issue"])
            with self.assertRaises(ValueError):
                compare.load_exclusions(path, {"Suite.Other"})
            item["issue"] = ""
            path.write_text(json.dumps(registry))
            with self.assertRaises(ValueError):
                compare.load_exclusions(path, {"Suite.Case", "Suite.Other"})

    def test_connection_cannot_be_disabled(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "known.json"
            path.write_text(json.dumps({
                "msodbcsql18": [{
                    "issue": "https://github.com/saurabh500/sqldev/issues/52", "reason": "No",
                    "tests": ["OdbcConformance.Connection"]
                }], "mssql-rs": []
            }))
            with self.assertRaises(ValueError):
                compare.load_exclusions(path, {"OdbcConformance.Connection", "Other.Test"})

    def test_parse_success_failure_skip_and_missing(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "results.xml"
            path.write_text('<testsuite><testcase name="Pass"/><testcase name="Fail">'
                            '<failure>assertion</failure></testcase>'
                            '<testcase name="Skip"><skipped/></testcase></testsuite>')
            cases = compare.read_results(path, {"Pass", "Fail", "Skip"})
            self.assertEqual(["passed", "failed", "skipped"], [c["status"] for c in cases.values()])
            with self.assertRaises(ValueError):
                compare.read_results(path, {"Pass", "Fail", "Skip", "Missing"})

    def test_environment_selects_driver_and_shares_credentials(self):
        with patch.dict(compare.os.environ, {"ODBC_USER": "test", "ODBC_PASSWORD": "value",
                                            "ODBC_TEST_UID": "different"}, clear=True):
            env = compare.driver_environment("mssql-rs", "/driver.so")
            self.assertEqual("/driver.so", env["ODBC_DRIVER"])
            self.assertEqual("/driver.so", env["ODBC_TEST_DRIVER"])
            self.assertEqual("test", env["ODBC_TEST_UID"])
            self.assertEqual("mssql-rs", env["ODBC_TEST_TARGET"])

    def test_connection_override_is_rejected(self):
        with patch.dict(compare.os.environ, {"ODBC_CONNECTION_STRING": "DRIVER=other"}, clear=True):
            with self.assertRaises(ValueError):
                compare.driver_environment("mssql-rs", "/driver.so")

    def test_comparison_preserves_driver_specific_disabling(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            runs = {
                "msodbcsql18": {"exit_code": 0, "tests": {"Suite.Case": {
                    "status": "disabled", "issue": "https://github.com/saurabh500/sqldev/issues/52",
                    "reason": "Known"
                }}},
                "mssql-rs": {"exit_code": 0, "tests": {"Suite.Case": {"status": "passed"}}},
            }
            compare.write_comparison(output, runs, {"revision": "abc"})
            self.assertIn("Suite.Case,disabled,passed", (output / "comparison.csv").read_text())
            self.assertIn("issues/52", (output / "summary.md").read_text())


if __name__ == "__main__":
    unittest.main()
