#!/usr/bin/env python3
"""Create or update sqldev issues for newly passing upstream-main quarantines."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

from compare import DRIVERS, ROOT, audit_results, load_exclusions


REPOSITORY = "saurabh500/sqldev"
END_MARKER = "<!-- /odbc-main-recovery -->"


def recovery_groups(report, registry_path):
    source = report["source"]
    if (source.get("repository") != "https://github.com/microsoft/mssql-rs" or
            source.get("ref") != "main" or
            not re.fullmatch(r"[0-9a-f]{40}", source.get("revision", ""))):
        raise ValueError("Recovery issues require a resolved upstream-main revision")
    if "audit" not in report:
        raise ValueError("Recovery issues require an audit-mode report")
    runs = report["drivers"]
    names = set(runs["mssql-rs"].get("tests", {}))
    for driver in DRIVERS:
        run = runs[driver]
        if "error" in run or set(run.get("tests", {})) != names:
            raise ValueError(f"Incomplete comparison for {driver}")
        if run["tests"].get("OdbcConformance.Connection", {}).get("status") != "passed":
            raise ValueError(f"Connection did not pass for {driver}")
        allowed = {"passed", "failed", "skipped"}
        if driver == "msodbcsql18":
            allowed.add("disabled")
        if any(case["status"] not in allowed for case in run["tests"].values()):
            raise ValueError(f"Unexpected test status in audit for {driver}")
    registry = load_exclusions(registry_path, names)
    audit = audit_results(runs, registry)
    if audit != report["audit"] or audit["infrastructure_errors"]:
        raise ValueError("Invalid or infrastructure-failed audit report")
    groups = {}
    for case in audit["recoveries"]:
        groups.setdefault(case["issue"], []).append(case)
    return groups


def issue_content(issue, cases, revision, run_url):
    number = issue.rsplit("/", 1)[-1]
    marker = f"<!-- odbc-main-recovery:{number} -->"
    fingerprint = hashlib.sha256(
        "\n".join(sorted(case["test"] for case in cases)).encode()).hexdigest()
    signature = f"<!-- recovered-cases:{fingerprint} -->"
    title = f"ODBC upstream main: review {len(cases)} newly passing cases from #{number}"
    body = [
        marker, signature, "## Upstream-main conformance divergence", "",
        f"**{len(cases)} quarantined mssql-rs cases now pass** in an upstream-main audit.",
        f"Original tracking issue: {issue}.",
        f"Upstream revision: [microsoft/mssql-rs@{revision[:12]}]"
        f"(https://github.com/microsoft/mssql-rs/commit/{revision}).",
        f"Evidence: {run_url} (artifact `odbc-conformance-comparison`).", "",
        "These are **candidate fixes**, not proof that every tracked case is resolved. "
        "A passing quarantined test can also reflect intermittent behavior or a parity-policy "
        "change. The source pin and exclusions have not been changed.", "",
        "## Passing cases", "",
        "| Exact test | Driver 18 outcome in the same run |", "| --- | --- |",
    ]
    for case in sorted(cases, key=lambda case: case["test"])[:50]:
        body.append(f"| `{case['test']}` | {case['reference_status']} |")
    if len(cases) > 50:
        body.extend(["", f"Showing 50 of {len(cases)} cases. The full exact list is in "
                     "`comparison/comparison.json` under `audit.recoveries`, filtered "
                     f"by original issue `{issue}`."])
    body.extend([
        "", "## Follow-up", "",
        "- Confirm the fix and stability by rerunning the representative cases.",
        "- Review any failed/disabled reference outcomes before calling this a parity fix.",
        "- Update `odbc-conformance/driver-source.json` to a verified upstream revision.",
        "- Remove only the resolved exact entries from `known-failures.json`.",
        "- Run the normal pinned comparison before merging.", "",
        "This section is maintained by upstream-main audits. One open notification is "
        "updated per original tracking issue; closing a notification suppresses the "
        "same set of recovered cases on subsequent runs.", END_MARKER,
    ])
    return title, "\n".join(body), marker, signature


def gh(arguments, body=None):
    result = subprocess.run(["gh", *arguments], input=body, text=True,
                            capture_output=True, check=True)
    return result.stdout


def publish(groups, revision, run_url):
    if not groups:
        print("No newly passing quarantined Rust cases; no issues to file.")
        return
    pages = json.loads(gh([
        "api", "--paginate", "--slurp",
        f"repos/{REPOSITORY}/issues?labels=odbc&state=all&per_page=100",
    ]))
    existing = [item for page in pages for item in page if "pull_request" not in item]
    for issue, cases in sorted(groups.items()):
        title, section, marker, signature = issue_content(issue, cases, revision, run_url)
        matches = [item for item in existing if marker in (item.get("body") or "")]
        opened = [item for item in matches if item["state"] == "open"]
        if len(opened) > 1:
            raise ValueError(f"Multiple open recovery notifications for {issue}")
        if opened:
            item = opened[0]
            body = item["body"]
            start = body.index(marker)
            end = body.find(END_MARKER, start)
            if end < 0:
                raise ValueError(f"Missing managed-section boundary in issue {item['number']}")
            body = body[:start] + section + body[end + len(END_MARKER):]
            if body != item["body"] or item["title"] != title:
                gh(["issue", "edit", str(item["number"]), "--repo", REPOSITORY,
                    "--title", title, "--body-file", "-"], body)
            print(f"Recovery tracked in {item['html_url']}")
        elif any(signature in (item.get("body") or "") for item in matches):
            print(f"Previously closed notification already covers these cases for {issue}")
        else:
            url = gh(["issue", "create", "--repo", REPOSITORY, "--label", "odbc",
                      "--title", title, "--body-file", "-"], section).strip()
            print(f"Created recovery notification: {url}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--registry", type=Path, default=ROOT / "known-failures.json")
    parser.add_argument("--run-url", required=True)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    if not re.fullmatch(r"https://github\.com/saurabh500/sqldev/actions/runs/[1-9][0-9]*",
                        args.run_url):
        parser.error("Expected a sqldev Actions run URL")
    report = json.loads(args.report.read_text())
    groups = recovery_groups(report, args.registry)
    if args.dry_run:
        for issue, cases in sorted(groups.items()):
            title, section, _, _ = issue_content(
                issue, cases, report["source"]["revision"], args.run_url)
            print(f"{title}\n{section}\n")
        print(f"Dry run: {len(groups)} recovery issue groups; no GitHub changes.")
    else:
        publish(groups, report["source"]["revision"], args.run_url)


if __name__ == "__main__":
    main()
