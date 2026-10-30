#!/usr/bin/env python3
"""Mirror the repository's commit history into the graph.

`CLAUDE.md` §6 requires the graph to be updated on every commit and to identify,
for each change, the commit and its date. The graph held 23 `Commit` nodes out of
165 at 41051f8, so most of that trail existed only in `git log` -- and a
`gitCommit` stamp pointing at a commit with no node cannot be followed.

This tool registers every commit and writes three edges, each as
`knowledge-model.md` defines it:

* `COMMITTED_IN` (`File` -> `Commit`): every file the commit changed, matched by
  `File.path`. A path with no `File` node gets no edge (the tool creates no
  `File` nodes), and a merge commit lists no files (`git log` shows no diff for
  one), so its changes are linked through the commits it merges.
* `CLOSES_TASK` (`Commit` -> `Task`): the commit that closed the task.
* `REFERENCES_TASK` (`Commit` -> `Task`): the tasks a `Refs` footer names,
  which the commit contributes to without closing. A task the same commit also
  closes gets only `CLOSES_TASK`.

**Which commit closed a task.** `rmp` is the source of truth. `--closings FILE`
takes its export -- a JSON list of `{"task": N, "commit_close": "<hash>"}`, the
hash abbreviated or full -- and each hash is resolved with `git rev-parse`. A
malformed file (not JSON, not a list of `{task, commit_close}`, a task listed
twice) stops the tool. An entry whose hash is not hex or names no single commit
is printed as `BAD_CLOSING task=N hash=H reason=...`, appended to
`--bad-closings-report`, and treated as absent from the file. Every task
in that file is closed by exactly the commit the file names, whatever any
message says; where a message claims a different closing commit for such a task,
the disagreement is printed on stderr. The tool never calls `rmp` itself.

A task is closed once. For a task the file does not list, when several commits'
messages claim to close it, only the newest commit keeps `CLOSES_TASK`; each
older claim is printed as `SUPERSEDED_MESSAGE_CLOSING`. In the graph, a
`CLOSES_TASK` a task holds from any commit other than its closer (the file's,
or the newest message's) is deleted and printed as `RETIRED CLOSES_TASK`.

A sync leaves only facts it derives: for every commit in the processed range,
a `CLOSES_TASK` or `REFERENCES_TASK` edge this run does not derive is deleted
and printed as `RETIRED <edge> <hash> -> task #N: not derived`. A `--since` run
touches no commit outside its range.

The messages decide only for a task the closings file does not list (the tasks
closed before `rmp` recorded hashes, or every task when no file is given). A
task number is then read from:

* a `Closes` / `Fixes` footer at the start of a line, optionally followed by
  `rmp` and `task(s)` and a colon: `Fixes #192, #209`, `Closes rmp task #18.`;
* a `Task: #15` trailer: a line holding only `Task:` and the list;
* the task tag of the **first body line**, followed by `:`, `.`, `,` or the end
  of the line, where a comma must not open a relative clause:
  `tasks #202, #203 and #204. ...`, `task #200, acceptance criterion 3.`, but
  not `Task #12, which ...`; a line-start tag further down the body is prose;
* a task tag anywhere in the **subject line**, which is a title and not prose:
  `Task #8: ...`, `Task 99: ...`, `Tasks #82 and #83: ...`,
  `fix(index): task 93 SegmentReader ...`, `Sprint 1 Task #2: ...`,
  `Task #74 checkpoint: ...`. The singular `task` takes one number; the plural
  takes `#` numbers or a list of two or more, so `tasks 3 ways` is not a tag.

A number ends at a word boundary: `task 10x faster` and `tasks 2.5` are not
tags, while `task #201.` is.

Body prose never counts (`now task #205: ...`, `Task #161's triage ...`), nor
does a bare number. `Refs` footers (`Refs #192, #209`) give `REFERENCES_TASK`.
A list is read whole: `#1, #2, #3 and #4` yields all four.

`--since <commit>` mirrors only the commits after `<commit>`
(`git log <commit>..HEAD`), for all three edges; a closing in the closings file
is written only when its commit is in that range.

Usage:
    python3 tools/kg/commits_kg.py rucene [--since <commit>] [--closings FILE]
"""
import argparse
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from kgio import esc, kg_target, read_dicts as kg_read_dicts, unwind as kg_unwind

SEP = "\x1e"
REC = "\x1d"

# `#1`, `#1, #2`, `#1 and #2`, `#1, #2, and #3`.
LIST = r"#\d+(?:[ \t]*(?:,[ \t]*(?:and[ \t]+)?|and[ \t]+)#\d+)*"
FOOTER_RE = re.compile(
    rf"^[ \t]*(refs|closes|fixes)(?:[ \t]+rmp)?(?:[ \t]+tasks?)?[ \t]*:?[ \t]*({LIST})",
    re.I | re.M,
)
# The early subjects wrote the tag without `#` (`Task 99: port ...`), so the
# `#` is optional here -- and only here, where `Task` and the trailing
# punctuation already make the number a task id.
#
# A number ends at a boundary: `task 10x faster`, `tasks 3ways` and `tasks 2.5`
# are not task ids, while `task #201.` (a sentence-ending period) is.
TAG_NUM = r"#?\d+(?!\w|\.\d)"
TAG_LIST = rf"{TAG_NUM}(?:[ \t]*(?:,[ \t]*(?:and[ \t]+)?|and[ \t]+){TAG_NUM})*"
# The line-start tag of the body. The history writes it only as the *first* body
# line (`tasks #202, #203 and #204. The knowledge graph ...`), so only that line
# is read: a line-start `Task #N` further down is prose. The list must end in
# `:`, `.`, `,` or the end of the line, and a comma must not open a relative
# clause (`Task #12, which ...`), which makes the line a sentence about the task.
TAG_RE = re.compile(
    rf"^[ \t]*tasks?[ \t]*:?[ \t]*({TAG_LIST})"
    r"(?=[ \t]*(?:[:.]|,(?![ \t]*(?:which|who|whose|whom|where|that)\b)|$))",
    re.I,
)
NUM_RE = re.compile(r"#(\d+)")
TAG_NUM_RE = re.compile(r"#?(\d+)")
CLOSING_KEYWORDS = {"closes", "fixes"}


# The subject tag needs no trailing punctuation (`fix(index): task 93
# SegmentReader ...`), so it is narrower on number form instead: the singular
# `task` takes one number, `#` or not; the plural `tasks` takes `#` numbers or a
# list of at least two, so `tasks 3 ways` is not a tag.
TAG_SEP = r"[ \t]*(?:,[ \t]*(?:and[ \t]+)?|and[ \t]+)"
HASH_NUM = r"#\d+(?!\w|\.\d)"
# A `Task: #15` git trailer: the colon is required and the line holds nothing
# else, which is what makes it a footer rather than a sentence.
TRAILER_RE = re.compile(rf"^[ \t]*tasks?[ \t]*:[ \t]*({LIST})[ \t]*$", re.I | re.M)
SUBJECT_TAG_RE = re.compile(
    rf"\b(?:task(?!s)[ \t]*:?[ \t]*({TAG_LIST})"
    rf"|tasks[ \t]*:?[ \t]*({HASH_NUM}(?:{TAG_SEP}{TAG_NUM})*|{TAG_NUM}(?:{TAG_SEP}{TAG_NUM})+))",
    re.I,
)
HEX_RE = re.compile(r"^[0-9a-f]{7,40}$")


def parse_tasks(message):
    """`(closes, refs)` the message alone states: sorted task ids it closes, and
    those only a `Refs` footer names (a task that is also closed is not
    repeated). The first line of `message` is the subject."""
    closes, refs = set(), set()
    for m in FOOTER_RE.finditer(message):
        ids = {int(n) for n in NUM_RE.findall(m.group(2))}
        (closes if m.group(1).lower() in CLOSING_KEYWORDS else refs).update(ids)
    for m in TRAILER_RE.finditer(message):
        closes.update(int(n) for n in NUM_RE.findall(m.group(1)))
    lines = message.split("\n")
    subject = lines[0]
    first_body = next((ln for ln in lines[1:] if ln.strip()), "")
    m = TAG_RE.match(first_body)
    if m:
        closes.update(int(n) for n in TAG_NUM_RE.findall(m.group(1)))
    for m in SUBJECT_TAG_RE.finditer(subject):
        closes.update(int(n) for n in TAG_NUM_RE.findall(m.group(1) or m.group(2)))
    return sorted(closes), sorted(refs - closes)


def load_closings(path):
    """`(closings, bad)` from an `rmp` closings export.

    `closings` maps each task to the full hash its `commit_close` resolves to.
    A file that is not JSON, is not a list of `{"task": <int>, "commit_close":
    <str>}`, or lists a task twice is malformed and stops the tool. An entry
    whose hash is not 7-40 hex digits, or names no single commit of this
    repository, is returned in `bad` (`task`, `hash`, `reason`) and left out of
    `closings`, so that task falls back to its commit messages.
    """
    import json
    try:
        data = json.loads(Path(path).read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise SystemExit(f"{path}: not JSON: {exc}") from exc
    if not isinstance(data, list):
        raise SystemExit(f"{path}: expected a JSON list of closings")
    malformed, seen = [], set()
    for i, e in enumerate(data):
        task = e.get("task") if isinstance(e, dict) else None
        h = e.get("commit_close") if isinstance(e, dict) else None
        if not isinstance(task, int) or isinstance(task, bool) or not isinstance(h, str):
            malformed.append(f"entry {i}: {e!r} is not {{task: int, commit_close: str}}")
        elif task in seen:
            malformed.append(f"entry {i}: task {task} listed twice")
        seen.add(task)
    if malformed:
        raise SystemExit(f"{path}: malformed closings file:\n  " + "\n  ".join(malformed))
    out, bad = {}, []
    for e in data:
        task, h = e["task"], e["commit_close"]
        if not HEX_RE.match(h):
            bad.append({"task": task, "hash": h, "reason": "not 7-40 lowercase hex digits"})
            continue
        r = subprocess.run(["git", "rev-parse", "--verify", "--quiet", f"{h}^{{commit}}"],
                           capture_output=True, text=True)
        if r.returncode != 0 or not r.stdout.strip():
            bad.append({"task": task, "hash": h,
                        "reason": "names no commit, or more than one, in this repository"})
            continue
        out[task] = r.stdout.strip()
    return out, bad


def write_bad_closings(path, bad):
    """Print each bad closing on stderr and append it to `path` as JSON lines."""
    import json
    for b in bad:
        print(f"  BAD_CLOSING task={b['task']} hash={b['hash']} reason={b['reason']}",
              file=sys.stderr)
    if bad:
        out = Path(path)
        out.parent.mkdir(parents=True, exist_ok=True)
        with out.open("a", encoding="utf-8") as fh:
            for b in bad:
                fh.write(json.dumps(b, sort_keys=True) + "\n")


def classify(rows, closings):
    """Apply the closings file to the message-derived rows.

    `rows` carry `hash`, `msg_closes` and `msg_refs`; each gets `tasks` (what it
    closes), `refs` and `task_id`. Returns the disagreements: `(task, file hash,
    message hash)` for each message that claims a closing of a task the file
    assigns to another commit.
    """
    by_commit = {}
    for task, h in closings.items():
        by_commit.setdefault(h, set()).add(task)
    disagreements = []
    for r in rows:
        claimed = set(r["msg_closes"])
        for task in sorted(claimed & set(closings)):
            if closings[task] != r["hash"]:
                disagreements.append((task, closings[task], r["hash"]))
        closes = (claimed - set(closings)) | by_commit.get(r["hash"], set())
        r["tasks"] = sorted(closes)
        r["refs"] = sorted(set(r["msg_refs"]) - closes)
        r["task_id"] = min(closes) if closes else None
    return disagreements


def pick_newest_message_closings(rows, closings):
    """Keep one message-derived closing per task: the newest commit's.

    For a task the closings file does not list, several commits' messages may
    claim to close it (a checkpoint, then the commit that finished it). A task is
    closed once, so only the newest claim -- the first in `rows`, which is `git
    log` order, newest first -- keeps `CLOSES_TASK`; each older one is printed as
    `SUPERSEDED_MESSAGE_CLOSING` and taken off its row. Run after `classify`.
    Returns `(closers, superseded)`: `{task: kept hash}` and `(task, kept hash,
    dropped hash)` triples.
    """
    closers, superseded = {}, []
    for r in rows:
        keep = []
        for task in r["tasks"]:
            if task in closings:
                keep.append(task)
            elif task not in closers:
                closers[task] = r["hash"]
                keep.append(task)
            else:
                superseded.append((task, closers[task], r["hash"]))
        r["tasks"] = keep
        r["task_id"] = min(keep) if keep else None
    for task, kept, dropped in superseded:
        print(f"  SUPERSEDED_MESSAGE_CLOSING task=#{task} kept={kept[:7]} "
              f"dropped={dropped[:7]}", file=sys.stderr)
    return closers, superseded


def sh(*args):
    r = subprocess.run(args, capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"{' '.join(args)} failed: {r.stderr[:300]}")
    return r.stdout


def resolve_since(since):
    """Full hash of `since`, or `SystemExit` when it names no commit."""
    r = subprocess.run(["git", "rev-parse", "--verify", "--quiet",
                        f"{since}^{{commit}}"], capture_output=True, text=True)
    if r.returncode != 0 or not r.stdout.strip():
        raise SystemExit(f"--since {since!r} does not name a commit")
    return r.stdout.strip()


def commits(since=None):
    """Every commit reachable from HEAD, or only those after `since`."""
    fmt = SEP.join(["%H", "%s", "%aI", "%B"]) + REC
    rng = [f"{since}..HEAD"] if since else []
    out = sh("git", "log", f"--format={fmt}", *rng)
    for rec in out.split(REC):
        rec = rec.strip("\n")
        if not rec.strip():
            continue
        parts = rec.split(SEP)
        if len(parts) < 4:
            continue
        h, subject, date, body = parts[0], parts[1], parts[2], parts[3]
        tasks, refs = parse_tasks(body)
        yield {"hash": h, "message": subject, "date": date,
               "msg_closes": tasks, "msg_refs": refs}


def changed_files(since=None):
    """`{hash: [path, ...]}`: the files each commit changed, renames as a
    delete plus an add so both paths are listed whatever `diff.renames` says."""
    rng = [f"{since}..HEAD"] if since else []
    out = sh("git", "log", f"--format={REC}%H", "--name-only", "--no-renames", *rng)
    files = {}
    for rec in out.split(REC):
        lines = [ln for ln in rec.split("\n") if ln.strip()]
        if lines:
            files[lines[0].strip()] = sorted(set(lines[1:]))
    return files


def unwind_read(roadmap, rows, body, batch):
    """Rows (as dicts) of `UNWIND [<rows>] AS row <body>`, in batches."""
    out = []
    for i in range(0, len(rows), batch):
        maps = ", ".join("{" + ", ".join(f"{k}:{esc(v)}" for k, v in r.items()) + "}"
                         for r in rows[i:i + batch])
        doc_rows = kg_read_dicts(roadmap, f"UNWIND [{maps}] AS row\n{body}")
        out.extend(doc_rows)
    return out


TASK_EDGES = ("CLOSES_TASK", "REFERENCES_TASK")


def write_task_edges(roadmap, edge, links, batch, unwind, read_rows):
    """Write one Commit -> Task edge type for `links` (`hash`, `task`, `gitDate`).

    The model allows one task edge per (Commit, Task) pair: a commit either
    closes a task or references it, never both (`knowledge-model.md`,
    `REFERENCES_TASK`). So the other type's edge for each pair is deleted first,
    then the edge is merged and stamped, then read back; a pair
    still without it is reported on stderr. The summary line gives the engine's
    own counts, never the number of rows sent. Returns the missing pairs.
    """
    other = next(e for e in TASK_EDGES if e != edge)
    pairs = [{"hash": r["hash"], "task": r["task"]} for r in links]
    cleared = unwind("delete", pairs,
                     f"MATCH (c:Commit {{hash: row.hash}})-[r:{other}]->"
                     "(t:Task {id: row.task}) DELETE r",
                     f"{other} cleared under {edge}") or {}
    created = unwind("create", links,
                     "MATCH (c:Commit {hash: row.hash}) WITH c, row "
                     f"MATCH (t:Task {{id: row.task}}) MERGE (c)-[:{edge}]->(t)",
                     f"{edge} edges") or {}
    unwind("update", links,
           f"MATCH (c:Commit {{hash: row.hash}})-[e:{edge}]->(t:Task {{id: row.task}}) "
           "SET e.gitCommit = row.hash, e.gitDate = row.gitDate",
           f"{edge} stamped")
    missing = []
    if pairs:
        found = {(r["hash"], r["task"]) for r in read_rows(
            pairs,
            f"MATCH (c:Commit {{hash: row.hash}})-[e:{edge}]->(t:Task {{id: row.task}}) "
            "RETURN c.hash AS hash, t.id AS task")}
        missing = [(p_["hash"], p_["task"]) for p_ in pairs
                   if (p_["hash"], p_["task"]) not in found]
    for h, task in missing:
        print(f"  MISSING {edge}: {h[:7]} -> task #{task} (no Commit or Task node, "
              "or another edge holds the pair)", file=sys.stderr)
    print(f"{edge}: {len(pairs)} wanted, {len(pairs) - len(missing)} present after "
          f"the write ({created.get('relationshipsCreated', 0)} created), "
          f"{cleared.get('relationshipsDeleted', 0)} {other} cleared, "
          f"{len(missing)} missing", file=sys.stderr)
    return missing


def retire_superseded_closings(closers, unwind, read_rows, source="the closings file"):
    """Delete each `CLOSES_TASK` of a task in `closers` from any other commit.

    `closers` is `{task: hash}` from an authority -- the closings file, or the
    newest closing message of a task the file does not list. An edge an earlier
    run wrote from another commit's message goes. Each deletion is printed, and
    the summary gives the engine's deleted count, which must equal the edges
    read. Returns the deleted `(hash, task)` pairs.
    """
    rows = [{"task": t, "hash": h} for t, h in sorted(closers.items())]
    if not rows:
        return []
    body = ("MATCH (c:Commit)-[r:CLOSES_TASK]->(t:Task {id: row.task}) "
            "WHERE c.hash <> row.hash ")
    stale = [(r["hash"], r["task"])
             for r in read_rows(rows, body + "RETURN c.hash AS hash, t.id AS task")]
    for h, task in stale:
        print(f"  RETIRED CLOSES_TASK {h[:7]} -> task #{task}: {source} "
              f"says {closers[task][:7]}", file=sys.stderr)
    deleted = 0
    if stale:
        deleted = (unwind("delete", rows, body + "DELETE r",
                          f"CLOSES_TASK superseded by {source}") or {}).get(
                              "relationshipsDeleted", 0)
    print(f"CLOSES_TASK superseded by {source}: {len(stale)} found, {deleted} deleted",
          file=sys.stderr)
    if deleted != len(stale):
        print(f"  WARNING: {len(stale)} superseded CLOSES_TASK edges were read but the "
              f"engine deleted {deleted}", file=sys.stderr)
    return stale


def retire_underived(rows, unwind, read_rows):
    """Delete every Commit -> Task edge of the processed commits this run did not derive.

    A sync leaves only facts it derives: for each commit in `rows` (the processed
    range, so a `--since` run touches nothing older), a `CLOSES_TASK` whose task
    is not in the row's `tasks`, or a `REFERENCES_TASK` whose task is not in its
    `refs`, was written by an earlier run under rules that no longer hold. Each
    is printed as `RETIRED <edge> <hash> -> task #N: not derived`, and the
    summary gives the engine's deleted count. Returns the deleted
    `(edge, hash, task)` triples.
    """
    hashes = [{"hash": r["hash"]} for r in rows]
    retired = []
    for edge, key in (("CLOSES_TASK", "tasks"), ("REFERENCES_TASK", "refs")):
        if not hashes:
            break
        derived = {(r["hash"], t) for r in rows for t in r[key]}
        present = [(x["hash"], x["task"]) for x in read_rows(
            hashes, f"MATCH (c:Commit {{hash: row.hash}})-[e:{edge}]->(t:Task) "
                    "RETURN c.hash AS hash, t.id AS task")]
        stale = sorted(set(present) - derived)
        for h, task in stale:
            print(f"  RETIRED {edge} {h[:7]} -> task #{task}: not derived", file=sys.stderr)
        deleted = 0
        if stale:
            deleted = (unwind("delete", [{"hash": h, "task": task} for h, task in stale],
                              f"MATCH (c:Commit {{hash: row.hash}})-[r:{edge}]->"
                              "(t:Task {id: row.task}) DELETE r",
                              f"{edge} not derived") or {}).get("relationshipsDeleted", 0)
        print(f"{edge} not derived: {len(stale)} found, {deleted} deleted", file=sys.stderr)
        if deleted != len(stale):
            print(f"  WARNING: {len(stale)} underived {edge} edges were read but the "
                  f"engine deleted {deleted}", file=sys.stderr)
        retired += [(edge, h, task) for h, task in stale]
    return retired


def main(argv=None):
    ap = argparse.ArgumentParser(
        description="Mirror the commit history, COMMITTED_IN, CLOSES_TASK and "
                    "REFERENCES_TASK into the graph.")
    ap.add_argument("roadmap")
    ap.add_argument("--batch-size", type=int, default=100)
    ap.add_argument("--since", metavar="COMMIT",
                    help="process only the commits after COMMIT "
                         "(`git log COMMIT..HEAD`); default: the whole history")
    ap.add_argument("--closings", metavar="FILE",
                    help="rmp closings export, a JSON list of "
                         '{"task": N, "commit_close": "<hash>"}; '
                         "it decides CLOSES_TASK for every task it lists")
    ap.add_argument("--bad-closings-report", metavar="FILE",
                    default=str(kg_target() / "rucene_kg" / "bad_closings.jsonl"),
                    help="JSON-lines file each unresolvable closing is appended to "
                         "(default: %(default)s)")
    args = ap.parse_args(argv)

    since = resolve_since(args.since) if args.since else None
    closings, bad = load_closings(args.closings) if args.closings else ({}, [])
    write_bad_closings(args.bad_closings_report, bad)
    rows = list(commits(since))
    disagreements = classify(rows, closings)
    message_closers, _superseded = pick_newest_message_closings(rows, closings)
    for task, file_hash, msg_hash in disagreements:
        print(f"  DISAGREEMENT task #{task}: the closings file says {file_hash[:7]}, "
              f"the message of {msg_hash[:7]} claims it; the file wins", file=sys.stderr)
    in_range = {r["hash"] for r in rows}
    if closings:
        print(f"closings file: {len(closings)} tasks, "
              f"{sum(1 for h in closings.values() if h in in_range)} closed in range, "
              f"{len(disagreements)} disagreement(s)", file=sys.stderr)
    files = changed_files(since)
    print(f"commits: {len(rows)}"
          + (f" after {since[:7]}" if since else "")
          + f", {sum(1 for r in rows if r['tasks'])} closing a task, "
          f"{sum(1 for r in rows if r['refs'])} referencing one", file=sys.stderr)

    def unwind(mode, data, body, label):
        return kg_unwind(args.roadmap, mode, data, body, label, args.batch_size)

    unwind("create", [{"hash": r["hash"]} for r in rows],
           "MERGE (c:Commit {hash: row.hash})", "Commit nodes")
    unwind("update",
           [{"hash": r["hash"], "message": r["message"], "date": r["date"],
             "task_id": r["task_id"], "gitDate": r["date"][:10]} for r in rows],
           "MATCH (c:Commit {hash: row.hash}) SET c.message = row.message, "
           "c.date = row.date, c.task_id = row.task_id, "
           "c.gitCommit = row.hash, c.gitDate = row.gitDate",
           "Commit properties")

    date_of = {r["hash"]: r["date"][:10] for r in rows}
    touched = [{"path": p, "hash": r["hash"], "gitDate": date_of[r["hash"]]}
               for r in rows for p in files.get(r["hash"], [])]
    unwind("create", touched,
           "MATCH (f:File {path: row.path}) WITH f, row "
           "MATCH (c:Commit {hash: row.hash}) MERGE (f)-[:COMMITTED_IN]->(c)",
           "COMMITTED_IN edges")
    unwind("update", touched,
           "MATCH (f:File {path: row.path})-[e:COMMITTED_IN]->(c:Commit {hash: row.hash}) "
           "SET e.gitCommit = row.hash, e.gitDate = row.gitDate",
           "COMMITTED_IN stamped")

    def read_rows(data, body):
        return unwind_read(args.roadmap, data, body, args.batch_size)

    retire_superseded_closings(closings, unwind, read_rows)
    retire_superseded_closings(message_closers, unwind, read_rows,
                               source="the newest closing message")
    retire_underived(rows, unwind, read_rows)
    for edge, key in (("CLOSES_TASK", "tasks"), ("REFERENCES_TASK", "refs")):
        links = [{"hash": r["hash"], "task": t, "gitDate": date_of[r["hash"]]}
                 for r in rows for t in r[key]]
        write_task_edges(args.roadmap, edge, links, args.batch_size, unwind, read_rows)

if __name__ == "__main__":
    main()
