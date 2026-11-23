"""The incremental sync (#637): `sync_plan.py`, `compare_kg.py`, the
`REFERENCES` difference of `rust_refs_kg.py` and the expansion reuse of
`extract_rucene_kg.py`."""
import json
import subprocess
from pathlib import Path

import pytest

import compare_kg as ck
import extract_rucene_kg as ex
import rust_refs_kg as rr
import sync_plan as sp


# --- sync_plan: which crate a path is, and what a diff reaches ---------------

DIRS = {"rucene": "", "rucene-facet": "crates/rucene-facet",
        "rucene-benchmark": "crates/rucene-benchmark", "nekohtml": "crates/nekohtml"}
DEPS = {"rucene": set(), "rucene-facet": {"rucene"}, "nekohtml": {"rucene"},
        "rucene-benchmark": {"rucene", "rucene-facet", "nekohtml"}}


@pytest.mark.parametrize("path, crate", [
    ("crates/rucene-facet/src/lib.rs", "rucene-facet"),
    ("crates/rucene-facet/Cargo.toml", "rucene-facet"),
    ("src/index/index_writer.rs", "rucene"),
    ("tests/merge_panic.rs", "rucene"),
    ("Cargo.toml", "rucene"),
    ("build.rs", "rucene"),
    ("Cargo.lock", "*"),
    ("rust-toolchain.toml", "*"),
    (".cargo/config.toml", "*"),
    ("README.md", None),
    ("specs/modules/facet.md", None),
    ("tools/kg/sync_plan.py", None),
])
def test_crate_of(path, crate):
    assert sp.crate_of(path, DIRS) == crate


def test_dependents_are_transitive():
    assert sp.dependents({"rucene-facet"}, DEPS) == {"rucene-facet", "rucene-benchmark"}
    assert sp.dependents({"rucene"}, DEPS) == set(DEPS)
    assert sp.dependents(set(), DEPS) == set()


def _git(repo, *args):
    return subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t",
                           "-c", "commit.gpgsign=false", *args], cwd=repo, check=True,
                          capture_output=True, text=True).stdout.strip()


def _write(repo, rel, text):
    p = repo / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text, encoding="utf-8")


@pytest.fixture
def workspace_repo(tmp_path, monkeypatch):
    """A three-crate workspace: `rucene` <- `rucene-facet` <- `rucene-benchmark`
    (a dev-dependency), with a README one crate includes."""
    repo = tmp_path / "ws"
    repo.mkdir()
    _git(repo, "init", "-q", "-b", "main")
    _write(repo, "Cargo.toml", '[package]\nname = "rucene"\n[workspace]\n')
    _write(repo, "src/lib.rs", "pub fn a() {}\n")
    _write(repo, "crates/rucene-facet/Cargo.toml",
           '[package]\nname = "rucene-facet"\n[dependencies]\nrucene = { path = "../.." }\n')
    _write(repo, "crates/rucene-facet/src/lib.rs",
           'pub fn f() {}\n#[doc = include_str!("../../../NOTES.md")]\npub struct N;\n')
    _write(repo, "crates/rucene-benchmark/Cargo.toml",
           '[package]\nname = "rucene-benchmark"\n[dependencies]\nrucene = { path = "../.." }\n'
           '[dev-dependencies]\nrucene-facet = { path = "../rucene-facet" }\n')
    _write(repo, "crates/rucene-benchmark/src/lib.rs", "pub fn b() {}\n")
    _write(repo, "NOTES.md", "notes\n")
    _write(repo, "README.md", "readme\n")
    _git(repo, "add", "-A")
    _git(repo, "commit", "-q", "-m", "base")
    monkeypatch.setattr(sp, "REPO", repo)
    return repo


def _commit(repo, changes):
    for rel, text in changes.items():
        _write(repo, rel, text)
    _git(repo, "add", "-A")
    _git(repo, "commit", "-q", "-m", "change")
    return _git(repo, "rev-parse", "HEAD~1"), _git(repo, "rev-parse", "HEAD")


def _changed(repo, base, head):
    return _git(repo, "diff", "--name-only", "--no-renames", base, head).split()


def test_workspace_reads_members_and_dev_dependencies(workspace_repo):
    head = _git(workspace_repo, "rev-parse", "HEAD")
    dirs, deps = sp.workspace(head)
    assert dirs == {"rucene": "", "rucene-facet": "crates/rucene-facet",
                    "rucene-benchmark": "crates/rucene-benchmark"}
    assert deps == {"rucene": set(), "rucene-facet": {"rucene"},
                    "rucene-benchmark": {"rucene", "rucene-facet"}}


def test_a_leaf_change_re_expands_and_re_indexes_only_the_leaf(workspace_repo):
    base, head = _commit(workspace_repo, {"crates/rucene-benchmark/src/lib.rs": "pub fn c() {}\n"})
    crates, expand, index, loose = sp.affected(base, head, _changed(workspace_repo, base, head))
    assert (crates, expand, index, loose) == (
        {"rucene-benchmark"}, {"rucene-benchmark"}, {"rucene-benchmark"}, [])


def test_a_root_change_re_indexes_its_dependents_but_re_expands_only_itself(workspace_repo):
    base, head = _commit(workspace_repo, {"src/lib.rs": "pub fn a2() {}\n"})
    crates, expand, index, _ = sp.affected(base, head, _changed(workspace_repo, base, head))
    assert crates == {"rucene"} and expand == {"rucene"}
    assert index == {"rucene", "rucene-facet", "rucene-benchmark"}


def test_a_changed_macro_re_expands_every_dependent(workspace_repo):
    base, head = _commit(workspace_repo, {
        "src/lib.rs": "#[macro_export]\nmacro_rules! m { () => {} }\npub fn a() {}\n"})
    _, expand, _, _ = sp.affected(base, head, _changed(workspace_repo, base, head))
    assert expand == {"rucene", "rucene-facet", "rucene-benchmark"}


def test_an_included_file_belongs_to_the_crate_that_includes_it(workspace_repo):
    base, head = _commit(workspace_repo, {"NOTES.md": "changed\n", "README.md": "changed\n"})
    crates, expand, index, loose = sp.affected(base, head, _changed(workspace_repo, base, head))
    assert crates == {"rucene-facet"} and expand == {"rucene-facet"}
    assert index == {"rucene-facet", "rucene-benchmark"}
    assert loose == ["README.md"]


def test_a_documentation_change_reaches_no_crate(workspace_repo):
    base, head = _commit(workspace_repo, {"README.md": "changed\n"})
    assert sp.affected(base, head, _changed(workspace_repo, base, head)) == (
        set(), set(), set(), ["README.md"])


def test_a_lockfile_change_reaches_every_crate(workspace_repo):
    base, head = _commit(workspace_repo, {"Cargo.lock": "# lock\n"})
    crates, expand, index, _ = sp.affected(base, head, _changed(workspace_repo, base, head))
    assert crates == expand == index == {"rucene", "rucene-facet", "rucene-benchmark"}


def test_pretend_unchanged_drops_a_changed_crate(workspace_repo):
    base, head = _commit(workspace_repo, {"crates/rucene-facet/src/lib.rs": "pub fn g() {}\n"})
    changed = _changed(workspace_repo, base, head)
    assert sp.affected(base, head, changed, ("rucene-facet",))[:3] == (set(), set(), set())


# --- sync_plan: eligibility ---------------------------------------------------

@pytest.fixture
def planner(workspace_repo, tmp_path, monkeypatch):
    target = tmp_path / "kgt"
    target.mkdir()
    monkeypatch.setattr(sp, "kg_target", lambda: target)
    monkeypatch.setattr(sp, "fingerprint", lambda roots=None: "fp1")
    synced = {"commit": None}
    monkeypatch.setattr(sp, "synced_commit", lambda roadmap: synced["commit"])
    return workspace_repo, target, synced


def test_without_a_state_the_sync_is_full(planner):
    repo, _target, _ = planner
    out, path = sp.plan("HEAD", "rucene")
    assert out["mode"] == "full" and "no state file" in out["reason"]
    assert json.loads(path.read_text())["mode"] == "full"


def test_a_recorded_state_matching_the_graph_plans_incrementally(planner):
    repo, target, synced = planner
    base = _git(repo, "rev-parse", "HEAD")
    synced["commit"] = base
    sp.record(base, "rucene", "/r/refs.jsonl")
    _commit(repo, {"crates/rucene-facet/src/lib.rs": "pub fn g() {}\n"})
    out, _ = sp.plan("HEAD", "rucene")
    assert (out["mode"], out["base"], out["refs"]) == ("incremental", base, "/r/refs.jsonl")
    assert out["crates"] == ["rucene-facet"]
    assert out["index"] == ["rucene-benchmark", "rucene-facet"]


@pytest.mark.parametrize("spoil, reason", [
    ("full", "--full"),
    ("synced", "is not the recorded"),
    ("fingerprint", "tools, clones or toolchain changed"),
    ("roadmap", "state recorded for roadmap"),
])
def test_an_untrusted_base_falls_back_to_full(planner, monkeypatch, spoil, reason):
    repo, target, synced = planner
    base = _git(repo, "rev-parse", "HEAD")
    synced["commit"] = base
    sp.record(base, "other" if spoil == "roadmap" else "rucene", "/r")
    _commit(repo, {"src/lib.rs": "pub fn z() {}\n"})
    if spoil == "synced":
        synced["commit"] = _git(repo, "rev-parse", "HEAD")
    if spoil == "fingerprint":
        monkeypatch.setattr(sp, "fingerprint", lambda roots=None: "fp2")
    out, _ = sp.plan("HEAD", "rucene", force_full=(spoil == "full"))
    assert out["mode"] == "full" and reason in out["reason"]


def test_record_refuses_a_graph_synced_elsewhere(planner):
    repo, _target, synced = planner
    synced["commit"] = "0" * 40
    with pytest.raises(SystemExit, match="nothing recorded"):
        sp.record("HEAD", "rucene", "/r")


def test_the_fingerprint_follows_tools_and_clones(tmp_path, monkeypatch):
    clone = tmp_path / "clone"
    (clone / "a").mkdir(parents=True)
    (clone / "a" / "X.java").write_text("class X {}\n")
    monkeypatch.setattr(sp, "_version", lambda cmd: "v")
    one = sp.fingerprint({"lib": clone})
    assert sp.fingerprint({"lib": clone}) == one
    (clone / "a" / "X.java").write_text("class X { int y; }\n")
    assert sp.fingerprint({"lib": clone}) != one


# --- compare_kg -----------------------------------------------------------------

def _dump(nodes, edges):
    labels, types = {}, {}
    for lbls, _p in nodes:
        for lb in lbls:
            labels[lb] = labels.get(lb, 0) + 1
    ids = [ck.node_identity(lbls, p) for lbls, p in nodes]
    es = []
    for t, a, b, p in edges:
        types[t] = types.get(t, 0) + 1
        es.append(ck.edge_identity(t, ck.sha(ids[a]), ck.sha(ids[b]), p))
    return {"labels": labels, "types": types, "unlabelled": 0,
            "nodes": sorted(ids), "edges": sorted(es)}


def test_provenance_is_excluded_and_content_is_not():
    a = _dump([(["Class"], {"name": "A", "gitCommit": "1"}), (["File"], {"path": "x"})],
              [("DECLARES", 1, 0, {"gitCommit": "1", "gitDate": "d"})])
    b = _dump([(["File"], {"path": "x", "gitDate": "e"}), (["Class"], {"name": "A",
                                                                      "gitCommit": "2"})],
              [("DECLARES", 0, 1, {"gitCommit": "2"})])
    equal, lines = ck.compare(a, b)
    assert equal and lines[-1].endswith("only in B 0")
    c = _dump([(["Class"], {"name": "A", "portState": "ported"}), (["File"], {"path": "x"})],
              [("DECLARES", 1, 0, {})])
    equal, lines = ck.compare(a, c)
    assert not equal and any("only in B" in ln and "ported" in ln for ln in lines)


def test_a_duplicated_edge_or_a_missing_one_is_a_difference():
    nodes = [(["Class"], {"name": "A"}), (["Class"], {"name": "B"})]
    one = _dump(nodes, [("EXTENDS", 0, 1, {})])
    two = _dump(nodes, [("EXTENDS", 0, 1, {}), ("EXTENDS", 0, 1, {})])
    none = _dump(nodes, [])
    assert not ck.compare(one, two)[0] and not ck.compare(one, none)[0]
    assert ck.compare(one, one)[0]


# --- rust_refs_kg: the REFERENCES difference -------------------------------------

def _row(src, dst, prod, test=0, ev=rr.EVIDENCE):
    return {"src": ["RustFn", src], "dst": ["RustFn", dst], "prod": prod, "test": test,
            "evidence": ev, "commit": "c"}


def test_load_diff_deletes_recounts_and_creates_only_the_difference(monkeypatch, capsys):
    writes, created = [], []
    monkeypatch.setattr(rr, "unwind_maps", lambda roadmap, mode, maps, body, label:
                        writes.append((mode, label, body, list(maps))) or {})
    monkeypatch.setattr(rr, "_create", lambda roadmap, rows, commit, date:
                        created.extend(rows))
    previous = [_row("a", "b", 1), _row("a", "c", 2), _row("x", "y", 5)]
    rows = [_row("a", "b", 1), _row("a", "c", 3), _row("n", "m", 1)]
    rr.load_diff("rucene", rows, previous, "c" * 40, "2026-11-23")
    by = {label: (mode, body, maps) for mode, label, body, maps in writes}
    mode, body, maps = by["REFERENCES delete RustFn->RustFn"]
    assert mode == "delete" and body.endswith("DELETE e") and len(maps) == 1 and "'x'" in maps[0]
    mode, body, maps = by["REFERENCES recount RustFn->RustFn"]
    assert "SET e.prodRefs = row.p" in body and len(maps) == 1 and "p:3" in maps[0]
    assert [r["src"][1] for r in created] == ["n"]
    assert "1 deleted, 1 re-counted, 1 to create, 1 unchanged" in capsys.readouterr().err


def test_an_unchanged_extraction_writes_nothing(monkeypatch):
    calls = []
    monkeypatch.setattr(rr, "unwind_maps", lambda *a: calls.append(a) or {})
    monkeypatch.setattr(rr, "_create", lambda *a: calls.append(a))
    rows = [_row("a", "b", 1)]
    rr.load_diff("rucene", rows, rows, "c" * 40, "d")
    assert calls == []


# --- extract_rucene_kg: reused expansions ------------------------------------------

def test_reused_expansions_skip_the_fresh_crates(tmp_path, monkeypatch):
    target = tmp_path / "kgt"
    (target / "expanded").mkdir(parents=True)
    monkeypatch.setattr(ex, "kg_target", lambda: target)
    root = target / "rucene-survey-new"
    root.mkdir()
    monkeypatch.setattr(ex, "WORKSPACE", {"crates/a": ("crate-a", "crate_a"),
                                          "crates/b": ("crate-b", "crate_b")})
    for name in ("root", "crate-a", "crate-b"):
        (target / "expanded" / f"rucene-survey-old--{name}.rs").write_text(f"// {name}")
    reused = ex.reuse_expansions(root, "rucene-survey-old", {"crate-b"})
    assert reused == ["rucene", "crate-a"]
    assert (target / "expanded" / "rucene-survey-new--crate-a.rs").read_text() == "// crate-a"
    assert not (target / "expanded" / "rucene-survey-new--crate-b.rs").exists()
    assert not ex.all_expansions_cached(root)
    (target / "expanded" / "rucene-survey-new--crate-b.rs").write_text("// b")
    assert ex.all_expansions_cached(root)


# --- kgcache: the content-addressed memo of pure text functions -------------------

def test_disk_memo_returns_what_the_function_returns(tmp_path, monkeypatch):
    import kgcache
    monkeypatch.setattr(kgcache, "kg_target", lambda: tmp_path)
    calls = []

    @kgcache.disk_memo
    def shout(text, suffix="!"):
        calls.append(text)
        return {"out": text.upper() + suffix, "parts": [text]}

    assert shout("abc") == {"out": "ABC!", "parts": ["abc"]}
    assert shout("abc") == {"out": "ABC!", "parts": ["abc"]}       # served from disk
    assert shout("abd") == {"out": "ABD!", "parts": ["abd"]}       # another text misses
    assert shout("abc", suffix="?") == {"out": "ABC?", "parts": ["abc"]}
    assert calls == ["abc", "abd", "abc"]
    shout("abc")["parts"].append("x")                               # a fresh object each time
    assert shout("abc")["parts"] == ["abc"]
    monkeypatch.setenv("KG_CACHE", "off")
    shout("abc")
    assert calls == ["abc", "abd", "abc", "abc"]


def test_memoized_parse_equals_the_uncached_parse(tmp_path, monkeypatch):
    import kgcache
    monkeypatch.setattr(kgcache, "kg_target", lambda: tmp_path)
    text = ("/// Equivalent to `org.apache.lucene.util.Foo`.\n"
            "pub struct Foo { x: i32 }\nimpl Foo {\n    pub fn get(&self) -> i32 { self.x }\n}\n"
            "macro_rules! m { () => { struct Hidden; } }\n")
    cold = ex.parse_file("src/a.rs", text)
    warm = ex.parse_file("src/a.rs", text)
    monkeypatch.setenv("KG_CACHE", "off")
    assert cold == warm == ex.parse_file("src/a.rs", text)
    assert rr.scan_regions(text) == rr.scan_regions.__wrapped__(text)
