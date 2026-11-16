"""Task #516: the KG defects K1-K8 that hid whether the test framework is ported.

One test (or a few) per Rust shape the extractor and the triage now read, each
on a hand-built source tree modelled on the crate file that showed the shape.
"""
import json
from pathlib import Path

import ask
import divergences_kg as dv
import lucene_tests_kg as lt
import port_coverage_kg as pc
import port_evidence_kg as pe
import port_modules_kg as pm
import rust_refs_kg as rr
import rust_surface as rs
import triage_members as tm


# --- the Rust surface ---------------------------------------------------------

def test_a_raw_identifier_fn_is_recorded_with_its_prefix():
    rec = rs.parse("a.rs", "impl P for W {\n    fn r#move(&self) {}\n}\n")
    assert [f["name"] for im in rec["impls"] for f in im["fns"]] == ["r#move"]


def test_struct_fields_are_recorded_at_depth_zero_with_type_and_doc():
    src = ("pub struct Seeds {\n    /// `value()`.\n    pub value: Vec<Seed>,\n"
           "    inner: Option<X>,\n}\n"
           "pub struct R {\n    verify: Arc<dyn Fn(&Method) -> bool + Send + Sync>,\n}\n")
    rec = rs.parse("a.rs", src)
    f = {n: [(x["name"], x["pub"], x["doc"]) for x in t["fields"]]
         for n, t in rec["types"].items()}
    assert f["Seeds"] == [("value", True, "`value()`."), ("inner", False, "")]
    assert rec["types"]["R"]["fields"][0]["ty"].startswith("Arc<dyn Fn(")


def test_a_type_alias_keeps_its_right_hand_side():
    rec = rs.parse("a.rs", "pub type Accept = Arc<dyn Fn(&Field) -> bool + Send>;\n")
    assert rec["types"]["Accept"]["alias"] == "Arc<dyn Fn(&Field) -> bool + Send>"


# --- the member triage --------------------------------------------------------

CLS = "org.apache.lucene.tests.x.Foo"


def triage(tmp_path, files, members, *, kind="class", ports=None, supers=(),
           exclusions=None, extra_classes=()):
    """Run `triage_members.main` over a hand-built snapshot and source tree.

    `files` maps a path under `src/` to its text; `members` is
    `[(name, modifiers, signature)]` of `CLS`, every one of them unmatched.
    Returns `{member: (class, rule, evidence)}`."""
    snap = tmp_path / "snap"
    snap.mkdir(parents=True)
    root = tmp_path / "root"
    for rel, text in files.items():
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)
    ports = ports or [(CLS, "src/foo.rs", "Foo", "PORTS")]
    classes = {CLS} | {c for c, *_ in ports} | {c for c, _ in supers} | set(extra_classes)
    dump = {
        "unmatched": [{"cls": CLS, "method": m} for m, _mods, _sig in members],
        "class_meta": [[c, c.rsplit(".", 1)[0], c.rsplit(".", 1)[1],
                        kind if c == CLS else "class", ""] for c in sorted(classes)],
        "ports_any": [[c, f, n, t] for c, f, n, t in ports],
        "supers": [list(s) for s in supers],
        "supers_all": [list(s) for s in supers],
        "member_kinds": [[CLS, m, "method", mods, sig, f"{CLS}#{m}", "void"]
                         for m, mods, sig in members],
    }
    for name, rows in dump.items():
        (snap / f"{name}.json").write_text(json.dumps(rows))
    ex = tmp_path / "exclusions.json"
    ex.write_text(json.dumps(exclusions or {}))
    out = tmp_path / "out.json"
    import sys
    argv = sys.argv
    sys.argv = ["triage_members.py", str(snap), str(out), "--root", str(root),
                "--exclusions", str(ex)]
    try:
        tm.main()
    finally:
        sys.argv = argv
    return {r["method"]: (r["class"], r["rule"], r["evidence"])
            for r in json.loads(out.read_text())}


def test_annotation_members_are_the_port_types_fields(tmp_path):
    src = "pub struct Foo {\n    pub value: Vec<String>,\n}\n"
    got = triage(tmp_path, {"src/foo.rs": src}, [("value", "", "String[] value()")],
                 kind="annotation")
    assert got["value"][:2] == ("renamed", "annotation-member[own]")


def test_an_overridable_hook_is_a_closure_field_even_through_an_alias(tmp_path):
    src = ("pub type Accept = Arc<dyn Fn(&Field) -> bool + Send + Sync>;\n"
           "pub struct Foo {\n    accept: Accept,\n    other: u32,\n}\n")
    got = triage(tmp_path, {"src/foo.rs": src},
                 [("accept", "protected", "boolean accept(Field field)"),
                  ("other", "protected", "int other()")])
    assert got["accept"][:2] == ("renamed", "closure-hook[own]")
    assert got["other"][0] == "absent"   # a plain field is not a hook


def test_a_raw_identifier_answers_a_keyword_member(tmp_path):
    src = "pub struct Foo;\nimpl Foo {\n    pub fn r#move(&self) {}\n}\n"
    got = triage(tmp_path, {"src/foo.rs": src}, [("move", "public", "void move(Path a)")])
    assert got["move"][:2] == ("renamed", "rust-keyword[own]")


def test_to_string_needs_a_hand_written_debug_never_a_derived_one(tmp_path):
    hand = "pub struct Foo;\nimpl std::fmt::Debug for Foo {\n    fn fmt(&self) {}\n}\n"
    got = triage(tmp_path, {"src/foo.rs": hand}, [("toString", "public", "String toString()")])
    assert got["toString"][:2] == ("renamed", "debug-impl[own]")
    derived = "#[derive(Debug)]\npub struct Foo;\n"
    got = triage(tmp_path / "d", {"src/foo.rs": derived},
                 [("toString", "public", "String toString()")])
    assert got["toString"][0] == "absent"


def test_a_doc_that_opens_with_the_member_is_its_port_unless_it_denies_it(tmp_path):
    src = ("pub struct Foo;\nimpl Foo {\n"
           "    /// `toString()`: `delegate.toString()`.\n    pub fn store_text(&self) {}\n"
           "    /// Overrides `verify(Method)`: `false` skips a method.\n"
           "    pub fn with_verify(self) {}\n"
           "    /// `close()` is not ported: nothing to release.\n    pub fn release(&self) {}\n}\n")
    got = triage(tmp_path, {"src/foo.rs": src},
                 [("toString", "public", "String toString()"),
                  ("verify", "protected", "boolean verify(Method m)"),
                  ("close", "public", "void close()")])
    assert got["toString"][:2] == ("renamed", "doc-lead[own]")
    assert got["verify"][:2] == ("renamed", "doc-lead[own]")
    assert got["close"][0] == "absent"


def test_a_java_parenthesis_on_a_declaration_names_the_member(tmp_path):
    src = ("pub struct Foo;\n/// The metadata of the field being written (Java's\n"
           "/// `setField(FieldMetadata)`).\npub struct FieldContext;\n")
    got = triage(tmp_path, {"src/foo.rs": src}, [("setField", "", "void setField(FieldMetadata m)")])
    assert got["setField"][:2] == ("renamed", "doc-lead[file]")


def test_a_non_public_helper_inlined_or_shared_is_its_call_site(tmp_path):
    files = {"src/foo.rs": "pub struct Foo;\nfn go() {\n    read_line(input, scratch)?;\n}\n",
             "src/util.rs": "pub fn read_line() {}\n"}
    got = triage(tmp_path, files,
                 [("readLine", "private", "void readLine()"),
                  ("other", "public", "void readLineX()")])
    assert got["readLine"][:2] == ("renamed", "inlined-call[file]")
    assert got["other"][0] == "absent"


def test_an_inline_marker_comment_names_the_inlined_member(tmp_path):
    src = ("pub struct Foo;\nimpl Foo {\n    fn fields_consumer(&self) {\n"
           "        with(|s| {\n            // createFieldsConsumer\n            build(s)\n"
           "        })\n    }\n}\n")
    got = triage(tmp_path, {"src/foo.rs": src},
                 [("createFieldsConsumer", "protected", "FieldsConsumer createFieldsConsumer()")])
    assert got["createFieldsConsumer"][:2] == ("renamed", "inline-marker[file]")


def test_a_junit_hook_registered_by_name_in_the_class_model(tmp_path):
    src = 'pub struct Foo;\nfn class() {\n    b.method(Method::new("afterClass"))\n}\n'
    got = triage(tmp_path, {"src/foo.rs": src}, [("afterClass", "public static", "void afterClass()")])
    assert got["afterClass"][:2] == ("renamed", "reflective-method[file]")


def test_a_factory_answered_by_a_default_bound_in_the_supertypes_file(tmp_path):
    base = "org.apache.lucene.tests.x.Base"
    files = {"src/foo.rs": "pub struct Foo;\n",
             "src/base.rs": "pub struct Base;\npub trait LookaheadPosition: Resettable + Default {}\n"}
    got = triage(tmp_path, files, [("newPosition", "protected", "Position newPosition()")],
                 ports=[(CLS, "src/foo.rs", "Foo", "PORTS"), (base, "src/base.rs", "Base", "PORTS")],
                 supers=[(CLS, base)])
    assert got["newPosition"][:2] == ("renamed", "default-factory[file]")


def test_a_declared_delegation_answers_through_the_delegate(tmp_path):
    files = {"src/foo.rs": ("/// **Divergence.** Java's `Foo` wraps a path; this port's\n"
                            "/// [`Path`] carries its filesystem, and every method delegates.\n"
                            "pub struct Foo;\n"),
             "src/path.rs": ("#[derive(PartialEq)]\npub struct Path;\nimpl Path {\n"
                             "    pub fn resolve(&self) {}\n    pub fn iter_names(&self) {}\n}\n")}
    got = triage(tmp_path, files, [("resolve", "public", "Path resolve(Path o)"),
                                   ("iterator", "public", "Iterator<Path> iterator()"),
                                   ("equals", "public", "boolean equals(Object o)")])
    assert got["resolve"][:2] == ("renamed", "delegated[divergence]")
    assert got["iterator"][:2] == ("renamed", "delegated[divergence]")
    assert got["equals"][:2] == ("renamed", "delegated[divergence]")


def test_a_declared_divergence_naming_the_member_is_not_open_work(tmp_path):
    src = ("/// **Divergences.** `toString`, `equals` and `hashCode` (which Java\n"
           "/// leaves to `Object`) are not reproduced.\npub struct Foo;\n")
    got = triage(tmp_path, {"src/foo.rs": src}, [("hashCode", "public", "int hashCode()")])
    assert got["hashCode"][:2] == ("na", "declared-divergence[file]")


def test_a_supertype_divergence_binds_only_when_it_names_this_type(tmp_path):
    base = "org.apache.lucene.tests.x.Base"
    para = ("/// **Divergence from Lucene 10.5.0: the factories are inlined.** Java calls\n"
            "/// `createLine()`, and `{who}` overrides it.\npub struct Base;\n")
    for who, verdict in (("Foo", "na"), ("Other", "absent")):
        files = {"src/foo.rs": "pub struct Foo;\n", "src/base.rs": para.format(who=who)}
        got = triage(tmp_path / who, files, [("createLine", "protected", "Line createLine()")],
                     ports=[(CLS, "src/foo.rs", "Foo", "PORTS"),
                            (base, "src/base.rs", "Base", "PORTS")],
                     supers=[(CLS, base)])
        assert got["createLine"][0] == verdict


def test_the_implemented_traits_module_divergence_list_covers_jdk_members(tmp_path):
    files = {"src/foo.rs": "pub struct Foo;\nimpl FileStore for Foo {}\n",
             "src/nio.rs": ("//! **Divergence.** The differences that span the module are:\n//!\n"
                            "//! * the attribute-view hierarchy (`getFileStoreAttributeView`)\n"
                            "//!   has no counterpart.\n\npub trait FileStore {}\n")}
    got = triage(tmp_path, files,
                 [("getFileStoreAttributeView", "public", "V getFileStoreAttributeView(Class c)")])
    assert got["getFileStoreAttributeView"][:2] == ("na", "declared-divergence[trait]")


def test_a_claim_on_a_sibling_files_function_is_the_port(tmp_path):
    files = {"src/x/foo.rs": "pub struct Foo;\n",
             "src/x/frame.rs": ("pub struct Ctx;\nimpl Ctx {\n"
                                "    /// Port of `Foo.initIndexInput()`.\n"
                                "    pub fn init_index_input(&mut self) {}\n}\n")}
    got = triage(tmp_path, files, [("initIndexInput", "", "void initIndexInput()")],
                 ports=[(CLS, "src/x/foo.rs", "Foo", "PORTS")])
    assert got["initIndexInput"][:2] == ("renamed", "doc-comment[sibling]")


def test_a_curated_member_verdict_carries_its_reason(tmp_path):
    ex = {CLS: {"memberVerdicts": {"wrap": {"triageClass": "renamed",
                                            "triageReason": "wrap is X::new (a.rs:1)"}}}}
    got = triage(tmp_path, {"src/foo.rs": "pub struct Foo;\n"},
                 [("wrap", "protected final", "Path wrap(Path o)")],
                 exclusions=ex)
    assert got["wrap"] == ("renamed", "curated", "wrap is X::new (a.rs:1)")


def test_doc_lead_and_paren_recognisers():
    assert tm.doc_lead("`toString()`: the delegate's text.") == (None, "toString")
    assert tm.doc_lead("The abstract `isOK(Throwable t)` a subclass defines.") == (None, "isOK")
    assert tm.doc_lead("Port of `Foo.bar(int)`.") == ("Foo", "bar")
    assert tm.doc_lead("x (Java's `setField`).") == (None, "setField")
    assert tm.doc_lead("`close()` is not ported.") is None
    assert tm.doc_lead("Reads the `close()` of the delegate.") is None


# --- nested types of the added modules (K1) -----------------------------------

OUTER = "org.apache.lucene.tests.x.Outer"


def test_comment_mention_reads_wrapped_and_qualified_spans_and_refuses_std_names():
    text = ("fn f() {\n    // `new Inner(..., seed ^\n    // name.hashCode())`.\n}\n"
            "/// A value: `Option<u32>`.\n")
    assert pc.comment_mention(text, "Outer", "Inner")[0] == 2
    assert pc.comment_mention("/// `Outer.Option`: which features.\n", "Outer", "Option")[0] == 1
    assert pc.comment_mention(text, "Outer", "Option") is None


def test_enum_variant_fold_needs_the_base_named_and_the_variant_declared():
    text = ("/// The kind of a node (one Java `Node` subclass each).\nenum Kind {\n"
            "    Eos,\n    And,\n}\n")
    sib = {f"{OUTER}$Node", f"{OUTER}$AndNode", f"{OUTER}$OrNode"}
    assert pc.enum_variant_fold(f"{OUTER}$AndNode", sib, ["a.rs"], lambda f: text)[2:] == \
        ("And", "Node")
    assert pc.enum_variant_fold(f"{OUTER}$OrNode", sib, ["a.rs"], lambda f: text) is None


def test_classify_added_nested_applies_the_evidence_in_order():
    rows = [{"qn": f"{OUTER}${n}", "module": "lucene/test-framework", "pkg": "org.apache.lucene.tests.x"}
            for n in ("Claimed", "Same", "Named", "Curated", "Missing")]
    host = {OUTER: {"a.rs"}}
    items = {("Same", "a.rs"): "RustStruct"}
    text = "// `Outer.Named` lives in this closure\n"
    verdicts = {"absorbed": {f"{OUTER}$Curated": {"host": "H", "where": "a.rs:9", "why": "w"}},
                "unported": {}}
    out, edges = pc.classify_added_nested(rows, {f"{OUTER}$Claimed"}, set(), host, items,
                                          lambda f: text, verdicts)
    st = {o["qn"].rsplit("$", 1)[1]: (o["state"], o["evidence"]) for o in out}
    assert st == {"Claimed": ("ported", "doc-comment"),
                  "Same": ("ported", "host-file-name"),
                  "Named": ("absorbed", "host-comment-mention"),
                  "Curated": ("absorbed", "verified-absorption"),
                  "Missing": ("unported", "no-port-claim")}
    assert edges == [{"name": "Same", "file": "a.rs", "label": "RustStruct",
                      "qn": f"{OUTER}$Same"}]
    assert [o["dest"] for o in out if o["state"] == "unported"] == ["crates/rucene-test-framework/src/x/"]


def test_enclosing_chain_is_innermost_first():
    assert pc.enclosing_chain("p.A$B$C") == ["p.A$B", "p.A"]
    assert pc.enclosing_chain("p.A") == []


# --- nested-type members are measured and triaged (K2) ------------------------

def test_depth_and_triage_read_nested_types_with_a_type_level_port():
    assert "c.portScope = 'nested' AND coalesce(e.claimLevel, 'type') = 'type'" in pe.DEPTH_PAIRS
    assert "c.portScope IN ['in', 'nested']" in pe.DEPTH_MEMBERS
    # `triage_dump.py` is a script (it writes on import), so its source is read.
    src = (Path(pe.__file__).parent / "triage_dump.py").read_text()
    assert '"pairs": DEPTH_PAIRS' in src and '"members": DEPTH_MEMBERS' in src
    for k in ("stored", "ports_any", "supers", "member_kinds", "class_meta"):
        line = next(x for x in src.splitlines() if x.strip().startswith(f'"{k}":'))
        assert "portScope IN ['in', 'nested']" in line, k
    assert "c.portScope IN ['in', 'nested']" in ask.Q_ABSENT


# --- the framework's tests and resources (K4) and ruceneCoverage (K5) ----------

def test_a_test_class_counterpart_is_a_crate_file_whose_comments_name_it():
    texts = {"a.rs": "//! Port of `com.carrotsearch.randomizedtesting.TestBiasedNumbers`\n",
             "b.rs": "fn x() {}\n// TestLeakFS\n",
             "c.rs": "let s = \"TestLeakFS\";\n// TestLeakFSX\n"}
    assert lt.test_class_counterparts("TestBiasedNumbers", texts) == ["a.rs"]
    assert lt.test_class_counterparts("TestLeakFS", texts) == ["b.rs"]


def test_a_resource_counterpart_is_a_copied_file_or_the_source_naming_it():
    names = {"lucene-9417.geojson.gz": ["crates/t/src/geo/resources/lucene-9417.geojson.gz"]}
    texts = {"crates/t/src/spi.rs": "// META-INF/services/org.apache.lucene.codecs.Codec\n"}
    geo = {"name": "lucene-9417.geojson.gz", "resource": "org/apache/lucene/tests/geo/lucene-9417.geojson.gz"}
    spi = {"name": "org.apache.lucene.codecs.Codec",
           "resource": "META-INF/services/org.apache.lucene.codecs.Codec"}
    none = {"name": "x.bin", "resource": "x.bin"}
    assert lt.resource_counterpart(geo, names, texts) == (names["lucene-9417.geojson.gz"][0], "same-file-name")
    assert lt.resource_counterpart(spi, names, texts) == ("crates/t/src/spi.rs", "named-by-source")
    assert lt.resource_counterpart(none, names, texts) == (None, None)


def test_framework_classes_are_marked_not_applicable(monkeypatch):
    seen = []
    monkeypatch.setattr(lt, "rmp", lambda _m, _r, q, *a: seen.append(q))
    lt.mark_framework_not_applicable("r", 200)
    assert "SET c.ruceneCoverage = 'not-applicable'" in seen[0]
    assert "c.testKind = 'framework'" in seen[0]
    assert "e.evidence = 'ported-type-file-has-tests' DELETE e" in seen[1]


def test_the_framework_trees_are_the_port_modules_tests_and_resources():
    trees = {(m, t) for m, _r, t, _k in lt.FRAMEWORK_TEST_TREES}
    assert ("lucene/test-framework", "lucene/test-framework/src/test") in trees
    assert ("randomizedtesting/randomized-runner", "randomized-runner/src/test/java") in trees
    assert lt.FRAMEWORK_RESOURCE_TREES[0][2] == "lucene/test-framework/src/resources"


# --- divergences in every declared form (K6/K8) --------------------------------

def test_heading_and_plain_divergences_are_read():
    text = ("/// Merges graphs.\n///\n/// # Divergences from Lucene 10.5.0\n///\n"
            "/// The port rebuilds (`HnswGraph.java:10-20`).\n///\n/// * one bullet\n"
            "///\n/// # Examples\n/// text\npub struct Merger;\n\n"
            "/// Divergence: Lucene declares this method on the base.\npub fn f() {}\n")
    got = dv.scan_text("src/m.rs", text)
    assert [(d["line"], d["title"], d["item"]) for d in got] == [
        (3, "Divergences from Lucene 10.5.0", "Merger"), (13, "Divergence", "f")]
    assert "one bullet" in got[0]["text"] and "text" not in got[0]["text"].split("bullet")[1]
    assert got[0]["javaRefs"] == [("HnswGraph.java", "10-20")]


# --- crate references across the workspace (K8) --------------------------------

def test_workspace_crate_symbols_resolve_to_their_crates_items(monkeypatch):
    monkeypatch.setitem(rr.PACKAGES, "randomizedtesting", "randomizedtesting")
    sym = "rust-analyzer cargo randomizedtesting 0.1.0 deadline_clock/DeadlineClock#new()."
    assert rr.is_crate_symbol(sym) and rr.crate_ident(sym) == "randomizedtesting"
    assert rr.item_of(sym) == ("fn", "DeadlineClock", "new", ("deadline_clock",))
    assert not rr.is_crate_symbol("rust-analyzer cargo tempfile 3.27.0 a/B#")
    kg = rr.KgIndex([], [("randomizedtesting::deadline_clock::DeadlineClock::new",
                          "crates/randomizedtesting/src/deadline_clock.rs", "DeadlineClock", "new")], [])
    assert kg.node(rr.item_of(sym), None, rr.crate_ident(sym)) == \
        ("RustFn", "randomizedtesting::deadline_clock::DeadlineClock::new")


def test_production_paths_include_member_crates(monkeypatch):
    monkeypatch.setattr(rr, "member_of", lambda p: ("crates/r", "r", "r")
                        if p.startswith("crates/r/") else None)
    assert rr.is_production_path("src/a.rs")
    assert rr.is_production_path("crates/r/src/a.rs")
    assert not rr.is_production_path("crates/r/tests/a.rs")


# --- fully qualified lookups and the coverage verdict (K7) ---------------------

def test_class_keys_map_dotted_nested_names_to_the_graph_key():
    assert ask.class_keys("org.apache.lucene.tests.util.fst.FSTTester.InputOutput") == [
        "org.apache.lucene.tests.util.fst.FSTTester$InputOutput",
        "org.apache.lucene.tests.util.fst.FSTTester.InputOutput"]
    assert ask.class_keys("p.A$B") == ["p.A$B"]


def test_item_routes_a_dotted_name_to_the_class_lookup(monkeypatch):
    seen = []
    monkeypatch.setattr(ask, "read", lambda _r, q: seen.append(q) or [])
    ask.main(["item", "com.carrotsearch.randomizedtesting.DeadlineClock"])
    ask.main(["item", "DeadlineClock"])
    assert "c.qualifiedName IN ['com.carrotsearch.randomizedtesting.DeadlineClock']" in seen[0]
    assert "WHERE t.name = 'DeadlineClock'" in seen[1]


def test_the_outer_inner_colon_doc_form_is_a_claim_under_the_name_guard():
    assert pe._item_paragraph_claims(
        [(1, "`FSTTester.InputOutput`: holds one input/output pair.")], "InputOutput") == \
        [(1, "FSTTester.InputOutput")]
    assert pe._item_paragraph_claims([(1, "`FSTTester.Other`: x.")], "InputOutput") == []
    assert pe._item_paragraph_claims([(1, "`FSTTester.InputOutput`: not ported.")],
                                     "InputOutput") == []


def test_fully_ported_needs_every_class_nested_type_and_member():
    top = [("m", "ported", 3), ("m", "notApplicable", 1), ("n", "unported", 1)]
    nested = [("m", "ported", 2), ("m", "absorbed", 1), ("n", "unported", 2)]
    absent = [("n", 1, 4)]
    v = pm.fully_ported(top, nested, absent)
    assert v["m"]["fullyPorted"] is True
    assert (v["n"]["topLevelOpen"], v["n"]["nestedOpen"], v["n"]["absentMembers"],
            v["n"]["fullyPorted"]) == (1, 2, 4, False)


def test_the_coverage_queries_narrow_to_one_module_and_the_model_holds_them():
    from pathlib import Path
    q = pm.nested_coverage_query("lucene/codecs")
    assert "c.portScope = 'nested' AND c.module = 'lucene/codecs'" in q
    assert "c.module = 'lucene/codecs'" in pm.member_absent_query("lucene/codecs")
    model = (Path(pm.__file__).resolve().parents[2] / "knowledge-model.md").read_text()
    for q in (pm.NESTED_COVERAGE_QUERY, pm.MEMBER_ABSENT_QUERY, ask.Q_ABSENT,
              pe.DEPTH_PAIRS, pe.DEPTH_MEMBERS):
        assert q in model, q


# --- shapes found once nested-type members were triaged -----------------------

def test_a_newtype_wrapping_the_port_type_answers_for_it(tmp_path):
    src = ("pub struct Foo;\n/// The `Terms` the fields hand out: the `Foo` itself.\n"
           "struct FooTerms(Arc<Foo>);\nimpl Terms for FooTerms {\n"
           "    fn size(&self) -> i64 { 0 }\n}\n")
    got = triage(tmp_path, {"src/foo.rs": src}, [("size", "public", "long size()")])
    assert got["size"][:2] == ("renamed", "exact[own]")


def test_a_fn_pointer_field_is_a_hook_too(tmp_path):
    src = "pub struct Foo {\n    populate_doc: fn(&mut Document),\n}\n"
    got = triage(tmp_path, {"src/foo.rs": src}, [("populateDoc", "", "void populateDoc(Document d)")])
    assert got["populateDoc"][:2] == ("renamed", "closure-hook[own]")


def test_a_colon_parenthesis_glosses_a_value_by_its_member():
    assert tm.doc_lead("its key (`compareSecondary`: the field number).") == \
        (None, "compareSecondary")
    assert tm.doc_lead("x (`foo` bar).") is None


def test_a_tests_doc_is_not_a_claim(tmp_path):
    src = ("pub struct Foo;\n/// `Foo.abort()`: a rollback aborts it.\n#[test]\n"
           "fn a_rollback_aborts() {}\n")
    got = triage(tmp_path, {"src/foo.rs": src}, [("abort", "public", "void abort()")])
    assert got["abort"][0] == "absent"


def test_a_nested_types_doc_claims_use_its_own_simple_name(tmp_path):
    global CLS
    old, CLS = CLS, "org.apache.lucene.tests.x.Outer$Inner"
    try:
        src = ("pub struct Inner;\nimpl Inner {\n"
               "    /// Port of `Inner.toString()`.\n    pub fn to_java_string(&self) {}\n}\n")
        got = triage(tmp_path, {"src/foo.rs": src}, [("toString", "public", "String toString()")],
                     ports=[(CLS, "src/foo.rs", "Inner", "PORTS")])
        assert got["toString"][:2] == ("renamed", "doc-comment[own]")
    finally:
        CLS = old


def test_a_bold_lead_that_wraps_is_one_divergence():
    text = ("/// Port of `BlockWriter(...)`.\n///\n/// **Divergence from Lucene 10.5.0: the factories are\n"
            "/// inlined.** Java calls `createLine()` (`BlockWriter.java:90-92`).\npub fn new() {}\n")
    got = dv.scan_text("src/w.rs", text)
    assert [(d["line"], d["title"]) for d in got] == [
        (3, "Divergence from Lucene 10.5.0: the factories are inlined")]
