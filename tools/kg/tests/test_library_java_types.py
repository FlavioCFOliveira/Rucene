"""The Java types of the sprint-17 libraries survive `load_rucene_kg.py`'s repair.

spatial4j 0.8 and s2-geometry 1.0.0 are cloned with the module at the clone's
root, so their `Class.file` reads `src/main/java/...` (or `src/test/java/...`).
The repair's legacy selector took every `Class` whose `file` starts `src/` for a
Rust node of the pre-survey era and relabelled it as the Rust type with the same
simple name -- the skeleton -- which is how the sync of ae7df7c lost 70 of the
87 library types from the Lucene side (log: `legacy nodes: 70 -> type labels`,
and 17 `UNRESOLVED legacy node` rows whose `file` is a `.java` path). These tests
drive `phase_repair` with the graph client replaced by a recorder.
"""
import load_rucene_kg as lo

JAVA_FILE = "src/main/java/org/locationtech/spatial4j/context/SpatialContext.java"
RUST_FILE = "crates/spatial4j/src/context/spatial_context.rs"


class Survey:
    """The `Survey` surface `phase_repair` uses, with one Rust type that has the
    Java type's simple name (the skeleton) and one legacy Rust type."""
    commit, date = "c" * 40, "2026-11-22"
    files, mods = [], []

    def __init__(self):
        self.types = {
            "SpatialContext": {"name": "SpatialContext", "file": RUST_FILE,
                               "qualifiedName": "spatial4j::context::spatial_context::"
                                                "SpatialContext",
                               "kind": "struct", "visibility": "pub", "scope": "crate"},
            "Legacy": {"name": "Legacy", "file": "src/legacy.rs",
                       "qualifiedName": "rucene::legacy::Legacy", "kind": "struct",
                       "visibility": "pub", "scope": "crate"},
        }

    def resolve_type(self, name, _file, _qn):
        return self.types.get(name)

    def resolve_fn(self, _name, _file, _qn):
        return None


def run_repair(monkeypatch, legacy_rows):
    statements, unwinds = [], []
    def read(_roadmap, q):
        if "RETURN id(n) AS id, labels(n) AS labels" in q:
            statements.append(q)
            return legacy_rows
        return []
    monkeypatch.setattr(lo, "read", read)
    monkeypatch.setattr(lo, "rmp", lambda _m, _r, q, *a, **k: statements.append(q) or "{}")
    monkeypatch.setattr(lo, "run_unwind",
                        lambda _m, _r, rows, body, _b, label: unwinds.append((label, rows, body)))
    lo.phase_repair("r", Survey(), 100)
    return statements, unwinds


def relabelled_ids(unwinds):
    return {r["id"] for label, rows, _b in unwinds if label.startswith("relabel ")
            for r in rows}


def test_a_library_java_class_is_never_relabelled_as_a_rust_type(monkeypatch):
    rows = [
        # the spatial4j Java type, as the structure phase writes it
        {"id": 1, "labels": ["Class"], "name": "SpatialContext", "file": JAVA_FILE,
         "qn": "org.locationtech.spatial4j.context.SpatialContext"},
        # a genuine legacy Rust-side `Class`, which the repair does relabel
        {"id": 2, "labels": ["Class"], "name": "Legacy", "file": "src/legacy.rs",
         "qn": "rucene::legacy::Legacy"},
    ]
    statements, unwinds = run_repair(monkeypatch, rows)
    assert relabelled_ids(unwinds) == {2}
    # the selector itself excludes a `.java` file
    (selector,) = [q for q in statements if "RETURN id(n) AS id, labels(n) AS labels" in q]
    assert "NOT n.file ENDS WITH '.java'" in selector


def test_a_java_type_already_swallowed_by_a_rust_label_is_dropped(monkeypatch):
    statements, _ = run_repair(monkeypatch, [])
    for label in lo.ALL_TYPE_LABELS:
        assert f"MATCH (n:{label}) WHERE n.module IS NOT NULL DETACH DELETE n" in statements


def test_is_java_type_keys_on_the_java_file():
    assert lo.is_java_type({"labels": ["Class"], "file": JAVA_FILE})
    assert lo.is_java_type({"labels": ["Class"],
                            "file": "src/main/java/com/google/common/geometry/S2Cell.java"})
    assert not lo.is_java_type({"labels": ["Class"], "file": "src/legacy.rs"})
    assert not lo.is_java_type({"labels": ["Class"], "file": None})
    assert not lo.is_java_type({"labels": ["Struct"], "file": JAVA_FILE})
