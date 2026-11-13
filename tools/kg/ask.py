#!/usr/bin/env python3
"""Ask the knowledge graph the questions the development work asks.

Each subcommand runs the statement `knowledge-model.md` records for one standing
question and prints compact JSON lines, so a reader needs no Cypher and meets
none of the engine's pitfalls (`knowledge-model.md`, *Query pitfalls*). Inputs
are escaped as Cypher literals.

    python3 tools/kg/ask.py status                      # is the graph current?
    python3 tools/kg/ask.py divergences --file src/index/index_writer.rs
    python3 tools/kg/ask.py divergences --type org.apache.lucene.index.IndexWriter
    python3 tools/kg/ask.py item IndexWriter            # where it lives, what it ports
    python3 tools/kg/ask.py refs rucene::index::index_writer::IndexWriter::commit
    python3 tools/kg/ask.py lucene-callers rucene::index::index_writer::IndexWriter::commit
    python3 tools/kg/ask.py no-caller --subsystem search --unverified
    python3 tools/kg/ask.py awaits                      # W-blocked items, by Java caller
    python3 tools/kg/ask.py absent                      # member work still open
    python3 tools/kg/ask.py tasks                       # open tasks and their blockers
    python3 tools/kg/ask.py coverage                    # in-scope classes per module and status
    python3 tools/kg/ask.py coverage --module lucene/codecs --list   # each class and its status
    python3 tools/kg/ask.py early --module lucene/test-framework     # cycle-rule early members
    python3 tools/kg/ask.py plans --class org.apache.lucene.analysis.ar.ArabicAnalyzer
                                                        # which task ports a class
    python3 tools/kg/ask.py plans --task 412            # which classes a task ports
    python3 tools/kg/ask.py skeletons --module lucene/analysis/common  # skeleton-only classes
    python3 tools/kg/ask.py cypher "MATCH (n:Module) RETURN n.name"

Every command needs the graph server (`rmp graph serve -r rucene`). Run
`status` first: an answer is about the commit the graph was synced at, and
`tools/kg/sync_commit.sh HEAD` brings it current.
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from kgio import esc, kg_target, read  # noqa: E402
import port_modules_kg as pm  # noqa: E402

R = "rucene"
TYPE_LABELS = ("RustStruct", "RustTrait", "RustEnum", "RustAlias")


def emit(rows, cols):
    for r in rows:
        print(json.dumps(dict(zip(cols, r)), ensure_ascii=False))


def q_divergences_in_file(path):
    return ("MATCH (d:Divergence)-[:DECLARED_IN]->(:File {path: " + esc(path) + "}) "
            "RETURN d.line, d.itemKind, d.item, d.javaRefs, d.text ORDER BY d.line")


def q_divergences_of_type(fqn):
    return ("MATCH (c:Class {qualifiedName: " + esc(fqn) + "}) "
            "OPTIONAL MATCH (d1:Divergence)-[:CONCERNS]->()-[:PORTS]->(c) "
            "OPTIONAL MATCH (d2:Divergence)-[:DIVERGES_FROM]->(c) "
            "WITH collect(DISTINCT d1) + collect(DISTINCT d2) AS ds "
            "UNWIND ds AS d RETURN DISTINCT d.id, d.item, d.text")


def q_item(name):
    return ("MATCH (f:File)-[:DECLARES]->(t) WHERE t.name = " + esc(name) + " "
            "OPTIONAL MATCH (t)-[e:PORTS]->(c:Class) "
            "RETURN f.path, labels(t)[0], t.qualifiedName, c.qualifiedName, e.evidence, e.note")


def q_refs(qn):
    return ("MATCH (r)-[e:REFERENCES]->(x) WHERE x.qualifiedName = " + esc(qn) + " "
            "AND e.evidence = 'rust-analyzer-scip' "
            "RETURN labels(r)[0], coalesce(r.qualifiedName, r.path), e.prodRefs, e.testRefs "
            "ORDER BY e.prodRefs DESC")


def q_lucene_callers(qn):
    return ("MATCH (:RustFn {qualifiedName: " + esc(qn) + "})-[:PORTS_MEMBER]->(m:Method) "
            "OPTIONAL MATCH (m)<-[:CALLS]-(c1) "
            "OPTIONAL MATCH (m)-[:OVERRIDES]->(:Method)<-[:CALLS]-(c2) "
            "WITH m, collect(DISTINCT coalesce(c1.parentQualifiedName, c1.qualifiedName)) + "
            "collect(DISTINCT coalesce(c2.parentQualifiedName, c2.qualifiedName)) AS cs "
            "RETURN m.qualifiedName, [c IN cs WHERE c IS NOT NULL]")


def q_verdicts(label):
    return (f"MATCH (n:{label}) WHERE n.noCallerVerdict IS NOT NULL "
            "RETURN n.qualifiedName, n.noCallerVerdict, n.noCallerTable")


Q_AWAITS = ("MATCH (n)-[:AWAITS_CALLER]->(c:Class) "
            "RETURN c.qualifiedName, c.portState, collect(n.qualifiedName) AS waiting "
            "ORDER BY size(waiting) DESC")
Q_ABSENT = ("MATCH (c:Class) WHERE c.portScope = 'in' AND c.memberAbsent > 0 "
            "OPTIONAL MATCH (c)-[:DECLARES]->(m:Method) WHERE m.portTriage = 'absent' "
            "RETURN c.qualifiedName, c.memberAbsent, collect(DISTINCT m.name) "
            "ORDER BY c.memberAbsent DESC")
Q_TASKS = ("MATCH (t:Task) WHERE t.status IN ['SPRINT', 'DOING', 'TESTING'] "
           "OPTIONAL MATCH (t)-[:DEPENDS_ON]->(b:Task) WHERE b.status <> 'COMPLETED' "
           "RETURN t.id, t.status, t.name, collect(b.id) ORDER BY t.id")


def join_no_caller(list_rows, verdicts, subsystem=None, unverified=False):
    """The no-caller list rows, each with its #269 verdict (or `None`).

    `list_rows` are the dicts of `list_<commit>.jsonl`; `verdicts` maps an item's
    qualified name to `(verdict, table)`.
    """
    out = []
    for r in list_rows:
        if subsystem and r.get("subsystem") != subsystem:
            continue
        v = verdicts.get(r["item"])
        if unverified and v:
            continue
        out.append({"item": r["item"], "label": r.get("label"), "auto": r.get("class"),
                    "verdict": v[0] if v else None, "table": v[1] if v else None,
                    "callerClasses": r.get("callerClasses") or [],
                    "blocked": r.get("blocked")})
    return out


Q_SYNCED = "MATCH (p:Project {name: 'Rucene'}) RETURN p.syncedCommit, p.syncedDate"


def surveyed_commit():
    """The commit of the last sync whose audit passed (`sync_commit.sh` writes it last)."""
    rows = read(R, Q_SYNCED)
    return rows[0][0] if rows and rows[0][0] else None


def cmd_status(_a):
    graph = surveyed_commit()
    head = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    changed = None
    if graph:
        changed = subprocess.run(["git", "diff", "--stat", graph, "HEAD", "--", "src", "tests", "crates"],
                                 capture_output=True, text=True).stdout.strip().splitlines()
    print(json.dumps({"graphCommit": graph, "head": head, "current": graph == head,
                      "codeChangedSinceGraph": changed[-1] if changed else "none"}))


def cmd_divergences(a):
    if a.file:
        emit(read(R, q_divergences_in_file(a.file)), ["line", "itemKind", "item", "javaRefs", "text"])
    else:
        emit(read(R, q_divergences_of_type(a.type)), ["id", "item", "text"])


def cmd_item(a):
    emit(read(R, q_item(a.name)), ["file", "label", "qualifiedName", "ports", "evidence", "note"])


def cmd_refs(a):
    emit(read(R, q_refs(a.qn)), ["label", "referrer", "prodRefs", "testRefs"])


def cmd_lucene_callers(a):
    emit(read(R, q_lucene_callers(a.qn)), ["member", "callerClasses"])


def cmd_no_caller(a):
    commit = surveyed_commit() or ""
    path = Path(a.list) if a.list else kg_target() / "no_caller" / f"list_{commit[:7]}.jsonl"
    if not path.exists():
        raise SystemExit(f"no no-caller list at {path}; run tools/kg/sync_commit.sh")
    rows = [json.loads(l) for l in path.read_text(encoding="utf-8").splitlines() if l.strip()]
    verdicts = {}
    for label in ("RustFn",) + TYPE_LABELS:
        for qn, v, t in read(R, q_verdicts(label)):
            verdicts[qn] = (v, t)
    for r in join_no_caller(rows, verdicts, a.subsystem, a.unverified):
        print(json.dumps(r, ensure_ascii=False))


def cmd_awaits(_a):
    emit(read(R, Q_AWAITS), ["javaCaller", "portState", "waiting"])


def cmd_absent(_a):
    emit(read(R, Q_ABSENT), ["type", "absent", "members"])


def cmd_tasks(_a):
    emit(read(R, Q_TASKS), ["id", "status", "name", "blockedBy"])


def q_coverage_list(module):
    """Each in-scope class of `module` and its status (`ported`, `skeleton`,
    `unported`), by status."""
    return ("MATCH (c:Class) WHERE c.portScope = 'in' AND c.module = " + esc(module) + " "
            "RETURN c.qualifiedName AS class, "
            "CASE WHEN c.portState = 'ported' THEN 'ported' "
            "WHEN c.portState = 'skeleton' THEN 'skeleton' ELSE 'unported' END AS status, "
            "c.portState AS portState ORDER BY status, class")


# The skeleton relation (`knowledge-model.md`, *Which task ports a class*).
PLANS_OF_CLASS = (
    "MATCH (t:Task)-[e:PLANS_PORT]->(c:Class {qualifiedName: $class}) "
    "RETURN c.qualifiedName AS class, c.portState AS portState, t.id AS task, "
    "t.status AS taskStatus, t.name AS taskName, e.declaredAt AS declaredAt"
)
CLASSES_OF_TASK = (
    "MATCH (t:Task {id: $task})-[e:PLANS_PORT]->(c:Class) "
    "RETURN c.qualifiedName AS class, c.portScope AS portScope, c.portState AS portState, "
    "e.declaredAt AS declaredAt ORDER BY class"
)
SKELETON_ONLY = (
    "MATCH (c:Class) WHERE c.portState = 'skeleton' "
    "MATCH (s)-[e:SKELETON_OF]->(c) "
    "RETURN c.module AS module, c.qualifiedName AS class, c.portScope AS portScope, "
    "s.name AS item, s.file AS file, e.task AS task ORDER BY module, class"
)


def q_plans_of_class(fqn):
    """The task(s) whose skeleton plans the port of `fqn` (a nested type is
    keyed `pkg.Outer$Inner`)."""
    return PLANS_OF_CLASS.replace("$class", esc(fqn))


def q_classes_of_task(task):
    return CLASSES_OF_TASK.replace("$task", str(int(task)))


def q_skeleton_only(module=None):
    if module is None:
        return SKELETON_ONLY
    return SKELETON_ONLY.replace("WHERE c.portState = 'skeleton' ",
                                 f"WHERE c.portState = 'skeleton' AND c.module = {esc(module)} ")


def cmd_plans(a):
    if a.cls:
        emit(read(R, q_plans_of_class(a.cls)),
             ["class", "portState", "task", "taskStatus", "taskName", "declaredAt"])
    else:
        emit(read(R, q_classes_of_task(a.task)), ["class", "portScope", "portState", "declaredAt"])


def cmd_skeletons(a):
    emit(read(R, q_skeleton_only(a.module)),
         ["module", "class", "portScope", "item", "file", "task"])


def cmd_coverage(a):
    if a.list:
        if not a.module:
            raise SystemExit("--list needs --module")
        emit(read(R, q_coverage_list(a.module)), ["class", "status", "portState"])
    else:
        emit(read(R, pm.coverage_query(a.module)), ["module", "status", "classes"])


def cmd_early(a):
    emit(read(R, pm.early_query(a.module)),
         ["module", "class", "member", "task", "ownerTask", "rustItem"])


def cmd_cypher(a):
    for row in read(R, a.statement):
        print(json.dumps(row, ensure_ascii=False))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("status").set_defaults(f=cmd_status)
    p = sub.add_parser("divergences")
    g = p.add_mutually_exclusive_group(required=True)
    g.add_argument("--file")
    g.add_argument("--type")
    p.set_defaults(f=cmd_divergences)
    p = sub.add_parser("item"); p.add_argument("name"); p.set_defaults(f=cmd_item)
    p = sub.add_parser("refs"); p.add_argument("qn"); p.set_defaults(f=cmd_refs)
    p = sub.add_parser("lucene-callers"); p.add_argument("qn"); p.set_defaults(f=cmd_lucene_callers)
    p = sub.add_parser("no-caller")
    p.add_argument("--subsystem")
    p.add_argument("--unverified", action="store_true")
    p.add_argument("--list")
    p.set_defaults(f=cmd_no_caller)
    sub.add_parser("awaits").set_defaults(f=cmd_awaits)
    sub.add_parser("absent").set_defaults(f=cmd_absent)
    sub.add_parser("tasks").set_defaults(f=cmd_tasks)
    p = sub.add_parser("coverage")
    p.add_argument("--module", choices=sorted(pm.BY_NAME))
    p.add_argument("--list", action="store_true")
    p.set_defaults(f=cmd_coverage)
    p = sub.add_parser("early")
    p.add_argument("--module", choices=sorted(pm.BY_NAME))
    p.set_defaults(f=cmd_early)
    p = sub.add_parser("plans")
    g = p.add_mutually_exclusive_group(required=True)
    g.add_argument("--class", dest="cls")
    g.add_argument("--task", type=int)
    p.set_defaults(f=cmd_plans)
    p = sub.add_parser("skeletons")
    p.add_argument("--module", choices=sorted(pm.BY_NAME))
    p.set_defaults(f=cmd_skeletons)
    p = sub.add_parser("cypher"); p.add_argument("statement"); p.set_defaults(f=cmd_cypher)
    a = ap.parse_args(argv)
    a.f(a)


if __name__ == "__main__":
    main()
