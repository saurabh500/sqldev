import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import compare


class ComparisonTests(unittest.TestCase):
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
