"""`commits_kg.py`: task references, `--since`, `COMMITTED_IN`, `REFERENCES_TASK`."""
import re
import subprocess

import pytest

import commits_kg as ck

REPO = ck.Path(ck.__file__).resolve().parents[2]


@pytest.mark.parametrize(
    "message, closes, refs",
    [
        # footers, with every number of a list
        ("feat: x\n\nRefs #144\n", [], [144]),
        ("feat: x\n\nRefs #192, #209\n", [], [192, 209]),
        ("feat: x\n\nRefs: #1, #2 and #3\n", [], [1, 2, 3]),
        ("feat: x\n\nRefs #1, #2, and #3\n", [], [1, 2, 3]),
        ("feat: x\n\nRefs rmp task #5\n", [], [5]),
        ("fix: x\n\nCloses #18\n", [18], []),
        ("fix: x\n\nFixes #7, #9\n", [7, 9], []),
        ("fix: x\n\ncloses: #5\n", [5], []),
        ("x\n\nCloses rmp task #10.\n", [10], []),
        # the task tag of the history's subjects and first body lines
        ("Task #8: port MMapDirectory\n", [8], []),
        ("Task 99: port IndexWriterConfig\n", [99], []),
        ("Task: #15\n", [15], []),
        ("Tasks #82 and #83: base search\n", [82, 83], []),
        ("Task #64, #68, #73: norms\n", [64, 68, 73], []),
        ("feat: x\n\ntasks #217, #218, #219 and #220, the second effort.\n",
         [217, 218, 219, 220], []),
        ("feat: x\n\ntask #210. The bulk-copy merge\n", [210], []),
        ("feat: x\n\ntask #201.\n", [201], []),
        # a task both closed and referenced is only closed
        ("x\n\nCloses #4\nRefs #4, #6\n", [4], [6]),
        # prose is never a reference
        ("x\n\nFound and not fixed, now task #205: the scorer\n", [], []),
        ("x\n\nTask #161's triage found 179 methods\n", [], []),
        ("x\n\nTask #156 re-synchronised the graph\n", [], []),
        # a tag anywhere in the subject line counts; the subject is a title
        ("Task #74 checkpoint: recursive writer\n", [74], []),
        ("fix(index): task 93 SegmentReader close\n", [93], []),
        ("Sprint 1 Task #2: port DataInput\n", [2], []),
        ("x\n\nfix(index): task 93 in the body is prose\n", [], []),
        ("x\n\nPhase 1 of task #92 (foundation)\n", [], []),
        ("this refs #5 in passing and fixes #6 too\n", [], []),
        ("bump to 1.2 (#12)\n", [], []),
        # B4: a number ends at a boundary; a period ending the sentence is fine
        ("fix: task 10x faster\n", [], []),
        ("perf: tasks 3 ways\n", [], []),
        ("x: tasks 2.5\n", [], []),
        ("x\n\ntask 10x faster.\n", [], []),
        ("Tasks 41 and 42: x\n", [41, 42], []),
        # B4: only the first body line carries the tag; a relative clause is prose
        ("x\n\nTask #12, which was split.\n", [], []),
        ("x\n\nTask #12, that one.\n", [], []),
        ("x\n\nfirst paragraph.\n\ntask #7.\n", [], []),
        ("x\n\ntask #200, acceptance criterion 3.\n", [200], []),
        # a `Task: #N` trailer is a footer on any line
        ("x\n\nbody.\n\nTask: #15\n", [15], []),
        ("x\n\nbody.\n\nTask: #15 and more prose\n", [], []),
    ],
)
def test_parse_tasks(message, closes, refs):
    assert ck.parse_tasks(message) == (closes, refs)


def row(h, closes=(), refs=()):
    return {"hash": h, "msg_closes": list(closes), "msg_refs": list(refs)}


def test_classify_closings_file_wins_and_messages_fill_the_rest():
    x, y, z = "1" * 40, "2" * 40, "3" * 40
    rows = [row(x, closes=[200, 5]), row(y, closes=[200], refs=[92]), row(z, refs=[200])]
    # the file says z closed #200 and y closed #92; #5 is not in the file
    dis = ck.classify(rows, {200: z, 92: y})
    by = {r["hash"]: r for r in rows}
    assert by[x]["tasks"] == [5] and by[x]["task_id"] == 5
    assert by[y]["tasks"] == [92] and by[y]["refs"] == []  # closed, so not also referenced
    assert by[z]["tasks"] == [200] and by[z]["refs"] == []
    assert dis == [(200, z, x), (200, z, y)]


def test_classify_a_file_closing_needs_no_message():
    x = "1" * 40
    rows = [row(x)]
    assert ck.classify(rows, {156: x}) == []
    assert rows[0]["tasks"] == [156] and rows[0]["task_id"] == 156


def test_classify_without_a_file_uses_the_messages():
    rows = [row("1" * 40, closes=[93], refs=[4])]
    assert ck.classify(rows, {}) == []
    assert rows[0]["tasks"] == [93] and rows[0]["refs"] == [4]


def _git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, cwd=REPO,
                          check=True).stdout.strip()


def test_load_closings_resolves_abbreviated_and_full_hashes(monkeypatch, tmp_path):
    monkeypatch.chdir(REPO)
    head, parent = _git("rev-parse", "HEAD"), _git("rev-parse", "HEAD~1")
    f = tmp_path / "closings.json"
    f.write_text(ck_json([{"task": 1, "commit_close": head[:7]},
                          {"task": 2, "commit_close": parent}]), encoding="utf-8")
    assert ck.load_closings(f) == ({1: head, 2: parent}, [])


@pytest.mark.parametrize("hash_, reason", [
    ("zzzzzzz", "not 7-40"),
    ("abc", "not 7-40"),
    ("0000000000", "names no commit"),
])
def test_a_bad_hash_is_reported_and_its_task_left_out(monkeypatch, tmp_path, hash_, reason):
    monkeypatch.chdir(REPO)
    head = _git("rev-parse", "HEAD")
    f = tmp_path / "closings.json"
    f.write_text(ck_json([{"task": 1, "commit_close": head},
                          {"task": 106, "commit_close": hash_}]), encoding="utf-8")
    closings, bad = ck.load_closings(f)
    assert closings == {1: head}
    assert [(b["task"], b["hash"]) for b in bad] == [(106, hash_)]
    assert reason in bad[0]["reason"]


def test_bad_closings_are_printed_and_appended(tmp_path, capsys):
    out = tmp_path / "sub" / "bad.jsonl"
    bad = [{"task": 106, "hash": "b0e1a75a", "reason": "names no commit"}]
    ck.write_bad_closings(out, bad)
    ck.write_bad_closings(out, bad)
    assert "BAD_CLOSING task=106 hash=b0e1a75a reason=names no commit" in capsys.readouterr().err
    lines = out.read_text(encoding="utf-8").splitlines()
    assert len(lines) == 2 and ck_json_load(lines[0]) == bad[0]


@pytest.mark.parametrize("content, msg", [
    ("{not json", "not JSON"),
    ('{"task": 1}', "expected a JSON list"),
    ('[{"task": "1", "commit_close": "abcdef0"}]', "malformed"),
    ('[{"task": 1}]', "malformed"),
    ('[{"task": 1, "commit_close": 7}]', "malformed"),
    ('[{"task": 1, "commit_close": "abcdef0"}, {"task": 1, "commit_close": "abcdef1"}]',
     "listed twice"),
])
def test_a_malformed_file_still_stops_the_tool(monkeypatch, tmp_path, content, msg):
    monkeypatch.chdir(REPO)
    f = tmp_path / "closings.json"
    f.write_text(content, encoding="utf-8")
    with pytest.raises(SystemExit, match=msg):
        ck.load_closings(f)


def ck_json_load(s):
    import json
    return json.loads(s)


def ck_json(v):
    import json
    return json.dumps(v)


def fake_log(*records):
    return "".join(ck.SEP.join(r) + ck.REC for r in records)


A, B, C = "a" * 40, "b" * 40, "c" * 40
LOG = fake_log(
    (A, "feat: one", "2026-10-30T10:00:00+00:00", "feat: one\n\nRefs #192, #209\n"),
    (B, "fix: two", "2026-10-29T10:00:00+00:00", "fix: two\n\nFixes #7, #9\nRefs #7\n"),
)
NAMES = f"{ck.REC}{A}\n\nsrc/a.rs\nsrc/b.rs\n{ck.REC}{B}\n\nsrc/a.rs\nsrc/old.rs\n"


@pytest.fixture
def run(monkeypatch):
    calls, writes = [], []

    def fake_sh(*args):
        calls.append(args)
        return NAMES if "--name-only" in args else LOG

    monkeypatch.setattr(ck, "sh", fake_sh)
    monkeypatch.setattr(ck, "resolve_since", lambda rev: C)
    monkeypatch.setattr(ck, "unwind_read", lambda roadmap, rows, body, batch: (
        [] if "<>" in body else [{"hash": r["hash"], "task": r["task"]} for r in rows]))
    monkeypatch.setattr(ck, "kg_unwind", lambda roadmap, mode, rows, body, label, batch:
                        writes.append((label, body, [dict(r) for r in rows])))

    def go(argv, closings=None):
        if closings is not None:
            monkeypatch.setattr(ck, "load_closings", lambda path: closings)
        monkeypatch.setattr(ck, "write_bad_closings", lambda path, bad: None)
        ck.main(argv)
        return calls, {label: (body, rows) for label, body, rows in writes}

    return go


def test_since_limits_every_git_read(run):
    calls, _ = run(["rucene", "--since", "HEAD~2"])
    assert len(calls) == 2
    for args in calls:
        assert args[:2] == ("git", "log") and args[-1] == C + "..HEAD"
    assert "--no-renames" in calls[1]


def test_without_since_the_whole_history_is_read(run):
    calls, _ = run(["rucene"])
    assert all(".." not in a[-1] for a in calls)


def test_committed_in_is_file_to_commit_by_path_and_hash(run):
    _, w = run(["rucene", "--since", "HEAD~2"])
    body, rows = w["COMMITTED_IN edges"]
    assert "MATCH (f:File {path: row.path})" in body
    assert "MATCH (c:Commit {hash: row.hash}) MERGE (f)-[:COMMITTED_IN]->(c)" in body
    assert "MERGE (f:File" not in body  # no File node is ever created here
    assert sorted((r["path"], r["hash"]) for r in rows) == [
        ("src/a.rs", A), ("src/a.rs", B), ("src/b.rs", A), ("src/old.rs", B)]
    sbody, srows = w["COMMITTED_IN stamped"]
    assert "SET e.gitCommit = row.hash, e.gitDate = row.gitDate" in sbody
    assert {r["gitDate"] for r in srows if r["hash"] == A} == {"2026-10-30"}


def test_closes_and_references_write_their_own_edges(run):
    _, w = run(["rucene"])
    cbody, closes = w["CLOSES_TASK edges"]
    rbody, refs = w["REFERENCES_TASK edges"]
    assert "MERGE (c)-[:CLOSES_TASK]->(t)" in cbody
    assert "MATCH (c:Commit {hash: row.hash})" in rbody
    assert "MATCH (t:Task {id: row.task}) MERGE (c)-[:REFERENCES_TASK]->(t)" in rbody
    assert [(r["hash"], r["task"]) for r in closes] == [(B, 7), (B, 9)]
    assert [(r["hash"], r["task"]) for r in refs] == [(A, 192), (A, 209)]
    assert {label for label in w} >= {"REFERENCES_TASK stamped", "CLOSES_TASK stamped"}
    props = w["Commit properties"][1]
    assert {r["hash"]: r["task_id"] for r in props} == {A: None, B: 7}
    assert not any("MODIFIES" in body for body, _rows in w.values())


def test_changed_files_against_this_repository(monkeypatch):
    monkeypatch.chdir(REPO)
    files = ck.changed_files("HEAD~1")
    head = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True,
                          text=True, check=True).stdout.strip()
    want = subprocess.run(["git", "show", "--no-renames", "--name-only", "--format=",
                           "HEAD"], capture_output=True, text=True,
                          check=True).stdout.split()
    assert list(files) == [head]
    assert files[head] == sorted(set(want))


def test_resolve_since_against_this_repository(monkeypatch):
    monkeypatch.chdir(REPO)
    head = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True,
                          text=True, check=True).stdout.strip()
    assert ck.resolve_since("HEAD") == head
    with pytest.raises(SystemExit, match="does not name a commit"):
        ck.resolve_since("no-such-revision-xyz")


def test_main_closings_file_decides_and_disagreements_are_printed(run, capsys):
    # B's message says `Fixes #7, #9`; the file says A closed #7, and #300 too
    _, w = run(["rucene", "--closings", "c.json"], closings=({7: A, 300: A}, []))
    closes = sorted((r["hash"], r["task"]) for r in w["CLOSES_TASK edges"][1])
    assert closes == [(A, 7), (A, 300), (B, 9)]
    refs = sorted((r["hash"], r["task"]) for r in w["REFERENCES_TASK edges"][1])
    assert refs == [(A, 192), (A, 209)]
    err = capsys.readouterr().err
    assert "DISAGREEMENT task #7: the closings file says aaaaaaa, the message of bbbbbbb" in err
    assert "closings file: 2 tasks, 2 closed in range, 1 disagreement(s)" in err


def test_main_skips_a_closing_whose_commit_is_outside_the_range(run):
    _, w = run(["rucene", "--since", "HEAD~2", "--closings", "c.json"],
               closings=({400: "9" * 40}, []))
    assert all(r["task"] != 400 for r in w["CLOSES_TASK edges"][1])


def test_main_falls_back_to_the_message_for_a_bad_closing(run, monkeypatch, tmp_path, capsys):
    # #7 has a bad entry, so B's `Fixes #7, #9` decides it
    monkeypatch.setattr(ck, "load_closings", lambda path: (
        {}, [{"task": 7, "hash": "b0e1a75a", "reason": "names no commit"}]))
    report = tmp_path / "bad.jsonl"
    ck.main(["rucene", "--closings", "c.json", "--bad-closings-report", str(report)])
    assert "BAD_CLOSING task=7 hash=b0e1a75a" in capsys.readouterr().err
    assert report.exists()


class CommitGraph:
    """Commit -> Task edges as the engine keeps them: one edge per ordered pair."""

    def __init__(self, commits, tasks):
        self.commits, self.tasks = set(commits), set(tasks)
        self.edges = {}  # (hash, task) -> [type, props]
        self.log = []

    def unwind(self, mode, rows, body, label):
        self.log.append((label, body))
        m = re.search(r"-\[r:(\w+)\]->", body)
        for row in rows:
            key = (row.get("hash"), row["task"])
            if body.endswith("DELETE r") and m and "<>" not in body:
                if self.edges.get(key, [None])[0] == m.group(1):
                    del self.edges[key]
            elif "<>" in body and body.endswith("DELETE r"):
                for k in [k for k, v in self.edges.items()
                          if k[1] == row["task"] and k[0] != row["hash"] and v[0] == "CLOSES_TASK"]:
                    del self.edges[k]
            elif "MERGE" in body:
                typ = re.search(r"MERGE \(c\)-\[:(\w+)\]->\(t\)", body).group(1)
                if key[0] in self.commits and key[1] in self.tasks and key not in self.edges:
                    self.edges[key] = [typ, {}]
            elif "SET e.gitCommit" in body:
                typ = re.search(r"-\[e:(\w+)\]->", body).group(1)
                if self.edges.get(key, [None])[0] == typ:
                    self.edges[key][1].update(gitCommit=row["hash"], gitDate=row["gitDate"])
            else:
                raise AssertionError(body)

    def read_rows(self, rows, body):
        if "<>" in body:
            tasks = {r["task"]: r["hash"] for r in rows}
            return [{"hash": h, "task": t} for (h, t), (typ, _p) in sorted(self.edges.items())
                    if typ == "CLOSES_TASK" and t in tasks and h != tasks[t]]
        typ = re.search(r"-\[e:(\w+)\]->", body).group(1)
        return [{"hash": r["hash"], "task": r["task"]} for r in rows
                if self.edges.get((r["hash"], r["task"]), [None])[0] == typ]


def link(h, task):
    return {"hash": h, "task": task, "gitDate": "2026-10-30"}


def test_b1_a_closing_replaces_an_older_reference_on_the_same_pair(capsys):
    g = CommitGraph([A], [7])
    g.edges[(A, 7)] = ["REFERENCES_TASK", {}]  # written by an earlier run
    missing = ck.write_task_edges("rucene", "CLOSES_TASK", [link(A, 7)], 100,
                                  g.unwind, g.read_rows)
    assert missing == [] and g.edges[(A, 7)][0] == "CLOSES_TASK"
    assert g.edges[(A, 7)][1]["gitDate"] == "2026-10-30"
    assert "MISSING" not in capsys.readouterr().err


def test_b1_without_the_delete_the_merge_would_create_nothing():
    g = CommitGraph([A], [7])
    g.edges[(A, 7)] = ["REFERENCES_TASK", {}]
    g.unwind("create", [link(A, 7)], "MATCH (c:Commit {hash: row.hash}) WITH c, row "
             "MATCH (t:Task {id: row.task}) MERGE (c)-[:CLOSES_TASK]->(t)", "x")
    assert g.edges[(A, 7)][0] == "REFERENCES_TASK"


def test_b1_a_reference_replaces_an_older_closing(capsys):
    g = CommitGraph([A], [7])
    g.edges[(A, 7)] = ["CLOSES_TASK", {}]
    assert ck.write_task_edges("rucene", "REFERENCES_TASK", [link(A, 7)], 100,
                               g.unwind, g.read_rows) == []
    assert g.edges[(A, 7)][0] == "REFERENCES_TASK"


def test_b1_a_missing_edge_is_reported(capsys):
    g = CommitGraph([A], [7])  # task #8 has no Task node
    missing = ck.write_task_edges("rucene", "CLOSES_TASK", [link(A, 7), link(A, 8)], 100,
                                  g.unwind, g.read_rows)
    assert missing == [(A, 8)]
    assert "MISSING CLOSES_TASK: aaaaaaa -> task #8" in capsys.readouterr().err


def test_b2_superseded_closings_are_retired_and_reported(capsys):
    g = CommitGraph([A, B, C], [92, 93])
    g.edges[(B, 92)] = ["CLOSES_TASK", {}]   # message-derived, older run
    g.edges[(C, 92)] = ["CLOSES_TASK", {}]   # message-derived, older run
    g.edges[(A, 92)] = ["CLOSES_TASK", {}]   # the file's commit: kept
    g.edges[(B, 93)] = ["CLOSES_TASK", {}]   # #93 is not in the file: kept
    g.edges[(C, 93)] = ["REFERENCES_TASK", {}]
    stale = ck.retire_superseded_closings({92: A}, g.unwind, g.read_rows)
    assert sorted(stale) == [(B, 92), (C, 92)]
    assert set(g.edges) == {(A, 92), (B, 93), (C, 93)}
    err = capsys.readouterr().err
    assert "RETIRED CLOSES_TASK bbbbbbb -> task #92: the closings file says aaaaaaa" in err
    assert "RETIRED CLOSES_TASK ccccccc -> task #92" in err


def test_b2_nothing_to_retire_writes_nothing():
    g = CommitGraph([A], [92])
    g.edges[(A, 92)] = ["CLOSES_TASK", {}]
    assert ck.retire_superseded_closings({92: A}, g.unwind, g.read_rows) == []
    assert g.log == []
