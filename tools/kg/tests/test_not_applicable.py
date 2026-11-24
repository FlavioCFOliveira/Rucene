"""The `notApplicable` port state (rmp #508): the curated verdict file, its
application by `port_coverage_kg`, the coverage status, and the audit."""
import json

import audit_kg
import ask
import port_coverage_kg as pc
import port_modules_kg as pm

TSM = "org.apache.lucene.tests.util.TestSecurityManager"
TOIFF = "org.apache.lucene.tests.util.TestObjectInputFilterFactory"
GJR = "org.locationtech.spatial4j.io.GeoJSONReader"


def test_the_verdict_file_justifies_both_test_framework_classes():
    verdicts = pc.load_not_applicable()
    # and spatial4j's GeoJSONReader, which needs the absent noggit (user decision, #621)
    assert set(verdicts) == {TSM, TOIFF, GJR}
    assert pc.verdict_problems(verdicts) == []
    for qn in (TSM, TOIFF):
        v = verdicts[qn]
        assert any("randomization.gradle" in s for s in v["lucene"])
        assert any("JDK 25" in s or "JEP 486" in s for s in v["jdk"])
    # GeoJSONReader: noggit is absent from Lucene's resolved dependencies, and
    # spatial4j registers the reader only when `org.noggit.JSONParser` loads
    g = verdicts[GJR]
    assert any("versions.lock" in s for s in g["lucene"])
    assert any("SpatialContextFactory.java:250,263-268" in s for s in g["lucene"])
    assert any("Class.forName" in s and "JDK 25" in s for s in g["jdk"])
    assert "#621" in g["reason"]


def test_a_verdict_without_its_sources_is_a_problem():
    assert pc.verdict_problems({"a.B": {"reason": "r", "lucene": ["x"], "jdk": []},
                                "a.C": {"reason": "", "lucene": ["x"], "jdk": ["y"]}}) \
        == [("a.B", "jdk"), ("a.C", "reason")]


def test_rows_only_for_in_scope_classes():
    verdicts = {"a.B": {"reason": "r", "lucene": ["L"], "jdk": ["J"], "rucene": ["R"]},
                "a.Gone": {"reason": "r", "lucene": ["L"], "jdk": ["J"]}}
    rows, unknown = pc.not_applicable_rows(verdicts, {"a.B"})
    assert rows == [{"qn": "a.B", "note": "r", "sources": "L | J | R"}]
    assert unknown == ["a.Gone"]


def test_apply_writes_the_state_with_its_note_and_retires_a_dropped_verdict(
        tmp_path, monkeypatch):
    path = tmp_path / "v.json"
    path.write_text(json.dumps({"notApplicable": {
        "a.B": {"reason": "no Rust form", "lucene": ["L"], "jdk": ["J"]}}}))
    writes = []
    monkeypatch.setattr(pc, "read", lambda _r, _q: [{"qn": "a.B"}, {"qn": "a.Old"}])
    monkeypatch.setattr(pc, "run_unwind",
                        lambda mode, _r, rows, body, _b, label: writes.append((label, rows, body)))
    listed = pc.apply_not_applicable("t", {"a.B"}, "c.gitCommit = 'x'", 100, path)
    assert listed == {"a.B"}
    by_label = {label: (rows, body) for label, rows, body in writes}
    rows, body = by_label["portState = notApplicable"]
    assert rows == [{"qn": "a.B", "note": "no Rust form", "sources": "L | J"}]
    assert "c.portStateNote = row.note" in body
    assert "c.portStateEvidence = 'curated-not-applicable'" in body
    rows, body = by_label["notApplicable retired"]
    assert rows == [{"qn": "a.Old"}] and "REMOVE c.portStateNote" in body


def test_an_unjustified_verdict_stops_the_pass(tmp_path):
    path = tmp_path / "v.json"
    path.write_text(json.dumps({"notApplicable": {"a.B": {"reason": "r", "lucene": ["L"]}}}))
    try:
        pc.apply_not_applicable("t", {"a.B"}, "", 1, path)
    except SystemExit as e:
        assert "a.B" in str(e)
    else:
        raise AssertionError("expected SystemExit")


def test_coverage_reports_not_applicable_as_its_own_status():
    for query in (ask.q_coverage_list("lucene/test-framework"),
                  pm.coverage_query("lucene/test-framework")):
        assert "WHEN c.portState = 'notApplicable' THEN 'notApplicable'" in query


def _audit(rows_na, ported, verdicts):
    def q(cypher):
        if "RETURN c.qualifiedName, c.portScope, c.portStateNote" in cypher:
            return rows_na
        if "[:PORTS]" in cypher:
            return [[c] for c in ported]
        raise AssertionError(cypher)
    results = {}
    audit_kg.not_applicable_audit(q, lambda label, n, want, note="": results.update(
        {label: n == want}), verdicts)
    return results


def test_the_audit_accepts_a_justified_not_applicable_class():
    verdicts = pc.load_not_applicable()
    rows = [[qn, "in", v["reason"], pc.NA_EVIDENCE] for qn, v in verdicts.items()]
    assert all(_audit(rows, [], verdicts).values())


def test_the_audit_requires_a_justification_for_every_not_applicable_class():
    verdicts = {TSM: {"reason": "r", "lucene": ["L"], "jdk": ["J"]}}
    rows = [[TSM, "in", "r", pc.NA_EVIDENCE],
            ["a.NoVerdict", "in", "r", pc.NA_EVIDENCE],
            ["a.NoNote", "in", None, pc.NA_EVIDENCE]]
    res = _audit(rows, [], verdicts)
    assert res["notApplicable classes without a justification"] is False
    assert res["notApplicable classes not in scope"] is True


def test_the_audit_fails_a_ported_or_missing_not_applicable_class():
    verdicts = {TSM: {"reason": "r", "lucene": ["L"], "jdk": ["J"]},
                TOIFF: {"reason": "r", "lucene": ["L"], "jdk": ["J"]}}
    res = _audit([[TSM, "nested", "r", pc.NA_EVIDENCE]], [TSM], verdicts)
    assert res["notApplicable classes not in scope"] is False
    assert res["notApplicable classes with a PORTS edge"] is False
    assert res["not-applicable verdicts without the state"] is False
