"""Defect 4: a reconcile or prune never drops a `PORTS` edge or its note silently.

The graph client is replaced by `FakeGraph`, an in-memory node/edge store that
answers exactly the statements `load_rucene_kg.py` issues. It reproduces the
engine quirk the move has to survive: `MERGE (x)-[:T]->(y)` matches an edge of
*any* type between the same ordered pair and then creates nothing.
"""
import json
import re

import pytest

import load_rucene_kg as lo

TYPE_LABELS = {"RustStruct", "RustTrait", "RustEnum", "RustAlias"}
NOTE = "partial: the Lucene enum is a sealed trait here"
PORT_PROPS = {"evidence": "doc-comment", "note": NOTE,
              "declaredAt": "src/a.rs:3", "gitCommit": "f" * 40}


def _value(text):
    text = text.strip()
    if text == "null":
        return None
    if text in ("true", "false"):
        return text == "true"
    if text.startswith("'"):
        return text[1:-1].replace("\\'", "'").replace("\\\\", "\\")
    if text.startswith("["):
        return [_value(x) for x in re.findall(VALUE, text[1:-1])]
    return int(text)


VALUE = r"'(?:[^'\\]|\\.)*'|-?\d+|true|false|null"


class FakeGraph:
    def __init__(self):
        self.nodes = {}  # id -> (labels, props)
        self.edges = []  # [src, dst, type, props]
        self.unwinds = []

    def node(self, nid, labels, **props):
        self.nodes[nid] = (list(labels), props)
        return nid

    def edge(self, src, dst, typ, **props):
        self.edges.append([src, dst, typ, dict(props)])

    def ports(self, src):
        return {(d, dict(p).get("note")): p for s, d, t, p in self.edges
                if s == src and t == "PORTS"}

    def names(self, nid):
        p = self.nodes[nid][1]
        return {"qn": p.get("qualifiedName"), "path": p.get("path"), "name": p.get("name")}

    # -- the two entry points load_rucene_kg uses -----------------------------
    def read(self, _roadmap, q):
        m = re.search(r"MATCH \(x\)-\[e:(\w+)\]->\(y\) WHERE id\(x\) = (\d+) "
                      r"AND id\(y\) = (\d+) RETURN properties\(e\)", q)
        if m:
            typ, x, y = m.group(1), int(m.group(2)), int(m.group(3))
            return [{"props": dict(p)} for s, d, t, p in self.edges
                    if (s, d, t) == (x, y, typ)]
        m = re.search(r"MATCH \(a\)-\[r\]->\(b\) WHERE id\(a\) = (\d+) RETURN "
                      r"type\(r\) AS t, id\(b\) AS other, properties\(r\) AS props", q)
        if m:
            src = int(m.group(1))
            return [{"t": t, "other": d, "props": dict(p), **self.names(d)}
                    for s, d, t, p in self.edges if s == src]
        m = re.search(r"MATCH \(b\)-\[r\]->\(a\) WHERE id\(a\) = (\d+) RETURN "
                      r"type\(r\) AS t, id\(b\) AS other, properties\(r\) AS props", q)
        if m:
            dst = int(m.group(1))
            return [{"t": t, "other": s, "props": dict(p), **self.names(s)}
                    for s, d, t, p in self.edges if d == dst]
        if "RETURN id(n) AS id, labels(n) AS labels" in q:
            return [{"id": i, "labels": lb, "name": p.get("name"), "file": p.get("file"),
                     "qn": p.get("qualifiedName")}
                    for i, (lb, p) in self.nodes.items() if TYPE_LABELS & set(lb)]
        if q.startswith("MATCH (t) WHERE t:RustStruct") and "RETURN t.name AS name, t.file AS file" in q:
            return [{"name": p["name"], "file": p["file"]}
                    for lb, p in self.nodes.values() if TYPE_LABELS & set(lb)]
        if q.startswith("MATCH (f:RustFn) RETURN f.qualifiedName AS qn"):
            return [{"qn": p["qualifiedName"], "id": i, "name": p["name"], "file": p["file"]}
                    for i, (lb, p) in self.nodes.items() if "RustFn" in lb]
        if "e.evidence = 'curated'" in q:
            return []
        if q.startswith("MATCH (t)-[e:PORTS]->(c:Class) WHERE t:RustStruct"):
            return [{"name": self.nodes[s][1]["name"], "file": self.nodes[s][1]["file"],
                     "labels": self.nodes[s][0],
                     "lucene": self.nodes[d][1]["qualifiedName"], "props": dict(p)}
                    for s, d, t, p in self.edges
                    if t == "PORTS" and TYPE_LABELS & set(self.nodes[s][0])]
        if q.startswith("MATCH (f:RustFn)-[e:PORTS]->(c:Class) RETURN"):
            return [{"qn": self.nodes[s][1]["qualifiedName"],
                     "name": self.nodes[s][1]["name"], "file": self.nodes[s][1]["file"],
                     "cid": d, "lucene": self.nodes[d][1]["qualifiedName"],
                     "props": dict(p)}
                    for s, d, t, p in self.edges
                    if t == "PORTS" and "RustFn" in self.nodes[s][0]]
        if "MATCH (f:File)" in q:
            return [{"path": p["path"]} for lb, p in self.nodes.values() if "File" in lb]
        raise AssertionError(f"unexpected read: {q}")

    def rmp(self, _mode, _roadmap, q, _label="", attempts=6):
        m = re.search(r"WHERE id\(x\) = (\d+) WITH x MATCH \(y\) WHERE id\(y\) = (\d+) "
                      r"MERGE \((x|y)\)-\[:(\w+)\]->\((x|y)\)", q)
        if m:
            x, y = int(m.group(1)), int(m.group(2))
            src, dst = (x, y) if m.group(3) == "x" else (y, x)
            # the engine quirk: any edge between the ordered pair satisfies MERGE
            if not any((s, d) == (src, dst) for s, d, _t, _p in self.edges):
                self.edge(src, dst, m.group(4))
            return '{"ok": true}'
        m = re.search(r"MATCH \(x\)-\[e:(\w+)\]->\(y\) WHERE id\(x\) = (\d+) "
                      r"AND id\(y\) = (\d+) SET (.*)$", q, re.S)
        if m:
            typ, x, y = m.group(1), int(m.group(2)), int(m.group(3))
            lit = rf"\[[^\]]*\]|{VALUE}"
            assigns = re.findall(rf"e\.(\w+) = (?:coalesce\(e\.(\w+), ({lit})\)|({lit}))",
                                 m.group(4))
            for e in self.edges:
                if (e[0], e[1], e[2]) == (x, y, typ):
                    for k, ck_, cv, v in assigns:
                        if ck_:  # coalesce(e.k, v): keep a present value
                            assert ck_ == k
                            if e[3].get(k) is None:
                                e[3][k] = _value(cv)
                        else:
                            e[3][k] = _value(v)
            return '{"ok": true}'
        m = re.search(r"MATCH \(n\) WHERE id\(n\) = (\d+) DETACH DELETE n", q)
        if m:
            nid = int(m.group(1))
            del self.nodes[nid]
            self.edges = [e for e in self.edges if nid not in (e[0], e[1])]
            return '{"ok": true}'
        raise AssertionError(f"unexpected write: {q}")

    def run_unwind(self, mode, _roadmap, rows, body, _batch, label):
        self.unwinds.append((label, [dict(r) for r in rows]))
        if "DETACH DELETE t" in body:
            for r in rows:
                for nid, (lb, p) in list(self.nodes.items()):
                    if TYPE_LABELS & set(lb) and (p["name"], p["file"]) == (r["name"], r["file"]):
                        self.rmp("delete", "", f"MATCH (n) WHERE id(n) = {nid} DETACH DELETE n")
        if "MATCH (f:RustFn {qualifiedName:row.qn}) DETACH DELETE f" in body:
            for r in rows:
                for nid, (lb, p) in list(self.nodes.items()):
                    if "RustFn" in lb and p["qualifiedName"] == r["qn"]:
                        self.rmp("delete", "", f"MATCH (n) WHERE id(n) = {nid} DETACH DELETE n")


@pytest.fixture
def g(monkeypatch):
    fake = FakeGraph()
    monkeypatch.setattr(lo, "read", fake.read)
    monkeypatch.setattr(lo, "rmp", fake.rmp)
    monkeypatch.setattr(lo, "run_unwind", fake.run_unwind)
    return fake


def survey(tmp_path, types, fns, files=("src/a.rs",)):
    path = tmp_path / "survey.json"
    path.write_text(json.dumps({
        "commit": "c" * 40, "date": "2026-10-30",
        "files": [{"path": p} for p in files],
        "types": [{"name": n, "file": f, "kind": k, "qualifiedName": f"rucene::a::{n}"}
                  for n, f, k in types],
        "fns": [{"name": n, "file": f, "qualifiedName": qn} for n, f, qn in fns],
        "impls": [], "mods": [], "deps": [],
    }), encoding="utf-8")
    return lo.Survey(str(path))


def test_rekinded_type_keeps_its_port_and_every_property(g, tmp_path, capsys):
    """An enum that became a trait: same {name, file}, new label."""
    g.node(10, ["Class"], qualifiedName="org.apache.lucene.index.Foo")
    g.node(1, ["RustEnum"], name="Foo", file="src/a.rs", qualifiedName="rucene::a::Foo")
    g.node(2, ["RustTrait"], name="Foo", file="src/a.rs", qualifiedName="rucene::a::Foo")
    g.node(20, ["File"], path="src/a.rs")
    g.edge(1, 10, "PORTS", **PORT_PROPS)
    g.edge(20, 1, "DECLARES")
    s = survey(tmp_path, [("Foo", "src/a.rs", "trait")], [("f", "src/a.rs", "rucene::a::f")])
    dropped = []

    lo.phase_reconcile("rucene", s, 200, dropped)

    assert 1 not in g.nodes
    assert g.ports(2) == {(10, NOTE): PORT_PROPS}
    assert [e[:3] for e in g.edges if e[1] == 2] == [[20, 2, "DECLARES"]]
    assert dropped == []
    assert "DROPPED" not in capsys.readouterr().err


def test_a_port_the_engine_will_not_recreate_is_reported(g, tmp_path, capsys):
    """The survivor already has a PORTS_CANDIDATE to the class: MERGE is a no-op."""
    g.node(10, ["Class"], qualifiedName="org.apache.lucene.index.Foo")
    g.node(1, ["RustEnum"], name="Foo", file="src/a.rs", qualifiedName="rucene::a::Foo")
    g.node(2, ["RustTrait"], name="Foo", file="src/a.rs", qualifiedName="rucene::a::Foo")
    g.edge(1, 10, "PORTS", **PORT_PROPS)
    g.edge(2, 10, "PORTS_CANDIDATE", evidence="exact-name-match")
    s = survey(tmp_path, [("Foo", "src/a.rs", "trait")], [("f", "src/a.rs", "rucene::a::f")])
    dropped = []

    lo.phase_reconcile("rucene", s, 200, dropped)

    assert len(dropped) == 1
    rec = dropped[0]
    assert rec["node"] == {"label": "RustEnum", "name": "Foo", "file": "src/a.rs"}
    assert (rec["lucene"], rec["note"], rec["phase"]) == (
        "org.apache.lucene.index.Foo", NOTE, "reconcile")
    err = capsys.readouterr().err
    assert "DROPPED PORTS: RustEnum Foo @ src/a.rs -> org.apache.lucene.index.Foo" in err
    assert NOTE in err


def test_renamed_type_port_is_reported_before_prune_deletes_it(g, tmp_path, capsys):
    g.node(10, ["Class"], qualifiedName="org.apache.lucene.index.Old")
    g.node(1, ["RustStruct"], name="Old", file="src/a.rs", qualifiedName="rucene::a::Old")
    g.node(2, ["RustStruct"], name="New", file="src/a.rs", qualifiedName="rucene::a::New")
    g.node(20, ["File"], path="src/a.rs")
    g.edge(1, 10, "PORTS", **PORT_PROPS)
    s = survey(tmp_path, [("New", "src/a.rs", "struct")], [("f", "src/a.rs", "rucene::a::f")])
    g.node(30, ["RustFn"], name="f", file="src/a.rs", qualifiedName="rucene::a::f")
    dropped = []

    lo.phase_prune("rucene", s, 200, dropped)

    assert 1 not in g.nodes
    assert [(r["node"], r["lucene"], r["note"]) for r in dropped] == [
        ({"label": "RustStruct", "name": "Old", "file": "src/a.rs"},
         "org.apache.lucene.index.Old", NOTE)]
    err = capsys.readouterr().err
    assert "DROPPED PORTS: RustStruct Old @ src/a.rs -> org.apache.lucene.index.Old" in err
    assert NOTE in err


def test_rekinded_fn_port_moves_to_its_unique_successor(g, tmp_path, capsys):
    """A free fn that became a method: qualifiedName changes, file and name do not."""
    g.node(10, ["Class"], qualifiedName="org.apache.lucene.util.Bits")
    g.node(1, ["RustFn"], name="get", file="src/a.rs", qualifiedName="rucene::a::get")
    g.node(2, ["RustFn"], name="get", file="src/a.rs", qualifiedName="rucene::a::Bits::get")
    g.node(3, ["RustStruct"], name="Bits", file="src/a.rs", qualifiedName="rucene::a::Bits")
    g.node(20, ["File"], path="src/a.rs")
    g.edge(1, 10, "PORTS", **PORT_PROPS)
    s = survey(tmp_path, [("Bits", "src/a.rs", "struct")],
               [("get", "src/a.rs", "rucene::a::Bits::get")])
    dropped = []

    lo.phase_prune("rucene", s, 200, dropped)

    assert 1 not in g.nodes
    assert g.ports(2) == {(10, NOTE): PORT_PROPS}
    assert dropped == []
    assert "MOVED PORTS rucene::a::get -> rucene::a::Bits::get" in capsys.readouterr().err


def test_ambiguous_fn_successor_is_reported_not_guessed(g, tmp_path):
    g.node(10, ["Class"], qualifiedName="org.apache.lucene.util.Bits")
    g.node(1, ["RustFn"], name="get", file="src/a.rs", qualifiedName="rucene::a::get")
    g.node(2, ["RustFn"], name="get", file="src/a.rs", qualifiedName="rucene::a::A::get")
    g.node(3, ["RustFn"], name="get", file="src/a.rs", qualifiedName="rucene::a::B::get")
    g.node(4, ["RustStruct"], name="A", file="src/a.rs", qualifiedName="rucene::a::A")
    g.node(20, ["File"], path="src/a.rs")
    g.edge(1, 10, "PORTS", **PORT_PROPS)
    s = survey(tmp_path, [("A", "src/a.rs", "struct")],
               [("get", "src/a.rs", "rucene::a::A::get"),
                ("get", "src/a.rs", "rucene::a::B::get")])
    dropped = []

    lo.phase_prune("rucene", s, 200, dropped)

    assert g.ports(2) == {} and g.ports(3) == {}
    assert len(dropped) == 1 and "2 surviving RustFn" in dropped[0]["reason"]
    assert dropped[0]["node"]["qualifiedName"] == "rucene::a::get"


def test_dropped_report_is_written_as_json_lines(tmp_path):
    out = tmp_path / "sub" / "dropped.jsonl"
    rec = {"phase": "prune", "node": {"label": "RustStruct", "name": "Old",
                                      "file": "src/a.rs"},
           "lucene": "org.apache.lucene.index.Old", "note": NOTE,
           "evidence": "doc-comment", "declaredAt": "src/a.rs:3", "reason": "r"}
    lo.write_dropped_report(out, "c" * 40, [rec])
    lo.write_dropped_report(out, "d" * 40, [rec])
    lines = [json.loads(x) for x in out.read_text(encoding="utf-8").splitlines()]
    assert [x["surveyCommit"] for x in lines] == ["c" * 40, "d" * 40]
    assert lines[0]["note"] == NOTE and lines[0]["lucene"] == "org.apache.lucene.index.Old"


STAMP = {"gitCommit": "e" * 40, "gitDate": "2026-10-01"}


def test_reconcile_keeps_the_properties_of_every_edge_type(g, tmp_path):
    """Incoming and outgoing edges of any type, a list value and a self-loop."""
    g.node(10, ["Class"], qualifiedName="org.apache.lucene.index.Foo")
    g.node(1, ["RustEnum"], name="Foo", file="src/a.rs", qualifiedName="rucene::a::Foo")
    g.node(2, ["RustTrait"], name="Foo", file="src/a.rs", qualifiedName="rucene::a::Foo")
    g.node(3, ["RustTrait"], name="Base", file="src/b.rs", qualifiedName="rucene::b::Base")
    g.node(4, ["RustFn"], name="go", file="src/a.rs", qualifiedName="rucene::a::Foo::go")
    g.node(20, ["File"], path="src/a.rs")
    g.node(30, ["Component"], name="analysis")
    g.edge(20, 1, "DECLARES", **STAMP)
    g.edge(1, 4, "DECLARES", **STAMP)
    g.edge(1, 3, "IMPLEMENTS", **STAMP)
    g.edge(30, 1, "TESTS", note="pins the ordering", **STAMP)
    g.edge(1, 1, "DEPENDS_ON", note="recursive", tags=["a", "b"])
    g.edge(1, 10, "PORTS", **PORT_PROPS)
    s = survey(tmp_path, [("Foo", "src/a.rs", "trait"), ("Base", "src/b.rs", "trait")],
               [("go", "src/a.rs", "rucene::a::Foo::go")], files=("src/a.rs", "src/b.rs"))
    dropped = []

    lo.phase_reconcile("rucene", s, 200, dropped)

    assert 1 not in g.nodes and dropped == []
    got = {(s_, d, t_): p for s_, d, t_, p in g.edges if 2 in (s_, d)}
    assert got[(2, 2, "DEPENDS_ON")] == {"note": "recursive", "tags": ["a", "b"]}
    assert got[(30, 2, "TESTS")] == {"note": "pins the ordering", **STAMP}
    assert got[(20, 2, "DECLARES")] == STAMP and got[(2, 4, "DECLARES")] == STAMP
    assert got[(2, 3, "IMPLEMENTS")] == STAMP
    assert got[(2, 10, "PORTS")] == PORT_PROPS
    assert len(got) == 6


def test_a_non_ports_edge_that_cannot_move_is_reported(g, tmp_path, capsys):
    g.node(1, ["RustEnum"], name="Foo", file="src/a.rs", qualifiedName="rucene::a::Foo")
    g.node(2, ["RustTrait"], name="Foo", file="src/a.rs", qualifiedName="rucene::a::Foo")
    g.node(20, ["File"], path="src/a.rs")
    g.edge(20, 1, "DECLARES", **STAMP)
    g.edge(20, 2, "CONTAINS")  # occupies the (File, survivor) pair: MERGE is a no-op
    s = survey(tmp_path, [("Foo", "src/a.rs", "trait")], [("f", "src/a.rs", "rucene::a::f")])
    dropped = []

    lo.phase_reconcile("rucene", s, 200, dropped)

    assert len(dropped) == 1
    rec = dropped[0]
    assert (rec["edgeType"], rec["direction"], rec["other"], rec["properties"]) == (
        "DECLARES", "in", "src/a.rs", STAMP)
    assert rec["lucene"] is None
    assert "DROPPED DECLARES: RustEnum src/a.rs -> Foo @ src/a.rs" in capsys.readouterr().err


def test_rekind_keeps_the_survivors_canonical_values(g, tmp_path):
    """B3: the survivor already ports the class; stale values only fill gaps."""
    g.node(10, ["Class"], qualifiedName="org.apache.lucene.index.Foo")
    g.node(1, ["RustEnum"], name="Foo", file="src/a.rs", qualifiedName="rucene::a::Foo")
    g.node(2, ["RustTrait"], name="Foo", file="src/a.rs", qualifiedName="rucene::a::Foo")
    g.edge(1, 10, "PORTS", **PORT_PROPS)  # stale: old note, old declaredAt, gitCommit
    canonical = {"evidence": "doc-comment", "note": "the current note",
                 "declaredAt": "src/a.rs:9"}
    g.edge(2, 10, "PORTS", **canonical)
    s = survey(tmp_path, [("Foo", "src/a.rs", "trait")], [("f", "src/a.rs", "rucene::a::f")])
    dropped = []

    lo.phase_reconcile("rucene", s, 200, dropped)

    assert dropped == []
    assert g.ports(2) == {(10, "the current note"): {**canonical,
                                                      "gitCommit": PORT_PROPS["gitCommit"]}}


def test_fn_heir_keeps_its_canonical_values(g, tmp_path):
    """B3 on the prune path: the heir RustFn already ports the class."""
    g.node(10, ["Class"], qualifiedName="org.apache.lucene.util.Bits")
    g.node(1, ["RustFn"], name="get", file="src/a.rs", qualifiedName="rucene::a::get")
    g.node(2, ["RustFn"], name="get", file="src/a.rs", qualifiedName="rucene::a::Bits::get")
    g.node(3, ["RustStruct"], name="Bits", file="src/a.rs", qualifiedName="rucene::a::Bits")
    g.node(20, ["File"], path="src/a.rs")
    g.edge(1, 10, "PORTS", **PORT_PROPS)
    g.edge(2, 10, "PORTS", evidence="curated", note="hand-verified")
    s = survey(tmp_path, [("Bits", "src/a.rs", "struct")],
               [("get", "src/a.rs", "rucene::a::Bits::get")])
    dropped = []

    lo.phase_prune("rucene", s, 200, dropped)

    assert dropped == []
    edge = g.ports(2)[(10, "hand-verified")]
    assert edge["evidence"] == "curated" and edge["note"] == "hand-verified"
    assert edge["declaredAt"] == PORT_PROPS["declaredAt"]  # a missing key is filled


def test_fake_graph_keeps_one_edge_per_pair(g):
    """The engine rule the moves must survive, as the fake models it."""
    g.node(1, ["X"]); g.node(2, ["Y"])
    g.edge(1, 2, "PORTS_CANDIDATE")
    g.rmp("create", "", "MATCH (x) WHERE id(x) = 1 WITH x MATCH (y) WHERE id(y) = 2 "
          "MERGE (x)-[:PORTS]->(y)")
    assert [e[2] for e in g.edges] == ["PORTS_CANDIDATE"]
