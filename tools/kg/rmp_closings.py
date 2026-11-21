#!/usr/bin/env python3
"""Export `rmp`'s closings, `[{task, commit_close}]`, for `commits_kg.py --closings`.

`rmp` is the authority on which commit closed a task. `rmp task list` returns at
most 100 tasks per call, so the export reads every task through
`port_coverage_kg.fetch_tasks`, which walks the project's dates one day and one
status at a time.

    python3 tools/kg/rmp_closings.py rucene <out.json>
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from port_coverage_kg import fetch_tasks  # noqa: E402


def closings(tasks):
    """The `{task, commit_close}` rows of the tasks that carry a closing commit."""
    return sorted(({"task": int(t["id"]), "commit_close": t["commit_close"]}
                   for t in tasks if t.get("commit_close")), key=lambda r: r["task"])


def main():
    if len(sys.argv) != 3:
        raise SystemExit("usage: rmp_closings.py <roadmap> <out.json>")
    rows = closings(fetch_tasks(sys.argv[1]))
    if not rows:
        raise SystemExit("rmp reported no closing at all: refusing to write an empty export")
    Path(sys.argv[2]).write_text(json.dumps(rows), encoding="utf-8")
    print(f"closings: {len(rows)} tasks -> {sys.argv[2]}", file=sys.stderr)


if __name__ == "__main__":
    main()
