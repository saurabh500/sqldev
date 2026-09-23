#!/usr/bin/env python3
"""Record reviewed failed cases with an existing issue or create a follow-up issue."""

import argparse
import json
from pathlib import Path
import re
import subprocess

from compare import DRIVERS, ROOT, load_exclusions


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--driver", choices=DRIVERS, required=True)
    parser.add_argument("--match", required=True, help="Regex selecting reviewed failed cases")
    parser.add_argument("--reason", required=True)
    issue = parser.add_mutually_exclusive_group(required=True)
    issue.add_argument("--issue", type=int, help="Existing issue number in saurabh500/sqldev")
    issue.add_argument("--create-issue", help="Title for a new odbc-labeled follow-up issue")
    args = parser.parse_args()
    report = json.loads(args.report.read_text())
    run = report["drivers"][args.driver]
    if "error" in run:
        parser.error("Infrastructure failures cannot be quarantined")
    cases = run["tests"]
    if cases.get("OdbcConformance.Connection", {}).get("status") != "passed":
        parser.error("Driver connection must pass before any cases can be quarantined")
    selected = sorted(name for name, case in cases.items()
                      if case["status"] == "failed" and re.search(args.match, name))
    if not selected:
        parser.error("No failed tests match the selection")
    if "OdbcConformance.Connection" in selected:
        parser.error("Readiness cannot be quarantined")
    path = ROOT / "known-failures.json"
    existing = load_exclusions(path, set(cases))[args.driver]
    if any(name in existing for name in selected):
        parser.error("A selected test is already quarantined")
    if args.issue is not None:
        result = subprocess.run(
            ["gh", "issue", "view", str(args.issue), "--repo", "saurabh500/sqldev",
             "--json", "url,state"], capture_output=True, text=True, check=True)
        metadata = json.loads(result.stdout)
        if metadata["state"] != "OPEN":
            parser.error("The follow-up issue must be open")
        url = metadata["url"]
    else:
        body = (f"Driver: **{args.driver}**\n\n{args.reason}\n\n"
                f"Rust source revision: `{report['source']['revision']}`\n\n"
                "These cases failed in the side-by-side run. Their assertions remain intact; "
                "only these exact names are excluded for the affected driver until resolved.\n\n"
                "## Affected cases\n\n" + "\n".join(f"- `{name}`" for name in selected) +
                "\n\n## Follow-up\n\n- [ ] Confirm cause and upstream disposition.\n"
                "- [ ] Rerun with `compare.py --include-known-failures`.\n"
                "- [ ] Remove these entries from `known-failures.json` when resolved.\n")
        # GitHub issue bodies have a finite size; the manifest retains the complete inventory.
        if len(body) > 60000:
            body = (f"Driver: **{args.driver}**\n\n{args.reason}\n\n"
                    f"Rust source revision: `{report['source']['revision']}`\n\n"
                    f"Affects {len(selected)} exact parameterized cases. Full inventory is in "
                    "`odbc-conformance/known-failures.json` in PR #51.\n\n"
                    "Representative cases:\n" + "\n".join(f"- `{n}`" for n in selected[:20]) +
                    "\n\nPreserve assertions; re-enable each case after validation with "
                    "`compare.py --include-known-failures`.\n")
        result = subprocess.run(
            ["gh", "issue", "create", "--repo", "saurabh500/sqldev", "--label", "odbc",
             "--title", args.create_issue, "--body-file", "-"],
            input=body, capture_output=True, text=True, check=True)
        url = result.stdout.strip()
    if not re.fullmatch(r"https://github\.com/saurabh500/sqldev/issues/[1-9][0-9]*", url):
        raise ValueError("GitHub did not return a valid repository issue URL")
    registry = json.loads(path.read_text())
    registry[args.driver].append({"issue": url, "reason": args.reason, "tests": selected})
    path.write_text(json.dumps(registry, indent=2) + "\n")
    print(f"{url}: disabled {len(selected)} exact cases for {args.driver}")


if __name__ == "__main__":
    main()
