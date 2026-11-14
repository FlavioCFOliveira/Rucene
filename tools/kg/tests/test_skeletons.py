"""Sprint 12: `lucene/analysis/common` as a port module, and the skeleton
relation (`SKELETON_OF`, `PLANS_PORT`, `portState = 'skeleton'`). Off the store:
every graph call is replaced by a recorder."""
from pathlib import Path

import ask
import extract_rucene_kg as ex
import port_coverage_kg as pc
import port_evidence_kg as pe
import port_modules_kg as pm

MODEL = Path(pm.__file__).resolve().parents[2] / "knowledge-model.md"

AC = "lucene/analysis/common"
ARABIC = "org.apache.lucene.analysis.ar.ArabicAnalyzer"
STEMMER = "org.tartarus.snowball.ext.EnglishStemmer"
NESTED = "org.apache.lucene.analysis.synonym.SynonymMap$Builder"

# The throwaway fixture: one skeleton, one port, one nested skeleton, one
# skeleton written in a module doc, and a deviating phrase that is neither.
FIXTURE = """//! Skeleton of `org.apache.lucene.analysis.ar.ArabicStemmer`; ported by rmp task #7.

/// Skeleton of `org.apache.lucene.analysis.ar.ArabicAnalyzer`; ported by rmp task #412.
pub struct ArabicAnalyzer;

/// Equivalent to `org.tartarus.snowball.ext.EnglishStemmer`.
pub struct EnglishStemmer;

/// Builds a synonym map.
///
/// Skeleton of
/// `org.apache.lucene.analysis.synonym.SynonymMap.Builder`;
/// ported by rmp task #413.
pub struct Builder;

/// Skeleton of `org.apache.lucene.analysis.ar.ArabicNormalizer`, ported by task 9.
pub struct ArabicNormalizer;
"""


def write(root: Path, rel: str, text: str):
    p = root / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text, encoding="utf-8")
    return p


# --- the module registry ------------------------------------------------------

def test_analysis_common_is_a_port_module_with_its_crate_and_rules():
    m = pm.BY_NAME[AC]
    assert (m["crate"], m["cratePath"], m["jpms"]) == (
        "rucene-analysis-common", "crates/rucene-analysis-common",
        "org.apache.lucene.analysis.common")
    assert m["descriptor"] == "lucene/analysis/common/src/java/module-info.java"
    assert (m["scopeRule"], m["classRule"]) == (
        "sprint-12-ports-the-module", "sprint-12-module-top-level")
    assert AC in pm.ADDED_MODULES and "lucene/core" not in pm.ADDED_MODULES
    assert pm.SKELETON_MODULES == (AC,)
    assert "org.tartarus.snowball" in pm.JAVA_PREFIXES
    assert [d["name"] for d in pm.DECISIONS][-1] == "Sprint 12 ports lucene/analysis/common"
    assert pm.module_of_file(
        "lucene/analysis/common/src/java/org/tartarus/snowball/Among.java") == AC


def test_crate_destination_covers_both_package_roots():
    assert pm.crate_destination(AC, "org.apache.lucene.analysis.ar") \
        == "crates/rucene-analysis-common/src/analysis/ar/"
    assert pm.crate_destination(AC, "org.apache.lucene.collation.tokenattributes") \
        == "crates/rucene-analysis-common/src/collation/tokenattributes/"
    assert pm.crate_destination(AC, "org.tartarus.snowball.ext") \
        == "crates/rucene-analysis-common/src/tartarus/snowball/ext/"
    assert pm.package_root(AC, "org.tartarus.snowball") == "org"
    assert pm.package_root(AC, "org.apache.lucene.analysis.ar") == "org.apache.lucene"
    assert pm.package_root(AC, "com.example") is None
    # the single-root modules are unchanged
    assert pm.crate_destination("lucene/codecs", "org.apache.lucene.codecs.simpletext") \
        == "crates/rucene-codecs/src/simpletext/"


# --- the skeleton phrase ------------------------------------------------------

def test_the_skeleton_phrase_is_matched_whole():
    assert pe.skeleton_in(f"Skeleton of `{ARABIC}`; ported by rmp task #412.") == (ARABIC, 412)
    assert pe.skeleton_in(f"Skeleton of  `{STEMMER}`;\nported by rmp task #5.") == (STEMMER, 5)
    for bad in (f"Skeleton of `{ARABIC}`.",                              # no task
                f"Skeleton of `{ARABIC}`; ported by task #4.",           # not 'rmp task'
                f"Skeleton of `ArabicAnalyzer`; ported by rmp task #4.",  # not a full name
                f"Skeleton of `{ARABIC}`; ported by rmp task #4. More.",  # extra text
                f"See the skeleton of `{ARABIC}`; ported by rmp task #4."):
        assert pe.skeleton_in(bad) is None, bad


def test_a_skeleton_never_yields_a_port_claim(tmp_path):
    write(tmp_path, "src/x.rs", FIXTURE)
    claims = pe.scan_file(tmp_path, "src/x.rs")
    assert [(c["target"], c["lucene"]) for c in claims] == [("EnglishStemmer", STEMMER)]
    # one line and wrapped, on an item named as the Java type
    for line in (f"Skeleton of `{ARABIC}`; ported by rmp task #412.",):
        assert pe.claim_in(line) == (None, None)
    assert pe._item_paragraph_claims([(1, "Skeleton of"), (2, f"`{ARABIC}`;"),
                                      (3, "ported by rmp task #412.")], "ArabicAnalyzer") == []


def test_the_fixture_parses_into_skeletons_and_one_port(tmp_path):
    write(tmp_path, "src/x.rs", FIXTURE)
    sk = pe.scan_skeletons(tmp_path, "src/x.rs")
    assert [(s["scope"], s["target"], s["lucene"], s["task"]) for s in sk] == [
        ("skeleton-module", None, "org.apache.lucene.analysis.ar.ArabicStemmer", 7),
        ("skeleton", "ArabicAnalyzer", ARABIC, 412),
        ("skeleton", "Builder", "org.apache.lucene.analysis.synonym.SynonymMap.Builder", 413)]
    assert sk[1]["line"] == 3 and sk[2]["line"] == 11


def test_extract_reads_skeletons_from_a_member_crate(tmp_path):
    write(tmp_path, "Cargo.toml", '[package]\nname = "rucene"\n')
    write(tmp_path, "src/lib.rs", "")
    write(tmp_path, "crates/rucene-analysis-common/Cargo.toml",
          '[package]\nname = "rucene-analysis-common"\n')
    write(tmp_path, "crates/rucene-analysis-common/src/lib.rs", FIXTURE)
    claims = pe.extract(tmp_path)
    ex.WORKSPACE.clear()
    assert sum(1 for c in claims if c["scope"] == "skeleton") == 2
    assert [c["lucene"] for c in claims if not pe.is_skeleton(c)] == [STEMMER]


# --- resolution and load ------------------------------------------------------

def test_resolution_needs_an_in_scope_class_a_rust_node_and_records_the_task():
    classes, _ = pe.build_class_index([ARABIC, NESTED, "org.apache.lucene.X"])
    scope = {ARABIC: "in", NESTED: "nested", "org.apache.lucene.X": "out"}
    items = {("ArabicAnalyzer", "f.rs"), ("Builder", "f.rs"), ("X", "f.rs")}
    sk = [{"scope": "skeleton", "target": "ArabicAnalyzer", "file": "f.rs", "line": 1,
           "lucene": ARABIC, "task": 412},
          {"scope": "skeleton", "target": "Builder", "file": "f.rs", "line": 2,
           "lucene": "org.apache.lucene.analysis.synonym.SynonymMap.Builder", "task": 999},
          {"scope": "skeleton", "target": "X", "file": "f.rs", "line": 3,
           "lucene": "org.apache.lucene.X", "task": 412},
          {"scope": "skeleton", "target": "Gone", "file": "f.rs", "line": 4,
           "lucene": ARABIC, "task": 412},
          {"scope": "skeleton-module", "target": None, "file": "f.rs", "line": 5,
           "lucene": ARABIC, "task": 412}]
    rows, problems = pe.resolve_skeletons(sk, classes, scope, items, {412})
    assert [(r["name"], r["lucene"], r["task"], r["hasTask"]) for r in rows] == [
        ("ArabicAnalyzer", ARABIC, 412, True), ("Builder", NESTED, 999, False)]
    assert [reason for reason, _c in problems] == [
        "names no Task of the graph", "names no in-scope Class",
        "documents no Rust node of the graph",
        "module-doc skeleton (only an item doc declares one)"]


class Store:
    """The reads `port_evidence_kg` sends, answered from a tiny graph, and every
    write recorded as `(label, rows, body)`."""

    def __init__(self, classes, scope, types, tasks):
        self.classes, self.scope, self.types, self.tasks = classes, scope, types, tasks
        self.writes, self.statements = [], []

    def read(self, _r, query):
        if "RETURN c.qualifiedName, c.portScope" in query:
            return [[c, self.scope[c]] for c in self.classes]
        if query == "MATCH (c:Class) RETURN c.qualifiedName":
            return [[c] for c in self.classes]
        if "t:RustStruct" in query and "RETURN t.name, t.file" in query:
            return [list(t) for t in self.types]
        if "MATCH (t:Task) RETURN t.id" in query:
            return [[t] for t in self.tasks]
        return []

    def unwind(self, _r, _mode, rows, body, label, batch=200):
        self.writes.append((label, [dict(r) for r in rows], body))

    def unwind_items(self, _r, _mode, rows, body, label, _items, batch=None):
        self.writes.append((label, [dict(r) for r in rows], body))

    def rmp(self, _mode, _r, query, *a):
        self.statements.append(query)
        return '{"ok":true}'

    def rows(self, label):
        return [r for lab, rows, _b in self.writes if lab == label for r in rows]


def install(monkeypatch, store):
    monkeypatch.setattr(pe, "kg_read", store.read)
    monkeypatch.setattr(pe, "unwind", store.unwind)
    monkeypatch.setattr(pe, "unwind_items", store.unwind_items)
    monkeypatch.setattr(pe, "kg_rmp", store.rmp)
    monkeypatch.setattr(pe, "item_labels", lambda _r: {})


def test_the_fixture_loads_one_skeleton_of_and_one_ports(tmp_path, monkeypatch):
    """The proof the sprint asked for: a file with one skeleton doc and one
    ported doc loads into exactly one `SKELETON_OF` and one `PORTS`."""
    write(tmp_path, "src/x.rs",
          f"/// Skeleton of `{ARABIC}`; ported by rmp task #412.\npub struct ArabicAnalyzer;\n\n"
          f"/// Equivalent to `{STEMMER}`.\npub struct EnglishStemmer;\n")
    store = Store([ARABIC, STEMMER], {ARABIC: "in", STEMMER: "in"},
                  [("ArabicAnalyzer", "src/x.rs"), ("EnglishStemmer", "src/x.rs")], [412])
    install(monkeypatch, store)
    claims = pe.scan_file(tmp_path, "src/x.rs") + pe.scan_skeletons(tmp_path, "src/x.rs")
    ports = pe.load("r", claims, "c" * 40, "2026-11-13")
    rows, problems = pe.load_skeletons("r", [c for c in claims if pe.is_skeleton(c)],
                                       "c" * 40, "2026-11-13")
    assert [(r["name"], r["lucene"]) for r in ports] == [("EnglishStemmer", STEMMER)]
    assert [(r["name"], r["lucene"]) for r in store.rows("PORTS edges")] == [
        ("EnglishStemmer", STEMMER)]
    assert [(r["name"], r["lucene"], r["task"]) for r in store.rows("SKELETON_OF edges")] == [
        ("ArabicAnalyzer", ARABIC, 412)]
    assert [(r["task"], r["lucene"]) for r in store.rows("PLANS_PORT edges")] == [
        (412, ARABIC)]
    assert problems == []
    # the pass owns both edges: they are cleared before they are written
    assert store.statements == ["MATCH ()-[e:SKELETON_OF]->(:Class) DELETE e",
                                "MATCH (:Task)-[e:PLANS_PORT]->(:Class) DELETE e"]
    body = next(b for lab, _r, b in store.writes if lab == "SKELETON_OF evidence stamped")
    assert "MATCH (t {name:row.name, file:row.file})-[e:SKELETON_OF]->" in body
    assert "e.task = row.task" in body


# --- portState ----------------------------------------------------------------

class Graph:
    def __init__(self, answers):
        self.answers, self.writes = answers, []

    def read(self, _roadmap, query):
        for key, rows in self.answers:
            if key in query:
                return rows
        return []

    def run_unwind(self, _mode, _roadmap, rows, body, _batch, label):
        self.writes.append((label, rows, body))

    def rows(self, label):
        return next(r for lab, r, _b in self.writes if lab == label)


def test_port_state_is_ported_then_skeleton_then_unported(monkeypatch):
    base = "lucene/analysis/common/src/java/org/apache/lucene/analysis/ar/"
    a, b, c = (f"org.apache.lucene.analysis.ar.{n}" for n in ("A", "B", "C"))
    top = {a: base + "A.java", b: base + "B.java", c: base + "C.java"}
    nested = "org.apache.lucene.analysis.ar.A$Inner"
    g = Graph([
        ("RETURN c.qualifiedName AS qn, c.file AS file", [
            {"qn": q, "file": f, "pkg": "org.apache.lucene.analysis.ar"} for q, f in top.items()]
         + [{"qn": nested, "file": top[a], "pkg": "org.apache.lucene.analysis.ar"}]),
        ("portStateNote IS NOT NULL", []),
        ("collect(coalesce(e.claimLevel, 'type'))", [
            {"qn": a, "module": AC, "levels": ["type"]}]),
        ("SKELETON_OF", [{"qn": a}, {"qn": b}, {"qn": nested}]),
    ])
    monkeypatch.setattr(pc, "read", g.read)
    monkeypatch.setattr(pc, "run_unwind", g.run_unwind)
    pc.phase_scope("r", top, "c" * 40, "2026-11-13", 200)
    assert {r["qn"]: r["rule"] for r in g.rows("portScope = in")}[b] == \
        "sprint-12-module-top-level"
    assert [r["qn"] for r in g.rows("portState = ported")] == [a]
    assert [r["qn"] for r in g.rows("portState = skeleton")] == [b]
    assert [r["qn"] for r in g.rows("portState = unported")] == [c]
    (n,) = g.rows("added-module nested portState")
    assert (n["qn"], n["state"], n["evidence"], n["dest"]) == (
        nested, "skeleton", "skeleton-doc-comment", None)


def test_nested_state_prefers_the_port():
    assert pc.nested_state("x", {"x"}, {"x"}) == ("ported", "doc-comment")
    assert pc.nested_state("x", set(), {"x"}) == ("skeleton", "skeleton-doc-comment")
    assert pc.nested_state("x", set(), set()) == ("unported", "no-port-claim")


def test_a_skeleton_type_is_never_a_candidate(monkeypatch, tmp_path):
    survey = tmp_path / "survey.json"
    survey.write_text('{"types": []}')
    g = Graph([])
    monkeypatch.setattr(pc, "read", lambda _r, q: g.writes.append(("read", [], q)) or [])
    monkeypatch.setattr(pc, "run_unwind", g.run_unwind)
    monkeypatch.setattr(pc, "rmp", lambda *a, **k: "{}")
    pc.phase_candidates("r", str(survey), "c" * 40, "2026-11-13", 200)
    (q,) = [b for lab, _r, b in g.writes if lab == "read"]
    assert "NOT c.portState IN ['ported', 'skeleton']" in q


# --- the questions --------------------------------------------------------------

def test_coverage_reports_ported_skeleton_and_unported():
    for q in (pm.COVERAGE_QUERY, ask.q_coverage_list(AC)):
        assert "THEN 'ported' WHEN c.portState = 'skeleton' THEN 'skeleton' " \
               "ELSE 'unported' END AS status" in q
    assert "c.module = 'lucene/analysis/common'" in ask.q_coverage_list(AC)


def test_the_skeleton_questions_are_answerable_and_recorded_in_the_model():
    assert f"{{qualifiedName: '{ARABIC}'}}" in ask.q_plans_of_class(ARABIC)
    assert "(t:Task {id: 412})" in ask.q_classes_of_task("412")
    q = ask.q_skeleton_only(AC)
    assert "c.portState = 'skeleton' AND c.module = 'lucene/analysis/common' " in q
    assert q.count("RETURN ") == 1
    model = MODEL.read_text(encoding="utf-8")
    for q in (ask.PLANS_OF_CLASS, ask.CLASSES_OF_TASK, ask.SKELETON_ONLY):
        assert q in model


def test_ask_parses_the_skeleton_commands(monkeypatch):
    seen = []
    monkeypatch.setattr(ask, "read", lambda _r, q: seen.append(q) or [])
    ask.main(["plans", "--task", "412"])
    ask.main(["plans", "--class", ARABIC])
    ask.main(["skeletons", "--module", AC])
    assert "(t:Task {id: 412})" in seen[0]
    assert ARABIC in seen[1]
    assert "c.module = 'lucene/analysis/common'" in seen[2]
