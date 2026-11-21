"""Port evidence: a wrapped lead-in claim, and the audit of stale `ported` states."""
import audit_kg
import port_evidence_kg as pe


def block(*lines):
    return [(i + 1, text) for i, text in enumerate(lines)]


def test_a_lead_in_wrapped_after_the_kind_is_a_claim():
    b = block("Warms a newly merged segment.", "",
              "Equivalent to the functional interface",
              "`org.apache.lucene.index.IndexWriter.IndexReaderWarmer`, which",
              "`IndexWriter.mergeMiddle` calls once per merged segment.")
    assert pe._item_paragraph_claims(b, "MergedSegmentWarmer") == [
        (3, "org.apache.lucene.index.IndexWriter.IndexReaderWarmer")]


def test_the_lead_ins_the_crate_writes():
    for lead in ("the functional interface", "the private functional interface",
                 "the protected abstract class", "the public static nested class",
                 "the abstract class", "the private record"):
        fqn, _ = pe.claim_in(f"Equivalent to {lead} `org.apache.lucene.a.B.C`.")
        assert fqn == "org.apache.lucene.a.B.C", lead


def test_prose_without_a_lead_in_still_needs_the_name_guard():
    b = block("Mirrors what", "`org.apache.lucene.index.IndexWriter.IndexReaderWarmer` does.")
    assert pe._item_paragraph_claims(b, "MergedSegmentWarmer") == []
    b = block("A comparator, equivalent to",
              "`org.apache.lucene.util.ArrayUtil.ByteArrayComparator`.")
    assert pe._item_paragraph_claims(b, "ByteArrayComparator") == [
        (1, "org.apache.lucene.util.ArrayUtil.ByteArrayComparator")]


def test_not_a_type_claim_is_still_refused():
    assert pe.claim_in("Equivalent to the anonymous class `MatchesUtils.forField` returns.") \
        == (None, None)


def test_audit_reports_a_nested_type_ported_without_an_edge():
    rows = [[1, "org.apache.lucene.index.IndexWriter$IndexReaderWarmer"],
            [2, "org.apache.lucene.search.TotalHits$Relation"]]
    q = lambda cypher: rows if "c.portState='ported'" in cypher else []
    assert audit_kg.nested_ported_without_ports(q, {2}) == [
        "org.apache.lucene.index.IndexWriter$IndexReaderWarmer"]
    assert audit_kg.nested_ported_without_ports(q, {1, 2}) == []
