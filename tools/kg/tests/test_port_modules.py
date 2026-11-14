"""Task #323: the port modules beyond lucene/core in the KG -- sprint 9's
lucene/test-framework, lucene/codecs and randomizedtesting 2.8.1, and sprint 12's
lucene/analysis/common (`test_skeletons.py`) -- and the workspace crates that
port them. Off the store: every graph call is replaced by a recorder."""
import json
import sys
from pathlib import Path

import ask
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


# --- the module registry ----------------------------------------------------

def test_the_registry_names_the_five_modules_and_their_crates():
    assert [m["name"] for m in pm.PORT_MODULES] == [
        "lucene/core", "lucene/codecs", "lucene/test-framework",
        "randomizedtesting/randomized-runner", "lucene/analysis/common"]
    assert pm.ADDED_MODULES == ("lucene/codecs", "lucene/test-framework",
                                "randomizedtesting/randomized-runner", "lucene/analysis/common")
    assert {m["cratePath"] for m in pm.PORT_MODULES} == {
        ".", "crates/rucene-codecs", "crates/rucene-test-framework", "crates/randomizedtesting",
        "crates/rucene-analysis-common"}


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
    deps = pm.module_dependencies(pm.repo_roots(lucene, rt))
    assert [(a, b) for a, b, _ in deps] == [
        ("lucene/codecs", "lucene/core"),
        ("lucene/test-framework", "lucene/core"),
        ("lucene/test-framework", "lucene/codecs"),
        ("lucene/test-framework", "randomizedtesting/randomized-runner"),
        ("lucene/analysis/common", "lucene/core")]


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
