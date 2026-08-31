#!/usr/bin/env python3
"""Mirror the repository's commit history into the graph.

`CLAUDE.md` §6 requires the graph to be updated on every commit and to identify,
for each change, the commit and its date. The graph held 23 `Commit` nodes out of
165 at 41051f8, so most of that trail existed only in `git log` -- and a
`gitCommit` stamp pointing at a commit with no node cannot be followed.

This tool registers every commit, links it to the files it touched
(`MODIFIES`), and to the `rmp` tasks its message closes (`CLOSES_TASK`). Task
references are read from the conventions the history actually uses -- `Task #N`,
`Tasks #N and #M`, `Closes rmp task #N` -- and never guessed from a bare number.

Usage:
    python3 tools/kg/commits_kg.py rucene [--since <rev>] [--limit N]
"""
import argparse
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from kgio import unwind as kg_unwind

SEP = "\x1e"
TASK_RE = re.compile(r"\btasks?\s+#?(\d+)(?:\s+and\s+#?(\d+))?", re.I)


def sh(*args):
    r = subprocess.run(args, capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"{' '.join(args)} failed: {r.stderr[:300]}")
    return r.stdout




def commits(limit=None):
    fmt = SEP.join(["%H", "%s", "%aI", "%B"]) + "\x1d"
    out = sh("git", "log", f"--format={fmt}")
    for rec in out.split("\x1d"):
        rec = rec.strip("\n")
        if not rec.strip():
            continue
        parts = rec.split(SEP)
        if len(parts) < 4:
            continue
        h, subject, date, body = parts[0], parts[1], parts[2], parts[3]
        tasks = set()
        for m in TASK_RE.finditer(body):
            for g in m.groups():
                if g:
                    tasks.add(int(g))
        yield {"hash": h, "message": subject, "date": date,
               "task_id": min(tasks) if tasks else None,
               "tasks": sorted(tasks)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("roadmap")
    ap.add_argument("--batch-size", type=int, default=100)
    args = ap.parse_args()

    rows = list(commits())
    print(f"commits: {len(rows)}, "
          f"{sum(1 for r in rows if r['tasks'])} referencing a task", file=sys.stderr)

    def unwind(mode, data, body, label):
        kg_unwind(args.roadmap, mode, data, body, label, args.batch_size)

    unwind("create", [{"hash": r["hash"]} for r in rows],
           "MERGE (c:Commit {hash: row.hash})", "Commit nodes")
    unwind("update",
           [{"hash": r["hash"], "message": r["message"], "date": r["date"],
             "task_id": r["task_id"], "gitDate": r["date"][:10]} for r in rows],
           "MATCH (c:Commit {hash: row.hash}) SET c.message = row.message, "
           "c.date = row.date, c.task_id = row.task_id, "
           "c.gitCommit = row.hash, c.gitDate = row.gitDate",
           "Commit properties")

    links = [{"hash": r["hash"], "task": t} for r in rows for t in r["tasks"]]
    unwind("create", links,
           "MATCH (c:Commit {hash: row.hash}) WITH c, row "
           "MATCH (t:Task {id: row.task}) MERGE (c)-[:CLOSES_TASK]->(t)",
           "CLOSES_TASK edges")
    unwind("update", links,
           "MATCH (c:Commit {hash: row.hash})-[e:CLOSES_TASK]->(t:Task {id: row.task}) "
           "SET e.gitCommit = row.hash",
           "CLOSES_TASK stamped")


if __name__ == "__main__":
    main()
