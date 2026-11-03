"""The 2026-11-03 KG additions: declared divergences, #269 verdicts, labelled
routing of the Rust item pattern, and the rmp closings export."""
import divergences_kg as dv
import kgio
import no_caller_verdicts_kg as nv
import rmp_closings as rc


SRC = """\
//! Module docs.
//!
//! **Divergence from Lucene 10.5.0.** The whole module is one file, where
//! Lucene splits it across `Foo.java:10-20` and `Bar.java`.

/// A wrapper.
///
/// **Divergence from Lucene 10.5.0.** This port wraps the delegate directly,
/// see `FilteredTermsEnum.java:42`.
///
/// Trailing paragraph that is not part of it.
#[derive(Debug)]
pub struct Wrapper {
    /// **Divergence:** the field is a `u16`.
    pub unit: u16,
}

impl Wrapper {
    /// **Divergences from Lucene 10.5.0** in the error class.
    pub fn new() -> Self {
        Self { unit: 0 }
    }
}

impl Clone for Wrapper {
    /// **Divergence from Lucene 10.5.0.** Clones share nothing.
    fn clone(&self) -> Self {
        Self { unit: self.unit }
    }
}
"""


def test_scan_finds_each_paragraph_with_its_item_and_java_refs():
    divs = dv.scan_text("src/x.rs", SRC)
    assert [(d["line"], d["itemKind"], d["item"]) for d in divs] == [
        (3, "module", None),
        (8, "struct", "Wrapper"),
        (14, "member", "unit"),
        (19, "fn", "new"),
        (26, "fn", "clone"),
    ]
    assert divs[0]["javaRefs"] == [("Foo.java", "10-20"), ("Bar.java", "")]
    assert divs[1]["javaRefs"] == [("FilteredTermsEnum.java", "42")]
    assert "Trailing paragraph" not in divs[1]["text"]
    assert divs[1]["id"] == "src/x.rs:8"
    assert divs[3]["owner"] == "Wrapper" and not divs[3]["traitImpl"]
    assert divs[4]["owner"] == "Wrapper" and divs[4]["traitImpl"]
    assert divs[3]["title"] == "Divergences from Lucene 10.5.0"


def test_items_resolve_only_when_unambiguous_and_modelled():
    divs = dv.scan_text("src/x.rs", SRC)
    types = {("RustStruct", "Wrapper", "src/x.rs"): "rucene::x::Wrapper"}
    fns = {("new", "src/x.rs"): [("Wrapper", "rucene::x::Wrapper::new"),
                                 ("Other", "rucene::x::Other::new")],
           ("clone", "src/x.rs"): [("Wrapper", "rucene::x::Wrapper::clone")]}
    edges = dv.resolve_items(divs, types, fns)
    assert ("src/x.rs:8", "RustStruct", "rucene::x::Wrapper") in edges
    assert ("src/x.rs:19", "RustFn", "rucene::x::Wrapper::new") in edges
    # a method of `impl Trait for Type` is not modelled as a RustFn
    assert not [e for e in edges if e[0] == "src/x.rs:26"]


def test_java_refs_resolve_to_one_top_level_type_or_none():
    divs = dv.scan_text("src/x.rs", SRC)
    classes = {"Foo.java": ["org.apache.lucene.Foo"], "Bar.java": [],
               "FilteredTermsEnum.java": ["org.apache.lucene.index.FilteredTermsEnum"]}
    assert dv.resolve_java(divs, classes) == [
        ("src/x.rs:3", "org.apache.lucene.Foo", "10-20"),
        ("src/x.rs:8", "org.apache.lucene.index.FilteredTermsEnum", "42"),
    ]


TABLE = """\
| item | auto | final | action | Java evidence |
|---|---|---|---|---|
| `codecs::FilterCodec` | W? | **P** | none | FilterCodec.java:31 a | b |
| `util::Gone::f` | D? | **D** | removed | absent |
| `index::Blocked::g` | W? | **W-blocked** | caller unported | Caller.java:9 |
| not a row |
"""


def test_verdict_tables_parse_rows_and_keep_pipes_in_evidence():
    rows = nv.parse_table("block9.md", TABLE)
    assert [(r["item"], r["verdict"]) for r in rows] == [
        ("rucene::codecs::FilterCodec", "P"),
        ("rucene::util::Gone::f", "D"),
        ("rucene::index::Blocked::g", "W-blocked"),
    ]
    assert rows[0]["evidence"] == "FilterCodec.java:31 a | b"


def test_a_later_table_overrides_an_earlier_verdict():
    later = "| `codecs::FilterCodec` | W? | **T** | test-only | x |\n"
    v = nv.latest_verdicts([("block1.md", TABLE), ("block2.md", later)])
    assert v["rucene::codecs::FilterCodec"]["verdict"] == "T"
    assert v["rucene::codecs::FilterCodec"]["table"] == "block2.md"


def test_rows_are_routed_to_every_label_their_name_and_file_carry():
    index = {("A", "src/a.rs"): {"RustStruct"}, ("B", "src/b.rs"): {"RustFn", "RustStruct"}}
    rows = [{"name": "A", "file": "src/a.rs"}, {"name": "B", "file": "src/b.rs"},
            {"name": "C", "file": "src/c.rs"}]
    routed = kgio.route_items(rows, index)
    assert routed == {"RustStruct": [rows[0], rows[1]], "RustFn": [rows[1]]}


def test_unwind_items_refuses_a_body_without_the_item_pattern():
    try:
        kgio.unwind_items("r", "update", [], "MATCH (t:RustFn) RETURN t", "x", {})
    except ValueError:
        return
    raise AssertionError("expected ValueError")


def test_closings_keep_only_tasks_with_a_closing_commit():
    tasks = [{"id": 3, "commit_close": "abc"}, {"id": 1, "commit_close": None},
             {"id": 2, "commit_close": "def"}]
    assert rc.closings(tasks) == [{"task": 2, "commit_close": "def"},
                                  {"task": 3, "commit_close": "abc"}]


def test_a_cfg_test_module_with_a_custom_name_is_a_test_span():
    import rust_refs_kg as rr
    lines = ["fn prod() {}", "", "#[cfg(test)]", "mod needs_refresh_tests {",
             "    fn segment() {}", "}"]
    # rust-analyzer's enclosing range of the module starts at `mod` (line 3)
    assert rr._span_is_test((3, 0, 5, 1), (3, 4), lines, "needs_refresh_tests")
    assert not rr._span_is_test((0, 0, 0, 12), (0, 3), lines, "prod")


def test_a_multi_line_cfg_test_attribute_above_the_span_counts():
    import rust_refs_kg as rr
    lines = ["#[cfg(", "    test", ")]", "mod helpers {", "}"]
    assert rr._span_is_test((3, 0, 4, 1), (3, 4), lines, "helpers")


def test_ask_escapes_its_inputs():
    import ask
    q = ask.q_divergences_in_file("src/a'b.rs")
    assert "{path: 'src/a\\'b.rs'}" in q
    assert "qualifiedName: 'x\\'y'" in ask.q_lucene_callers("x'y")


def test_ask_no_caller_joins_verdicts_and_filters():
    import ask
    rows = [{"item": "rucene::search::A", "label": "RustFn", "subsystem": "search",
             "class": "W?", "callerClasses": ["org.X"], "blocked": True},
            {"item": "rucene::search::B", "label": "RustFn", "subsystem": "search", "class": "P?"},
            {"item": "rucene::index::C", "label": "RustStruct", "subsystem": "index", "class": "D?"}]
    verdicts = {"rucene::search::A": ("W-blocked", "block5_search.md")}
    out = ask.join_no_caller(rows, verdicts, subsystem="search")
    assert [(r["item"], r["verdict"]) for r in out] == [
        ("rucene::search::A", "W-blocked"), ("rucene::search::B", None)]
    assert [r["item"] for r in ask.join_no_caller(rows, verdicts, unverified=True)] == [
        "rucene::search::B", "rucene::index::C"]


def test_a_span_starting_past_the_text_reads_no_attributes():
    import rust_refs_kg as rr
    # the walk starts at the last line, a code line, so it reads nothing
    assert rr._preceding_attributes(["#[cfg(test)]", "mod m {}"], 9) == []
    assert rr._preceding_attributes(["fn f() {}"], 5) == []
    # a start one past an attribute line reads it
    assert rr._preceding_attributes(["#[cfg(test)]"], 1) == ["#[cfg(test)]"]
