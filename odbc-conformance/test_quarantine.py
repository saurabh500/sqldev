import contextlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import quarantine


class QuarantineTests(unittest.TestCase):
    def invoke(self, driver_run, arguments, gh_result=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            registry = {"msodbcsql18": [], "mssql-rs": []}
            path = root / "known-failures.json"
            path.write_text(json.dumps(registry))
            report = root / "comparison.json"
            report.write_text(json.dumps({
                "source": {"revision": "abc"},
                "drivers": {"mssql-rs": driver_run},
            }))
            argv = ["quarantine.py", "--report", str(report), "--driver", "mssql-rs",
                    "--match", "^Suite.Case$", "--reason", "Reviewed", *arguments]
            with patch.object(quarantine, "ROOT", root), patch("sys.argv", argv), \
                    patch.object(quarantine.subprocess, "run", return_value=gh_result) as gh, \
                    contextlib.redirect_stdout(io.StringIO()), \
                    contextlib.redirect_stderr(io.StringIO()):
                try:
                    quarantine.main()
                except SystemExit as error:
                    self.assertEqual(2, error.code)
                    self.assertEqual(registry, json.loads(path.read_text()))
                    gh.assert_not_called()
                    return None
                return json.loads(path.read_text()), gh.call_args

    def test_records_only_selected_failure_for_one_driver(self):
        run = {"tests": {"OdbcConformance.Connection": {"status": "passed"},
                         "Suite.Case": {"status": "failed"}, "Suite.Other": {"status": "passed"}}}
        result = subprocess.CompletedProcess([], 0, json.dumps({
            "url": "https://github.com/saurabh500/sqldev/issues/52", "state": "OPEN"
        }))
        registry, _ = self.invoke(run, ["--issue", "52"], result)
        self.assertEqual([], registry["msodbcsql18"])
        self.assertEqual(["Suite.Case"], registry["mssql-rs"][0]["tests"])

    def test_creates_labeled_issue_before_recording(self):
        run = {"tests": {"OdbcConformance.Connection": {"status": "passed"},
                         "Suite.Case": {"status": "failed"}}}
        result = subprocess.CompletedProcess([], 0, "https://github.com/saurabh500/sqldev/issues/99\n")
        registry, call = self.invoke(run, ["--create-issue", "Tracked failure"], result)
        self.assertIn("odbc", call.args[0])
        self.assertIn("issue", call.args[0])
        self.assertIn("Suite.Case", call.kwargs["input"])
        self.assertEqual("https://github.com/saurabh500/sqldev/issues/99",
                         registry["mssql-rs"][0]["issue"])

    def test_refuses_infrastructure_failures(self):
        self.assertIsNone(self.invoke({"error": "Missing results"}, ["--issue", "52"]))

    def test_refuses_connection_failures(self):
        self.assertIsNone(self.invoke({"tests": {
            "OdbcConformance.Connection": {"status": "failed"}, "Suite.Case": {"status": "failed"}
        }}, ["--issue", "52"]))

    def test_refuses_disabling_passing_cases(self):
        self.assertIsNone(self.invoke({"tests": {
            "OdbcConformance.Connection": {"status": "passed"}, "Suite.Case": {"status": "passed"}
        }}, ["--issue", "52"]))


if __name__ == "__main__":
    unittest.main()
