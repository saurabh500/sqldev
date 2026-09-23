import contextlib
import copy
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import compare
import report_recoveries as reporter


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.registry_path = Path(self.directory.name) / "known.json"
        self.issue = "https://github.com/saurabh500/sqldev/issues/53"
        self.registry_path.write_text(json.dumps({
            "msodbcsql18": [], "mssql-rs": [
                {"issue": self.issue, "reason": "Known gap", "tests": ["Suite.Known"]},
            ],
        }))
        self.report = {
            "source": {"repository": "https://github.com/microsoft/mssql-rs",
                       "ref": "main", "revision": "a" * 40},
            "drivers": {
                driver: {"exit_code": 0, "tests": {
                    "OdbcConformance.Connection": {"status": "passed"},
                    "Suite.Known": {"status": "passed"},
                    "Suite.Other": {"status": "passed"},
                }} for driver in compare.DRIVERS
            },
        }
        self.refresh_audit()
        self.run_url = "https://github.com/saurabh500/sqldev/actions/runs/123"

    def refresh_audit(self):
        names = set(self.report["drivers"]["mssql-rs"]["tests"])
        registry = compare.load_exclusions(self.registry_path, names)
        self.report["audit"] = compare.audit_results(self.report["drivers"], registry)

    def groups(self):
        return reporter.recovery_groups(self.report, self.registry_path)

    def existing(self, state="open"):
        cases = self.groups()[self.issue]
        title, body, _, _ = reporter.issue_content(
            self.issue, cases, "a" * 40, self.run_url)
        return {"number": 80, "body": body, "title": title, "state": state,
                "html_url": "https://github.com/saurabh500/sqldev/issues/80"}

    def publish(self, pages, *responses):
        with patch.object(reporter, "gh", side_effect=[json.dumps(pages), *responses]) as gh, \
                contextlib.redirect_stdout(io.StringIO()):
            reporter.publish(self.groups(), "b" * 40, self.run_url)
        return gh

    def test_detects_only_registered_passing_cases(self):
        self.assertEqual(["Suite.Known"],
                         [case["test"] for case in self.groups()[self.issue]])
        self.report["drivers"]["mssql-rs"]["tests"]["Suite.Known"]["status"] = "failed"
        self.report["drivers"]["mssql-rs"]["exit_code"] = 8
        self.refresh_audit()
        self.assertEqual({}, self.groups())

    def test_refuses_pinned_or_incomplete_or_failed_connection_reports(self):
        original = copy.deepcopy(self.report)
        mutations = [
            lambda r: r["source"].update(ref="pinned"),
            lambda r: r["source"].update(revision="main"),
            lambda r: r.pop("audit"),
            lambda r: r["drivers"]["mssql-rs"].update(error="Missing XML"),
            lambda r: r["drivers"]["msodbcsql18"]["tests"].pop("Suite.Other"),
            lambda r: r["drivers"]["mssql-rs"]["tests"]["OdbcConformance.Connection"].update(
                status="failed"),
            lambda r: r["drivers"]["mssql-rs"]["tests"]["Suite.Known"].update(status="disabled"),
            lambda r: r["audit"].update(recoveries=[]),
        ]
        for mutate in mutations:
            self.report = copy.deepcopy(original)
            mutate(self.report)
            with self.subTest(mutate=mutate), self.assertRaises(ValueError):
                self.groups()

    def test_refuses_infrastructure_exit_even_with_passing_tests(self):
        self.report["drivers"]["mssql-rs"]["exit_code"] = 137
        self.refresh_audit()
        with self.assertRaises(ValueError):
            self.groups()

    def test_creates_labeled_issue(self):
        gh = self.publish([[]], "https://github.com/saurabh500/sqldev/issues/80")
        self.assertIn("--paginate", gh.call_args_list[0].args[0])
        self.assertIn("create", gh.call_args.args[0])
        self.assertIn("odbc", gh.call_args.args[0])
        self.assertIn("Suite.Known", gh.call_args.args[1])
        self.assertIn("candidate fixes", gh.call_args.args[1])

    def test_updates_one_open_issue_and_preserves_human_text(self):
        item = self.existing()
        item["body"] = "Human notes\n" + item["body"] + "\nOther notes"
        gh = self.publish([[item]], "")
        self.assertIn("edit", gh.call_args.args[0])
        self.assertIn("80", gh.call_args.args[0])
        self.assertTrue(gh.call_args.args[1].startswith("Human notes\n"))
        self.assertTrue(gh.call_args.args[1].endswith("\nOther notes"))
        self.assertIn("b" * 40, gh.call_args.args[1])

    def test_closed_issue_suppresses_same_case_set_across_revisions(self):
        gh = self.publish([[self.existing("closed")]])
        self.assertEqual(1, gh.call_count)

    def test_closed_issue_allows_different_case_set(self):
        item = self.existing("closed")
        item["body"] = item["body"].replace("<!-- recovered-cases:", "<!-- old-cases:")
        gh = self.publish([[item]], "https://github.com/saurabh500/sqldev/issues/81")
        self.assertIn("create", gh.call_args.args[0])

    def test_identical_rerun_does_not_update_issue(self):
        item = self.existing()
        with patch.object(reporter, "gh", return_value=json.dumps([[item]])) as gh, \
                contextlib.redirect_stdout(io.StringIO()):
            reporter.publish(self.groups(), "a" * 40, self.run_url)
        self.assertEqual(1, gh.call_count)

    def test_no_recoveries_never_calls_github(self):
        with patch.object(reporter, "gh") as gh, contextlib.redirect_stdout(io.StringIO()):
            reporter.publish({}, "a" * 40, self.run_url)
        gh.assert_not_called()

    def test_issue_body_bounds_large_groups(self):
        cases = [{"test": f"Suite.Case{i}", "reference_status": "passed"} for i in range(700)]
        _, body, _, signature = reporter.issue_content(self.issue, cases, "a" * 40, self.run_url)
        self.assertIn("Showing 50 of 700", body)
        self.assertLess(len(body), 60000)
        self.assertIn(signature, body)

    def test_manual_dry_run_never_calls_github(self):
        path = Path(self.directory.name) / "report.json"
        path.write_text(json.dumps(self.report))
        with patch("sys.argv", ["report_recoveries.py", "--report", str(path),
                               "--registry", str(self.registry_path),
                               "--run-url", self.run_url, "--dry-run"]), \
                patch.object(reporter, "gh") as gh, contextlib.redirect_stdout(io.StringIO()):
            reporter.main()
        gh.assert_not_called()


if __name__ == "__main__":
    unittest.main()
