"""Task #323: the port modules beyond lucene/core in the KG -- sprint 9's
lucene/test-framework, lucene/codecs and randomizedtesting 2.8.1, sprint 12's
lucene/analysis/common (`test_skeletons.py`), and sprint 13's lucene/queries,
lucene/queryparser and lucene/sandbox scoped to FuzzyLikeThisQuery.java -- and the
workspace crates that port them. Off the store: every graph call is replaced by a
recorder."""
import json
import sys
from pathlib import Path

import ask
import audit_kg as audit
import lucene_structure_kg as ls
import divergences_kg as dv
import extract_lucene_kg as el
import enrich_lucene_kg as en
import extract_rucene_kg as ex
import port_coverage_kg as pc
import port_evidence_kg as pe
import port_modules_kg as pm

MODEL = Path(pm.__file__).resolve().parents[2] / "knowledge-model.md"


def write(root: Path, rel: str, text: str):
    p = root / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text, encoding="utf-8")
    return p


def write_query_descriptors(lucene: Path):
    """The three sprint-13 descriptors, as Lucene 10.5.0 declares their `requires`."""
    write(lucene, "lucene/queries/src/java/module-info.java",
          "module org.apache.lucene.queries {\n requires org.apache.lucene.core;\n}")
    write(lucene, "lucene/queryparser/src/java/module-info.java",
          "module org.apache.lucene.queryparser {\n requires java.xml;\n"
          " requires org.apache.lucene.core;\n requires org.apache.lucene.queries;\n"
          " requires org.apache.lucene.sandbox;\n}")
    write(lucene, "lucene/sandbox/src/java/module-info.java",
          "module org.apache.lucene.sandbox {\n requires org.apache.lucene.core;\n"
          " requires org.apache.lucene.queries;\n requires org.apache.lucene.facet;\n}")


# --- the module registry ----------------------------------------------------

def test_the_registry_names_the_eight_modules_and_their_crates():
    assert [m["name"] for m in pm.PORT_MODULES] == [
        "lucene/core", "lucene/codecs", "lucene/test-framework",
        "randomizedtesting/randomized-runner", "lucene/analysis/common",
        "lucene/queries", "lucene/queryparser", "lucene/sandbox"]
    assert pm.ADDED_MODULES == ("lucene/codecs", "lucene/test-framework",
                                "randomizedtesting/randomized-runner", "lucene/analysis/common",
                                "lucene/queries", "lucene/queryparser", "lucene/sandbox")
    assert {m["cratePath"] for m in pm.PORT_MODULES} == {
        ".", "crates/rucene-codecs", "crates/rucene-test-framework", "crates/randomizedtesting",
        "crates/rucene-analysis-common", "crates/rucene-queries", "crates/rucene-queryparser",
        "crates/rucene-sandbox"}
    # only lucene/sandbox is narrowed to listed files
    assert [m["name"] for m in pm.PORT_MODULES if m.get("files")] == ["lucene/sandbox"]


def test_module_of_file_keys_on_the_declaring_tree():
    assert pm.module_of_file("lucene/core/src/java/org/apache/lucene/index/IndexWriter.java") \
        == "lucene/core"
    assert pm.module_of_file("lucene/core/src/java21/org/apache/lucene/store/X.java") \
        == "lucene/core"
    assert pm.module_of_file("lucene/codecs/src/java/org/apache/lucene/codecs/bloom/F.java") \
        == "lucene/codecs"
    assert pm.module_of_file("lucene/test-framework/src/java/org/apache/lucene/tests/util/"
                             "LuceneTestCase.java") == "lucene/test-framework"
    assert pm.module_of_file("randomized-runner/src/main/java/com/carrotsearch/"
                             "randomizedtesting/RandomizedTest.java") \
        == "randomizedtesting/randomized-runner"
    assert pm.module_of_file("lucene/core/src/test/org/apache/lucene/TestX.java") is None
    assert pm.module_of_file("tests/fixtures/X.java") is None
    assert pm.module_of_file(None) is None


def test_crate_destination_mirrors_the_package_below_the_module_root():
    assert pm.crate_destination("lucene/codecs", "org.apache.lucene.codecs.simpletext") \
        == "crates/rucene-codecs/src/simpletext/"
    assert pm.crate_destination("lucene/test-framework", "org.apache.lucene.tests.util") \
        == "crates/rucene-test-framework/src/util/"
    assert pm.crate_destination("randomizedtesting/randomized-runner",
                                "com.carrotsearch.randomizedtesting") \
        == "crates/randomizedtesting/src/"
    assert pm.crate_destination("randomizedtesting/randomized-runner",
                                "com.carrotsearch.randomizedtesting.generators") \
        == "crates/randomizedtesting/src/generators/"


def test_descriptor_requires_reads_through_modifiers_and_ignores_comments():
    text = """/* requires fake.module; */
    module org.apache.lucene.test_framework {
      requires org.apache.lucene.core;
      // requires commented.out;
      requires transitive randomizedtesting.runner;
      requires static org.apache.lucene.codecs;
    }"""
    assert pm.descriptor_requires(text) == [
        "org.apache.lucene.core", "randomizedtesting.runner", "org.apache.lucene.codecs"]


def test_module_dependencies_map_jpms_names_to_port_modules(tmp_path):
    lucene, rt = tmp_path / "lucene", tmp_path / "rt"
    write(lucene, "lucene/core/src/java/module-info.java",
          "module org.apache.lucene.core { requires jdk.unsupported; }")
    write(lucene, "lucene/codecs/src/java/module-info.java",
          "module org.apache.lucene.codecs { requires org.apache.lucene.core; }")
    write(lucene, "lucene/test-framework/src/java/module-info.java",
          "module org.apache.lucene.test_framework {\n requires org.apache.lucene.core;\n"
          " requires org.apache.lucene.codecs;\n requires transitive junit;\n"
          " requires transitive randomizedtesting.runner;\n}")
    write(lucene, "lucene/analysis/common/src/java/module-info.java",
          "module org.apache.lucene.analysis.common {\n requires java.xml;\n"
          " requires org.apache.lucene.core;\n}")
    write_query_descriptors(lucene)
    deps = pm.module_dependencies(pm.repo_roots(lucene, rt))
    assert [(a, b) for a, b, _ in deps] == [
        ("lucene/codecs", "lucene/core"),
        ("lucene/test-framework", "lucene/core"),
        ("lucene/test-framework", "lucene/codecs"),
        ("lucene/test-framework", "randomizedtesting/randomized-runner"),
        ("lucene/analysis/common", "lucene/core"),
        ("lucene/queries", "lucene/core"),
        ("lucene/queryparser", "lucene/core"),
        ("lucene/queryparser", "lucene/queries"),
        ("lucene/queryparser", "lucene/sandbox"),
        ("lucene/sandbox", "lucene/core"),
        ("lucene/sandbox", "lucene/queries")]


def test_queries_narrow_to_one_module_and_the_model_holds_them_verbatim():
    q = pm.coverage_query("lucene/codecs")
    assert "c.module = 'lucene/codecs'" in q and q.count("RETURN ") == 1
    assert "WHERE c.module = 'lucene/codecs' RETURN" in pm.early_query("lucene/codecs")
    model = MODEL.read_text(encoding="utf-8")
    assert pm.COVERAGE_QUERY in model
    assert pm.EARLY_QUERY in model


# --- the Java survey, reused over another module's tree ---------------------

RT_FILE = """package com.carrotsearch.randomizedtesting;

import com.carrotsearch.randomizedtesting.generators.RandomNumbers;
import org.junit.Assert;

public class RandomizedTest extends Assert {
  public static int randomInt() { return 0; }
  public static class Holder { int x; }
}
"""
RT_GEN = """package com.carrotsearch.randomizedtesting.generators;

public final class RandomNumbers {
  public static int randomIntBetween(int a, int b) { return a; }
}
"""


def test_survey_reads_a_file_without_a_licence_header_and_restores_the_extractors(tmp_path):
    rt = tmp_path / "rt"
    base = "randomized-runner/src/main/java/com/carrotsearch/randomizedtesting"
    write(rt, f"{base}/RandomizedTest.java", RT_FILE)
    write(rt, f"{base}/generators/RandomNumbers.java", RT_GEN)
    saved = (el.ROOT, el.LUCENE_ROOT, el.CORE_PREFIX, en.BASE_ROOT, en.CORE_ROOT_JAVA,
             en.CORE_ROOT_JAVA21)
    data = pm.survey_module("randomizedtesting/randomized-runner",
                            pm.repo_roots(tmp_path / "lucene", rt))
    assert (el.ROOT, el.LUCENE_ROOT, el.CORE_PREFIX, en.BASE_ROOT, en.CORE_ROOT_JAVA,
            en.CORE_ROOT_JAVA21) == saved
    assert sorted(t["qualified"] for t in data["types"]) == [
        "com.carrotsearch.randomizedtesting.RandomizedTest",
        "com.carrotsearch.randomizedtesting.generators.RandomNumbers"]
    assert {f["path"] for f in data["files"]} == {
        f"{base}/RandomizedTest.java", f"{base}/generators/RandomNumbers.java"}
    assert data["packages"] == ["com.carrotsearch.randomizedtesting",
                                "com.carrotsearch.randomizedtesting.generators"]
    assert [n["qualifiedName"] for n in data["nested"]] == [
        "com.carrotsearch.randomizedtesting.RandomizedTest$Holder"]
    names = {m["qualifiedName"].split("#")[0] + "#" + m["name"]
             for v in data["members"].values() for m in v}
    assert "com.carrotsearch.randomizedtesting.RandomizedTest#randomInt" in names
    assert "com.carrotsearch.randomizedtesting.generators.RandomNumbers#randomIntBetween" \
        in names


def test_scan_lucene_covers_every_module_and_crosses_module_boundaries(tmp_path):
    lucene, rt = tmp_path / "lucene", tmp_path / "rt"
    write(lucene, "lucene/core/src/java/org/apache/lucene/util/Bits.java",
          "/* licence */\npackage org.apache.lucene.util;\npublic interface Bits {}\n")
    write(lucene, "lucene/test-framework/src/java/org/apache/lucene/tests/util/TestUtil.java",
          "/* licence */\npackage org.apache.lucene.tests.util;\n"
          "import org.apache.lucene.util.Bits;\n"
          "import com.carrotsearch.randomizedtesting.generators.RandomNumbers;\n"
          "public final class TestUtil { Bits b; }\n")
    write(lucene, "lucene/codecs/src/java/org/apache/lucene/codecs/bloom/Fuzzy.java",
          "/* licence */\npackage org.apache.lucene.codecs.bloom;\npublic class Fuzzy {}\n")
    base = "randomized-runner/src/main/java/com/carrotsearch/randomizedtesting"
    write(rt, f"{base}/generators/RandomNumbers.java", RT_GEN)
    top, deps = pc.scan_lucene(str(lucene), str(rt))
    assert top == {
        "org.apache.lucene.util.Bits": "lucene/core/src/java/org/apache/lucene/util/Bits.java",
        "org.apache.lucene.tests.util.TestUtil":
            "lucene/test-framework/src/java/org/apache/lucene/tests/util/TestUtil.java",
        "org.apache.lucene.codecs.bloom.Fuzzy":
            "lucene/codecs/src/java/org/apache/lucene/codecs/bloom/Fuzzy.java",
        "com.carrotsearch.randomizedtesting.generators.RandomNumbers":
            f"{base}/generators/RandomNumbers.java",
    }
    assert sorted(deps) == [
        ("org.apache.lucene.tests.util.TestUtil",
         "com.carrotsearch.randomizedtesting.generators.RandomNumbers"),
        ("org.apache.lucene.tests.util.TestUtil", "org.apache.lucene.util.Bits")]


# --- port state of the sprint-9 modules -------------------------------------

class Graph:
    """Answers `read` by the first registered substring a query contains and
    records every `run_unwind` it is sent."""

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


def test_a_member_only_claim_leaves_a_sprint9_type_missing_but_not_a_core_one(monkeypatch):
    core = "org.apache.lucene.index.TwoPhaseCommitTool"
    tf = "org.apache.lucene.tests.util.TestUtil"
    tf_full = "org.apache.lucene.tests.util.LuceneTestCase"
    nested = "org.apache.lucene.tests.util.LuceneTestCase$Nightly"
    top = {core: "lucene/core/src/java/org/apache/lucene/index/TwoPhaseCommitTool.java",
           tf: "lucene/test-framework/src/java/org/apache/lucene/tests/util/TestUtil.java",
           tf_full: "lucene/test-framework/src/java/org/apache/lucene/tests/util/"
                    "LuceneTestCase.java"}
    g = Graph([
        ("RETURN c.qualifiedName AS qn, c.file AS file", [
            {"qn": q, "file": f, "pkg": q.rsplit(".", 1)[0]} for q, f in top.items()] + [
            {"qn": nested, "file": top[tf_full], "pkg": "org.apache.lucene.tests.util"}]),
        ("portStateNote IS NOT NULL", []),
        ("collect(coalesce(e.claimLevel, 'type'))", [
            {"qn": core, "module": "lucene/core", "levels": ["member"]},
            {"qn": tf, "module": "lucene/test-framework", "levels": ["member"]},
            {"qn": tf_full, "module": "lucene/test-framework", "levels": ["member", "type"]}]),
        ("c.portScope = 'nested' AND coalesce", []),
    ])
    monkeypatch.setattr(pc, "read", g.read)
    monkeypatch.setattr(pc, "run_unwind", g.run_unwind)
    pc.phase_scope("r", top, "c" * 40, "2026-11-03", 200)

    ins = {r["qn"]: r for r in g.rows("portScope = in")}
    assert ins[core]["module"] == "lucene/core" and ins[core]["rule"] == "lucene-core-top-level"
    assert ins[tf]["module"] == "lucene/test-framework"
    assert ins[tf]["rule"] == "sprint-9-module-top-level"
    assert {r["qn"] for r in g.rows("portState = ported")} == {core, tf_full}
    assert {r["qn"] for r in g.rows("portState = unported")} == {tf}
    assert g.rows("portScope = nested") == [{"qn": nested, "module": "lucene/test-framework"}]
    (n,) = g.rows("added-module nested portState")
    assert (n["qn"], n["state"], n["evidence"], n["dest"]) == (
        nested, "unported", "no-port-claim", "crates/rucene-test-framework/src/util/")


def test_candidates_are_looked_for_only_in_the_crate_of_the_module(monkeypatch, tmp_path):
    survey = tmp_path / "survey.json"
    survey.write_text(json.dumps({"types": [
        {"name": "TestUtil", "kind": "struct", "scope": "crate", "file": "src/util/x.rs"},
        {"name": "Fuzzy", "kind": "struct", "scope": "crate",
         "file": "crates/rucene-codecs/src/bloom/fuzzy.rs"},
        {"name": "Bits", "kind": "trait", "scope": "crate",
         "file": "crates/rucene-codecs/src/bits.rs"}]}))
    g = Graph([("NOT c.portState IN ['ported', 'skeleton']", [
        {"qn": "org.apache.lucene.tests.util.TestUtil", "name": "TestUtil",
         "module": "lucene/test-framework"},
        {"qn": "org.apache.lucene.codecs.bloom.Fuzzy", "name": "Fuzzy",
         "module": "lucene/codecs"},
        {"qn": "org.apache.lucene.util.Bits", "name": "Bits", "module": "lucene/core"}])])
    monkeypatch.setattr(pc, "read", g.read)
    monkeypatch.setattr(pc, "run_unwind", g.run_unwind)
    monkeypatch.setattr(pc, "rmp", lambda *a, **k: "{}")
    pc.phase_candidates("r", str(survey), "c" * 40, "2026-11-03", 200)
    assert [r["qn"] for r in g.rows("portState = candidate")] == [
        "org.apache.lucene.codecs.bloom.Fuzzy"]


# --- the workspace crates ----------------------------------------------------

def make_workspace(root: Path):
    write(root, "Cargo.toml", '[package]\nname = "rucene"\n')
    write(root, "src/lib.rs", "pub mod util;\n")
    write(root, "src/util.rs", "pub struct Bits;\n")
    write(root, "crates/rucene-codecs/Cargo.toml",
          '[package]\nname = "rucene-codecs"\nversion = "0.1.0"\n\n[dependencies]\n'
          'name = "not-the-package"\n')
    write(root, "crates/rucene-codecs/src/lib.rs", "pub mod bloom;\n")
    write(root, "crates/rucene-codecs/src/bloom.rs",
          "use crate::lib_helper;\nuse rucene::util::Bits;\n\n"
          "/// Equivalent to `org.apache.lucene.codecs.bloom.FuzzySet`.\n"
          "pub struct FuzzySet;\n\n"
          "/// **Divergence from Lucene 10.5.0.** See `FuzzySet.java:10`.\n"
          "pub fn hash() {}\n")
    write(root, "crates/randomizedtesting/Cargo.toml",
          '[package]\nname = "randomizedtesting"\n')
    write(root, "crates/randomizedtesting/src/lib.rs",
          "/// Equivalent to `com.carrotsearch.randomizedtesting.RandomizedTest`.\n"
          "pub struct RandomizedTest;\n")
    write(root, "crates/randomizedtesting/tests/seeds.rs", "#[test]\nfn t() {}\n")


def test_workspace_members_are_read_from_their_own_cargo_toml(tmp_path):
    make_workspace(tmp_path)
    assert ex.load_workspace(tmp_path) == {
        "crates/randomizedtesting": ("randomizedtesting", "randomizedtesting"),
        "crates/rucene-codecs": ("rucene-codecs", "rucene_codecs")}
    assert ex.module_path_for("crates/rucene-codecs/src/bloom.rs") == "rucene_codecs::bloom"
    assert ex.module_path_for("crates/rucene-codecs/src/lib.rs") == "rucene_codecs"
    assert ex.module_path_for("src/util.rs") == "rucene::util"
    assert ex.module_path_for("crates/randomizedtesting/tests/seeds.rs") is None
    mods = {"rucene::util": "src/util.rs", "rucene_codecs::bloom": "crates/rucene-codecs/src/bloom.rs",
            "rucene_codecs": "crates/rucene-codecs/src/lib.rs"}
    assert ex.resolve_use("crate::bloom::X", "rucene_codecs", mods, False, "rucene_codecs") \
        == "crates/rucene-codecs/src/bloom.rs"
    assert ex.resolve_use("rucene::util::Bits", "rucene_codecs::bloom", mods, False,
                          "rucene_codecs") == "src/util.rs"
    assert ex.resolve_use("crate::util::Bits", "rucene::x", mods, False) == "src/util.rs"
    ex.WORKSPACE.clear()


def test_the_survey_covers_the_member_crates(tmp_path, monkeypatch):
    make_workspace(tmp_path)
    out = tmp_path / "survey.json"
    monkeypatch.setattr(sys, "argv", ["x", "--source-root", str(tmp_path), "--commit", "c",
                                      "--date", "d", "--output", str(out)])
    ex.main()
    ex.WORKSPACE.clear()
    s = json.loads(out.read_text())
    files = {f["path"]: f for f in s["files"]}
    assert files["crates/rucene-codecs/src/bloom.rs"]["crate"] == "rucene_codecs"
    assert files["crates/rucene-codecs/src/bloom.rs"]["package"] == "rucene-codecs"
    assert files["crates/rucene-codecs/src/bloom.rs"]["modulePath"] == "rucene_codecs::bloom"
    assert files["crates/randomizedtesting/tests/seeds.rs"]["kind"] == "test"
    assert files["src/util.rs"]["package"] == "rucene"
    qns = {t["qualifiedName"] for t in s["types"]}
    assert {"rucene_codecs::bloom::FuzzySet", "randomizedtesting::RandomizedTest",
            "rucene::util::Bits"} <= qns
    assert {"from": "crates/rucene-codecs/src/bloom.rs", "to": "src/util.rs"} in s["deps"]


def test_port_claims_are_read_from_the_member_crates(tmp_path):
    make_workspace(tmp_path)
    claims = {(c["file"], c["lucene"]) for c in pe.extract(tmp_path)}
    ex.WORKSPACE.clear()
    assert ("crates/rucene-codecs/src/bloom.rs", "org.apache.lucene.codecs.bloom.FuzzySet") \
        in claims
    assert ("crates/randomizedtesting/src/lib.rs",
            "com.carrotsearch.randomizedtesting.RandomizedTest") in claims
    fqn, _ = pe.claim_in("Equivalent to `com.carrotsearch.randomizedtesting.RandomizedTest`.")
    assert fqn == "com.carrotsearch.randomizedtesting.RandomizedTest"
    assert pe.RE_TABLE_HEADER.match("//! | Rucene | randomizedtesting 2.8.1 |".lstrip("/!"))


def test_divergences_are_scanned_in_the_member_crates(tmp_path):
    make_workspace(tmp_path)
    ids = [d["id"] for d in dv.scan_tree(tmp_path)]
    assert ids == ["crates/rucene-codecs/src/bloom.rs:7"]


def test_ask_lists_one_module_with_its_status():
    q = ask.q_coverage_list("lucene/codecs")
    assert "c.module = 'lucene/codecs'" in q and "ELSE 'unported'" in q


def test_test_support_modules_are_named_by_location(monkeypatch):
    """Two `tests/<dir>/mod.rs` support modules never share a base path.

    Regression: both were named `mod`, so `TempDir` of
    `crates/rucene-codecs/tests/oracle/mod.rs` and of
    `crates/rucene-test-framework/tests/oracle_support/mod.rs` shared the
    `qualifiedName` `mod::TempDir` and the audit counted one type missing.
    """
    monkeypatch.setattr(ex, "WORKSPACE", {
        "crates/rucene-codecs": ("rucene-codecs", "rucene_codecs"),
        "crates/rucene-test-framework": ("rucene-test-framework", "rucene_test_framework"),
    })
    codecs = ex.member_of("crates/rucene-codecs/tests/oracle/mod.rs")
    tf = ex.member_of("crates/rucene-test-framework/tests/oracle_support/mod.rs")
    a = ex.test_file_path("crates/rucene-codecs/tests/oracle/mod.rs", codecs, {})
    b = ex.test_file_path("crates/rucene-test-framework/tests/oracle_support/mod.rs", tf, {})
    assert a == "rucene_codecs::tests::oracle"
    assert b == "rucene_test_framework::tests::oracle_support"
    # test targets keep the crate name Cargo gives them
    assert ex.test_file_path("crates/rucene-codecs/tests/memory_oracle.rs", codecs, {}) == "memory_oracle"
    assert ex.test_file_path("tests/self/main.rs", None, {}) == "self"
    assert ex.test_file_path("tests/portability/codecs.rs", None,
                             {"tests/portability/codecs.rs": "portability_codecs"}) == "portability_codecs"
    assert ex.test_file_path("tests/common/util.rs", None, {}) == f"{ex.CRATE}::tests::common::util"


# --- sprint 13: lucene/queries, lucene/queryparser, sandbox FuzzyLikeThisQuery --

SB = "lucene/sandbox"
SB_SRC = "lucene/sandbox/src/java/org/apache/lucene/sandbox"
FLT_FILE = f"{SB_SRC}/queries/FuzzyLikeThisQuery.java"
FLT = "org.apache.lucene.sandbox.queries.FuzzyLikeThisQuery"
FLT_JAVA = """/* licence */
package org.apache.lucene.sandbox.queries;

import org.apache.lucene.search.Query;

public class FuzzyLikeThisQuery extends Query {
  public void addTerms(String queryString, String fieldName) {}
  static class FieldVals { String fieldName; }
  private static class ScoreTerm { float score; }
}
"""
# Another sandbox type, outside the user's scope: it must never enter the graph.
OTHER_FILE = f"{SB_SRC}/search/CombinedFieldQuery.java"
OTHER = "org.apache.lucene.sandbox.search.CombinedFieldQuery"
OTHER_JAVA = """/* licence */
package org.apache.lucene.sandbox.search;

public final class CombinedFieldQuery {
  public int fieldCount() { return 0; }
  public static class Builder { }
}
"""


def write_sandbox(lucene: Path):
    write(lucene, FLT_FILE, FLT_JAVA)
    write(lucene, OTHER_FILE, OTHER_JAVA)
    write(lucene, f"{SB_SRC}/queries/package-info.java",
          "/* licence */\n/** Queries. */\npackage org.apache.lucene.sandbox.queries;\n")


def test_sprint13_modules_are_port_modules_with_their_crates_and_rules():
    for name, crate, jpms in (("lucene/queries", "rucene-queries", "org.apache.lucene.queries"),
                              ("lucene/queryparser", "rucene-queryparser",
                               "org.apache.lucene.queryparser"),
                              (SB, "rucene-sandbox", "org.apache.lucene.sandbox")):
        m = pm.BY_NAME[name]
        assert (m["crate"], m["cratePath"], m["jpms"]) == (crate, f"crates/{crate}", jpms)
        assert m["descriptor"] == f"{name}/src/java/module-info.java"
        assert m["sources"] == (f"{name}/src/java",)
        assert m["skeletons"] == "report"
        assert name in pm.ADDED_MODULES and name in pm.SKELETON_MODULES
    for name in ("lucene/queries", "lucene/queryparser"):
        assert (pm.BY_NAME[name]["scopeRule"], pm.BY_NAME[name]["classRule"]) == (
            "sprint-13-ports-the-module", "sprint-13-module-top-level")
        assert "files" not in pm.BY_NAME[name]
    assert (pm.BY_NAME[SB]["scopeRule"], pm.BY_NAME[SB]["classRule"]) == (
        "sprint-13-ports-listed-files", "sprint-13-listed-file-top-level")
    assert pm.BY_NAME[SB]["files"] == (FLT_FILE,)
    d = pm.DECISIONS[-1]
    assert (d["name"], d["on"], d["modules"], d["supersedes"]) == (
        "Sprint 13 ports lucene/queries and lucene/queryparser, with sandbox "
        "FuzzyLikeThisQuery", "2026-11-14",
        ("lucene/queries", "lucene/queryparser", SB), None)
    summary, rationale, alternatives = pm.decision_text(d)
    assert "FuzzyLikeThisQuery" in summary and "sprint-13-listed-file-top-level" in summary
    assert "FuzzyLikeThisQueryBuilder" in rationale and "rejected" in alternatives


def test_sprint13_crate_destinations_mirror_the_package_below_the_module_root():
    assert pm.crate_destination("lucene/queries",
                                "org.apache.lucene.queries.function.valuesource") \
        == "crates/rucene-queries/src/function/valuesource/"
    assert pm.crate_destination("lucene/queries", "org.apache.lucene.queries") \
        == "crates/rucene-queries/src/"
    assert pm.crate_destination("lucene/queryparser",
                                "org.apache.lucene.queryparser.flexible.core.nodes") \
        == "crates/rucene-queryparser/src/flexible/core/nodes/"
    assert pm.crate_destination(SB, "org.apache.lucene.sandbox.queries") \
        == "crates/rucene-sandbox/src/queries/"


def test_the_sandbox_scope_is_the_listed_file_and_the_descriptor():
    assert pm.file_in_scope(SB, FLT_FILE)
    assert pm.file_in_scope(SB, "lucene/sandbox/src/java/module-info.java")
    assert not pm.file_in_scope(SB, OTHER_FILE)
    assert not pm.file_in_scope(SB, f"{SB_SRC}/queries/package-info.java")
    assert not pm.file_in_scope(SB, "lucene/sandbox/src/test/org/apache/lucene/sandbox/"
                                    "queries/TestFuzzyLikeThisQuery.java")
    assert not pm.file_in_scope(SB, None)
    assert pm.module_of_file(FLT_FILE) == SB
    assert pm.module_of_file(OTHER_FILE) is None
    # the modules ported whole keep every file of their trees
    assert pm.module_of_file("lucene/queries/src/java/org/apache/lucene/queries/function/"
                             "valuesource/ConstValueSource.java") == "lucene/queries"
    assert pm.module_of_file("lucene/queryparser/src/java/org/apache/lucene/queryparser/"
                             "classic/QueryParser.java") == "lucene/queryparser"


def test_the_survey_of_the_sandbox_keeps_only_the_listed_file(tmp_path):
    lucene = tmp_path / "lucene"
    write_sandbox(lucene)
    data = pm.survey_module(SB, pm.repo_roots(lucene, tmp_path / "rt"))
    assert [f["path"] for f in data["files"]] == [FLT_FILE]
    assert [t["qualified"] for t in data["types"]] == [FLT]
    assert sorted(n["qualifiedName"] for n in data["nested"]) == [
        f"{FLT}$FieldVals", f"{FLT}$ScoreTerm"]
    assert data["packages"] == ["org.apache.lucene.sandbox.queries"]
    owners = {x["parentQualifiedName"] for v in data["members"].values() for x in v}
    assert owners and owners <= {FLT, f"{FLT}$FieldVals", f"{FLT}$ScoreTerm"}
    names = {x["name"] for v in data["members"].values() for x in v}
    assert "addTerms" in names and "fieldCount" not in names


def test_the_structure_loaders_take_the_survey_instead_of_re_extracting(monkeypatch):
    sent = []
    monkeypatch.setattr(ls, "unwind", lambda _r, _m, rows, body, label, batch=None:
                        sent.append((label, rows)))
    monkeypatch.setattr(ls.en, "extract_inner_classes", lambda: 1 / 0)
    monkeypatch.setattr(ls.en, "extract_members", lambda: 1 / 0)
    inner = {"qualifiedName": f"{FLT}$ScoreTerm", "name": "ScoreTerm", "kind": "class",
             "file": FLT_FILE, "package": "org.apache.lucene.sandbox.queries",
             "parentQualifiedName": FLT}
    ls.load_nested("r", "c" * 40, "2026-11-14", prune=False, inners=[inner])
    method = {"qualifiedName": f"{FLT}#addTerms(String,String)", "name": "addTerms",
              "parentQualifiedName": FLT}
    ls.load_members("r", "c" * 40, "2026-11-14", members={
        "methods": [method], "constructors": [], "fields": [], "enumConstants": []})
    nested_rows = next(r for lab, r in sent if lab == "nested Class nodes")
    assert [r["qn"] for r in nested_rows] == [f"{FLT}$ScoreTerm"]
    method_rows = next(r for lab, r in sent if lab == "Method nodes")
    assert [(r["qn"], r["kind"], r["parent"]) for r in method_rows] == [
        (f"{FLT}#addTerms(String,String)", "method", FLT)]


def test_scan_lucene_reads_the_listed_sandbox_file_only(tmp_path):
    lucene = tmp_path / "lucene"
    write_sandbox(lucene)
    write(lucene, "lucene/queryparser/src/java/org/apache/lucene/queryparser/xml/builders/"
                  "FuzzyLikeThisQueryBuilder.java",
          "/* licence */\npackage org.apache.lucene.queryparser.xml.builders;\n"
          "import org.apache.lucene.sandbox.queries.FuzzyLikeThisQuery;\n"
          "import org.apache.lucene.sandbox.search.CombinedFieldQuery;\n"
          "public class FuzzyLikeThisQueryBuilder { FuzzyLikeThisQuery q; }\n")
    top, deps = pc.scan_lucene(str(lucene), str(tmp_path / "rt"))
    builder = "org.apache.lucene.queryparser.xml.builders.FuzzyLikeThisQueryBuilder"
    assert top == {FLT: FLT_FILE, builder: "lucene/queryparser/src/java/org/apache/lucene/"
                   "queryparser/xml/builders/FuzzyLikeThisQueryBuilder.java"}
    assert deps == [(builder, FLT)]


def test_the_audit_census_honours_the_allow_list(tmp_path):
    lucene = tmp_path / "lucene"
    write_sandbox(lucene)
    write(lucene, "lucene/sandbox/src/java/module-info.java",
          "module org.apache.lucene.sandbox { requires org.apache.lucene.core; }")
    files, top, nested = audit.java_census(lucene, ("lucene/sandbox/src/java",),
                                           lambda rel: pm.file_in_scope(SB, rel))
    assert sorted(files) == sorted([FLT_FILE, "lucene/sandbox/src/java/module-info.java"])
    assert top == [(FLT, FLT_FILE)]
    assert sorted(nested) == [("FieldVals", FLT_FILE), ("ScoreTerm", FLT_FILE)]
    # without the predicate the census reads the whole tree
    all_files, all_top, _ = audit.java_census(lucene, ("lucene/sandbox/src/java",))
    assert OTHER_FILE in all_files and (OTHER, OTHER_FILE) in all_top


def test_an_unlisted_sandbox_type_never_enters_the_scope(monkeypatch):
    """Even a stray sandbox `Class` node is scoped `out`, never `in` or `nested`."""
    g = Graph([
        ("RETURN c.qualifiedName AS qn, c.file AS file", [
            {"qn": FLT, "file": FLT_FILE, "pkg": "org.apache.lucene.sandbox.queries"},
            {"qn": f"{FLT}$ScoreTerm", "file": FLT_FILE,
             "pkg": "org.apache.lucene.sandbox.queries"},
            {"qn": OTHER, "file": OTHER_FILE, "pkg": "org.apache.lucene.sandbox.search"},
            {"qn": f"{OTHER}$Builder", "file": OTHER_FILE,
             "pkg": "org.apache.lucene.sandbox.search"}]),
        ("portStateNote IS NOT NULL", []),
    ])
    monkeypatch.setattr(pc, "read", g.read)
    monkeypatch.setattr(pc, "run_unwind", g.run_unwind)
    pc.phase_scope("r", {FLT: FLT_FILE}, "c" * 40, "2026-11-14", 200)
    assert g.rows("portScope = in") == [{"qn": FLT, "file": FLT_FILE, "module": SB,
                                          "rule": "sprint-13-listed-file-top-level"}]
    assert g.rows("portScope = nested") == [{"qn": f"{FLT}$ScoreTerm", "module": SB}]
    assert sorted(r["qn"] for r in g.rows("portScope = out")) == [OTHER, f"{OTHER}$Builder"]
    assert [r["qn"] for r in g.rows("portState = unported")] == [FLT]
    (n,) = g.rows("added-module nested portState")
    assert (n["qn"], n["state"], n["dest"]) == (
        f"{FLT}$ScoreTerm", "unported", "crates/rucene-sandbox/src/queries/")


def test_phase_modules_records_the_sandbox_scope_files_and_the_decision(monkeypatch, tmp_path):
    lucene, rt = tmp_path / "lucene", tmp_path / "rt"
    for m in pm.PORT_MODULES:
        base = lucene if m["repo"] == "lucene" else rt
        write(base, f"{m['sources'][0]}/X.java", "package x;\nclass X {}\n")
        if m["descriptor"]:
            write(base, m["descriptor"], f"module {m['jpms']} {{ requires org.apache.lucene.core; }}")
    write_query_descriptors(lucene)
    writes, statements = [], []
    monkeypatch.setattr(pm, "unwind", lambda _r, _m, rows, body, label, *a, **k:
                        writes.append((label, rows, body)))
    monkeypatch.setattr(pm, "rmp", lambda _m, _r, q, *a, **k: statements.append(q) or "{}")
    monkeypatch.setattr(pm, "read", lambda _r, _q: [[0]])
    pm.phase_modules("r", pm.repo_roots(lucene, rt), "c" * 40, "2026-11-14")
    label, rows, body = next(w for w in writes if w[0] == "port Module properties")
    assert "m.scopeFiles = row.scopeFiles" in body
    scope = {r["name"]: r["scopeFiles"] for r in rows}
    assert scope.pop(SB) == FLT_FILE
    assert set(scope.values()) == {None}
    assert {r["name"]: r["scopeRule"] for r in rows}["lucene/queryparser"] == \
        "sprint-13-ports-the-module"
    decision = next(q for q in statements if "Sprint 13 ports" in q and "d.evidence" in q)
    # `javaFiles` stays the whole module's count; the evidence names the scope
    sandbox_files = {r["name"]: r["javaFiles"] for r in rows}[SB]
    assert sandbox_files == 2  # X.java and the descriptor
    assert f"lucene/sandbox {sandbox_files} (in scope: FuzzyLikeThisQuery.java)" in decision
    assert "lucene/queryparser -> lucene/sandbox" in decision


def test_the_model_documents_the_sprint13_scope():
    model = MODEL.read_text(encoding="utf-8")
    for term in ("sprint-13-ports-the-module", "sprint-13-ports-listed-files",
                 "sprint-13-module-top-level", "sprint-13-listed-file-top-level",
                 "`scopeFiles`", "lucene/queries", "lucene/queryparser", "lucene/sandbox"):
        assert term in model, term


QP_SRC = "lucene/queryparser/src/java/org/apache/lucene/queryparser/classic"
QP = "org.apache.lucene.queryparser.classic"
# The shape JavaCC writes (`classic/QueryParser.java:82`): the annotation on the
# same line as the declaration, nested types declared `static private final`.
QP_JAVA = """/* QueryParser.java */
/* Generated By:JavaCC: Do not edit this line. QueryParser.java */
package org.apache.lucene.queryparser.classic;

import java.util.Locale;

@SuppressWarnings({"unused","null"}) public class QueryParser extends QueryParserBase implements QueryParserConstants {
  final public int Conjunction() throws ParseException { return 0; }
  static private final class LookaheadSuccess extends java.lang.Error {
    @Override
    public Throwable fillInStackTrace() { return this; }
  }
  static final class JJCalls {
    int gen;
  }
}
"""


def test_an_annotation_on_the_declaration_line_is_seen_by_every_extractor(tmp_path):
    """Regression: the three JavaCC parsers of lucene/queryparser were missing.

    Every extractor anchored a declaration at `^\\s*` plus modifiers only, so
    `@SuppressWarnings({"unused","null"}) public class QueryParser` matched none,
    and with no top-level type on the stack its nested types and members were
    dropped too; the audit's census saw them (graph 218, real 221 top-level).
    """
    lucene = tmp_path / "lucene"
    write(lucene, f"{QP_SRC}/QueryParser.java", QP_JAVA)
    # an annotation on the line before the declaration still yields one type
    write(lucene, f"{QP_SRC}/QueryParserBase.java",
          f"/* licence */\npackage {QP};\n\n@Deprecated\npublic abstract class QueryParserBase {{}}\n")
    write(lucene, f"{QP_SRC}/QueryParserConstants.java",
          f"/* licence */\npackage {QP};\npublic interface QueryParserConstants {{ int EOF = 0; }}\n")
    data = pm.survey_module("lucene/queryparser", pm.repo_roots(lucene, tmp_path / "rt"))
    assert sorted(t["qualified"] for t in data["types"]) == [
        f"{QP}.QueryParser", f"{QP}.QueryParserBase", f"{QP}.QueryParserConstants"]
    assert sorted(n["qualifiedName"] for n in data["nested"]) == [
        f"{QP}.QueryParser$JJCalls", f"{QP}.QueryParser$LookaheadSuccess"]
    members = {(x["parentQualifiedName"], x["name"])
               for v in data["members"].values() for x in v}
    assert {(f"{QP}.QueryParser", "Conjunction"), (f"{QP}.QueryParser$JJCalls", "gen"),
            (f"{QP}.QueryParser$LookaheadSuccess", "fillInStackTrace")} <= members

    top, deps = pc.scan_lucene(str(lucene), str(tmp_path / "rt"))
    assert sorted(top) == [f"{QP}.QueryParser", f"{QP}.QueryParserBase",
                           f"{QP}.QueryParserConstants"]
    assert (f"{QP}.QueryParser", f"{QP}.QueryParserBase") in deps
    assert (f"{QP}.QueryParser", f"{QP}.QueryParserConstants") in deps

    _files, ctop, cnested = audit.java_census(lucene, ("lucene/queryparser/src/java",))
    assert sorted(q for q, _ in ctop) == sorted(top)
    assert sorted(n for n, _ in cnested) == ["JJCalls", "LookaheadSuccess"]
