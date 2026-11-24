"""Regression tests for the KG extractor defects of rmp #505.

Each test reproduces one defect on a minimal input and fails on the code
before the fix:

* module-level `Port of ...` paragraphs after a licence header were never read
  (`port_evidence_kg.scan_file`);
* the triage surface read only the root crate's `src/`, so every member of a
  type a workspace crate ports was reported `absent` (`rust_surface.parse_tree`);
* `lucene_tests_kg.strip_noise` blanked real code (substitution order);
* `lucene_tests_kg.TEST_METHOD_RE` counted `test*` helpers JUnit never runs;
* nested exception classes were stored with kind `class`;
* the modifiers-only declaration head of `lucene_tests_kg.TYPE_RE` and
  `enrich_lucene_kg.scan_external_supertypes`;
* `port_modules_kg.crate_destination` did not snake-case package segments.
"""
import textwrap

import enrich_lucene_kg as en
import lucene_tests_kg as lt
import port_evidence_kg as pe
import port_modules_kg as pm
import rust_surface

ASF_HEADER = textwrap.dedent("""\
    /*
     * Licensed to the Apache Software Foundation (ASF) under one or more
     * contributor license agreements.
     */

    """)


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


# --- port_evidence_kg: module claims after a licence header -----------------

def test_a_module_claim_after_a_licence_header_is_read(tmp_path):
    write(tmp_path / "crates/x/src/null_info_stream.rs", ASF_HEADER + textwrap.dedent("""\
        //! Port of `org.apache.lucene.tests.util.NullInfoStream`.

        use std::sync::Arc;

        pub struct NullInfoStream;
        """))
    claims = pe.scan_file(tmp_path, "crates/x/src/null_info_stream.rs")
    assert [(c["scope"], c["lucene"], c["line"]) for c in claims] == [
        ("module", "org.apache.lucene.tests.util.NullInfoStream", 6)]


def test_plain_comments_and_empty_doc_lines_do_not_end_the_module_doc(tmp_path):
    write(tmp_path / "src/a.rs", textwrap.dedent("""\
        // SPDX-License-Identifier: Apache-2.0
        /* one-line block */
        //! An overview paragraph that names nothing.
        //!
        //! Port of `org.apache.lucene.util.CollectionUtil`.

        pub fn sort() {}
        """))
    claims = pe.scan_file(tmp_path, "src/a.rs")
    assert [c["lucene"] for c in claims if c["scope"] == "module"] == [
        "org.apache.lucene.util.CollectionUtil"]


def test_the_module_doc_still_ends_at_the_first_item(tmp_path):
    write(tmp_path / "src/b.rs", ASF_HEADER + textwrap.dedent("""\
        //! Helpers.

        use std::sync::Arc;
        //! Port of `org.apache.lucene.util.CollectionUtil`.
        """))
    assert [c for c in pe.scan_file(tmp_path, "src/b.rs") if c["scope"] == "module"] == []


# --- rust_surface: the triage reads the workspace crates --------------------

def test_the_triage_surface_reads_workspace_crates(tmp_path):
    write(tmp_path / "src/lib.rs", "pub fn core_fn() {}\n")
    write(tmp_path / "crates/rucene-x/Cargo.toml", '[package]\nname = "rucene-x"\n')
    write(tmp_path / "crates/rucene-x/src/query_parser_base.rs", textwrap.dedent("""\
        pub trait QueryParserBaseOps {
            /// Equivalent to `getDefaultOperator()`.
            fn get_default_operator(&self) -> i32 {
                0
            }
        }
        """))
    tree = rust_surface.parse_tree(tmp_path)
    assert set(tree) == {"src/lib.rs", "crates/rucene-x/src/query_parser_base.rs"}
    trait = tree["crates/rucene-x/src/query_parser_base.rs"]["traits"][0]
    assert [f["name"] for f in trait["fns"]] == ["get_default_operator"]


# --- lucene_tests_kg: the lexical pass --------------------------------------

def test_strip_noise_keeps_code_after_a_url_string_and_a_comment_opener_string():
    src = textwrap.dedent("""\
        @AwaitsFix(bugUrl = "https://issues.apache.org/jira/browse/LUCENE-5737")
        public void testCRTReopen() throws Exception {
          String s = "/*";
        }
        public void testAfterTheOpener() {}
        // String t = "*/";
        """)
    out = lt.strip_noise(src)
    assert len(out) == len(src)
    assert [i for i, c in enumerate(out) if c == "\n"] == \
        [i for i, c in enumerate(src) if c == "\n"]
    assert [m.group(1) for m in lt.TEST_METHOD_RE.finditer(out)] == [
        "testCRTReopen", "testAfterTheOpener"]


# --- lucene_tests_kg: which methods JUnit runs ------------------------------

def test_only_public_non_static_no_argument_test_methods_count():
    src = lt.strip_noise(textwrap.dedent("""\
        public class TestX extends LuceneTestCase {
          public void testRuns() throws Exception {}
          @Override
          public final void testFinalRuns() {}
          @Nightly public void testAnnotatedRuns() {}
          public void testAllFunctions(SpanQuery q, int[] expDocNrs) {}
          private void testPrivateHelper() {}
          protected void testProtectedHelper() {}
          void testPackagePrivate() {}
          public static void testStatic() {}
          public abstract void testAbstract();
        }
        """))
    assert [m.group(1) for m in lt.TEST_METHOD_RE.finditer(src)] == [
        "testRuns", "testFinalRuns", "testAnnotatedRuns"]


# --- declaration heads with an annotation on the same line ------------------

def test_a_top_level_test_type_annotated_on_its_line_is_declared():
    src = lt.strip_noise(
        '@SuppressForbidden(reason = "x") public abstract class BaseFooTestCase {}\n')
    m = lt.TYPE_RE.search(src)
    assert m is not None and m.group("name") == "BaseFooTestCase"
    assert m.group("kind") == "class"


def test_external_supertypes_see_a_declaration_annotated_on_its_line(tmp_path):
    write(tmp_path / "src/org/apache/lucene/a/Foo.java", textwrap.dedent("""\
        package org.apache.lucene.a;

        import java.io.IOException;

        @SuppressWarnings("serial") public final class Foo extends IOException {}
        """))
    with pm.lucene_roots(tmp_path, "src") as (_el, enrich):
        ext, _impl = enrich.scan_external_supertypes()
    assert ext == {"org.apache.lucene.a.Foo": ["java.io.IOException"]}


# --- nested exception kind ---------------------------------------------------

def test_nested_exceptions_are_kind_exception(tmp_path):
    write(tmp_path / "src/org/apache/lucene/a/Outer.java", textwrap.dedent("""\
        package org.apache.lucene.a;

        public class Outer {
          public static final class DocTermsIndexException extends RuntimeException {}
          static private final class LookaheadSuccess extends java.lang.Error { }
          static final class Holder {}
          interface Thing {}
        }
        """))
    with pm.lucene_roots(tmp_path, "src") as (_el, enrich):
        kinds = {n["name"]: n["kind"] for n in enrich.extract_inner_classes()}
    assert kinds == {"DocTermsIndexException": "exception",
                     "LookaheadSuccess": "exception",
                     "Holder": "class", "Thing": "interface"}
    assert en.e.is_exception_decl("Holder", None) is False


# --- port_modules_kg.crate_destination --------------------------------------

def test_crate_destination_snake_cases_package_segments():
    assert pm.crate_destination(
        "lucene/queryparser", "org.apache.lucene.queryparser.complexPhrase") \
        == "crates/rucene-queryparser/src/complex_phrase/"
    assert pm.crate_destination(
        "lucene/queryparser", "org.apache.lucene.queryparser.flexible.standard.nodes") \
        == "crates/rucene-queryparser/src/flexible/standard/nodes/"


# --- port_evidence_kg.load: a module claim in a file whose items claim nothing

class _LoadGraph:
    """The reads `port_evidence_kg.load` makes, answered from fixed sets."""

    def __init__(self, classes, types, fns):
        self.classes, self.types, self.fns = classes, types, fns

    def read(self, _roadmap, cypher):
        if "RETURN c.qualifiedName" in cypher and "DECLARES" not in cypher \
                and "PORTS" not in cypher:
            return [[c] for c in self.classes]
        if "DECLARES" in cypher:
            return []
        if "t:RustStruct OR" in cypher:
            return [list(t) for t in self.types]
        if "Component" in cypher:
            return []
        if "MATCH (f:RustFn)" in cypher:
            return [list(f) for f in self.fns]
        if "PORTS" in cypher:
            return []
        raise AssertionError(cypher)


def _load(monkeypatch, claims, types, fns=()):
    graph = _LoadGraph(["org.apache.lucene.tests.geo.GeoTestUtil",
                        "org.apache.lucene.tests.search.BaseExplanationTestCase"],
                       types, fns)
    monkeypatch.setattr(pe, "q", graph.read)
    monkeypatch.setattr(pe, "run_unwind", lambda *a, **k: None)
    monkeypatch.setattr(pe, "item_labels", lambda _r: {})
    monkeypatch.setattr(pe, "unwind_items", lambda *a, **k: {})
    return pe.load("test", claims, "c0ffee", "2026-11-15")


def test_a_module_claim_lands_on_the_component_when_the_files_types_claim_nothing(
        monkeypatch):
    geo = "crates/x/src/geo/geo_test_util.rs"
    rows = _load(monkeypatch,
                 [{"file": geo, "line": 18, "target": None, "scope": "module",
                   "lucene": "org.apache.lucene.tests.geo.GeoTestUtil", "note": None}],
                 types=[("SvgObject", geo)], fns=[("next_latitude", geo)])
    assert [(r["name"], r["file"], r["lucene"]) for r in rows] == [
        ("GeoTestUtil", geo, "org.apache.lucene.tests.geo.GeoTestUtil")]


def test_a_module_claim_is_still_dropped_when_an_item_of_the_file_claims(monkeypatch):
    f = "crates/x/src/search/base_explanation_test_case.rs"
    fqn = "org.apache.lucene.tests.search.BaseExplanationTestCase"
    rows = _load(monkeypatch,
                 [{"file": f, "line": 18, "target": None, "scope": "module",
                   "lucene": fqn, "note": None},
                  {"file": f, "line": 99, "target": "class", "targetKind": "fn",
                   "scope": "item", "lucene": fqn, "note": None}],
                 types=[("ExplanationState", f)], fns=[("class", f)])
    assert [(r["name"], r["file"]) for r in rows] == [("class", f)]


# --- rmp #603: the triage binds a Rust type to the file that ports it ---------

ITEM_RS = textwrap.dedent("""\
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct Item {
        parent: u16,
    }

    pub(crate) type Alias = Vec<u8>;

    pub(crate) struct Iter;

    impl Iterator for Iter {
        type Item = Vec<u16>;
        fn next(&mut self) -> Option<Self::Item> { None }
    }

    trait Source {
        type Item;
    }
    """)


def test_an_associated_type_does_not_replace_the_files_type():
    types = rust_surface.parse("x/ternary_tree.rs", ITEM_RS)["types"]
    assert types["Item"]["kind"] == "struct"
    assert types["Item"]["derives"] == ["Clone", "Copy", "Debug"]
    assert types["Alias"]["kind"] == "type"


def _two_same_named_types():
    import triage_members as tm
    long_rs = textwrap.dedent("""\
        struct FieldValuesSource {
            field: String,
        }
        impl LongValuesSource for FieldValuesSource {
            /// Equivalent to `FieldValuesSource.hashCode()`.
            fn source_hash(&self) -> i32 { 0 }
        }
        """)
    double_rs = textwrap.dedent("""\
        #[derive(PartialEq)]
        struct FieldValuesSource {
            field: String,
        }
        impl DoubleValuesSource for FieldValuesSource {
            fn double_hash(&self) -> i32 { 0 }
        }
        """)
    other_rs = "impl Shared for FieldValuesSource {\n    fn shared(&self) {}\n}\n"
    tree = {f: rust_surface.parse(f, t) for f, t in (
        ("src/search/long_values_source.rs", long_rs),
        ("src/search/double_values_source.rs", double_rs),
        ("src/index/other.rs", other_rs))}
    return tm.Surface(tree)


def test_impls_derives_and_fields_bind_to_the_declaring_file():
    S = _two_same_named_types()
    lf, df = "src/search/long_values_source.rs", "src/search/double_values_source.rs"
    assert set(S.item_fns("FieldValuesSource", lf)) == {"source_hash", "shared"}
    assert set(S.item_fns("FieldValuesSource", df)) == {"double_hash", "shared"}
    assert "DoubleValuesSource" not in S.traits_of("FieldValuesSource", lf)
    assert S.derived_traits("FieldValuesSource", lf) == set()
    assert S.derived_traits("FieldValuesSource", df) == {"PartialEq"}
    assert [f for f, _ in S.fields("FieldValuesSource", lf)] == [lf]
    # No file named: every site, as before.
    assert set(S.item_fns("FieldValuesSource")) >= {"source_hash", "double_hash"}


# --- extract_rucene_kg: inline module blocks named by a raw identifier ------
#
# icu4j's `lib.rs` declares `pub mod r#impl { pub mod x; ... }` (Java package
# `com.ibm.icu.impl`). The expanded-crate scan read `mod r#impl {` as an
# anonymous block, so every item under it lost the `impl` segment, missed its
# file in `mod_to_file` and was re-added as a macro-origin item of `lib.rs`
# (rmp #634: ~3,260 false nodes and host-file-name edges).

ICU_EXPANDED = textwrap.dedent("""\
    pub mod jdk {
        pub struct Jdk;
    }
    pub mod r#impl {
        pub mod breakiter {
            pub mod ml_break_engine {
                pub struct ModelIndex {
                    value: i32,
                }
                impl ModelIndex {
                    pub fn get_value(&self) -> i32 { self.value }
                }
            }
        }
        pub mod locale_display_names_impl {
            struct Cache {
                key: u8,
            }
        }
    }
    """)

ICU_MOD_TO_FILE = {
    "icu4j": "crates/icu4j/src/lib.rs",
    "icu4j::jdk": "crates/icu4j/src/jdk/mod.rs",
    "icu4j::impl::breakiter::ml_break_engine":
        "crates/icu4j/src/impl/breakiter/ml_break_engine.rs",
    "icu4j::impl::locale_display_names_impl":
        "crates/icu4j/src/impl/locale_display_names_impl.rs",
}


def test_a_raw_identifier_module_keeps_its_segment_in_the_expanded_scan():
    import extract_rucene_kg as ex
    types, fns = ex.parse_expanded(ICU_EXPANDED, "icu4j")
    assert sorted(types) == [
        ("icu4j::impl::breakiter::ml_break_engine", "ModelIndex", "struct"),
        ("icu4j::impl::locale_display_names_impl", "Cache", "struct"),
        ("icu4j::jdk", "Jdk", "struct"),
    ]
    assert [(m, n, o) for m, n, o, _s in fns] == [
        ("icu4j::impl::breakiter::ml_break_engine", "get_value", "ModelIndex")]


def test_items_under_a_raw_identifier_module_are_not_re_added_to_lib_rs(tmp_path):
    import extract_rucene_kg as ex
    known = {
        ("Jdk", "crates/icu4j/src/jdk/mod.rs"),
        ("ModelIndex", "crates/icu4j/src/impl/breakiter/ml_break_engine.rs"),
        ("Cache", "crates/icu4j/src/impl/locale_display_names_impl.rs"),
    }
    known_fns = {("get_value", "crates/icu4j/src/impl/breakiter/ml_break_engine.rs",
                  "ModelIndex")}
    assert ex.macro_generated_types(
        tmp_path, ICU_MOD_TO_FILE, set(known), ICU_EXPANDED, "icu4j") == []
    assert ex.macro_generated_fns(
        ICU_MOD_TO_FILE, set(known_fns), ICU_EXPANDED, "icu4j") == []


def test_the_literal_scan_names_a_raw_identifier_module_without_its_prefix():
    import extract_rucene_kg as ex
    lib_rs = textwrap.dedent("""\
        pub mod jdk;
        pub mod r#impl {
            pub mod icu_binary;
            pub mod breakiter {
                pub mod ml_break_engine;
            }
        }
        """)
    _types, _fns, _impls, mods, _uses = ex.parse_file("crates/icu4j/src/lib.rs", lib_rs)
    assert [(m["modSuffix"], m["name"]) for m in mods] == [
        ([], "impl"), (["impl"], "breakiter")]


def test_a_use_path_through_a_raw_identifier_module_resolves_to_the_real_file():
    import extract_rucene_kg as ex
    target = ex.resolve_use(
        "crate::r#impl::breakiter::ml_break_engine::ModelIndex",
        "icu4j::text::transliterator", ICU_MOD_TO_FILE, False, "icu4j")
    assert target == "crates/icu4j/src/impl/breakiter/ml_break_engine.rs"
    assert ex.RE_EXP_MOD.match("pub mod r#impl {").group(1) == "impl"
