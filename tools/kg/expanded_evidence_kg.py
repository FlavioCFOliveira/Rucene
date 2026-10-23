#!/usr/bin/env python3
"""Port evidence that only exists after macro expansion.

`port_evidence_kg.py` reads the crate's equivalence claims from literal source
lines. A type produced by a `macro_rules!` macro has none: neither its
declaration nor its doc comment is written anywhere a line-oriented scanner can
see them. `src/document/range_doc_values_fields.rs` says

    #[doc = concat!("Equivalent to `", $field_equiv, "`, which extends")]
    pub struct $field_name { … }

and the claim about `org.apache.lucene.document.IntRangeDocValuesField` only
becomes text when the compiler expands the template. So the claim exists, the
crate does state it, and the graph could not read it.

The consequence was instrumental, not real. 32 in-scope Lucene types sat at
`portState = 'ported'` with no `PORTS` edge to anchor -- the 24
`util.packed.BulkOperationPackedN` and the eight `document` range doc-values
fields and their slow range queries -- and `measure_depth` in
`port_evidence_kg.py` resolves a ported type's Rust functions through
`(name, file)` on a `PORTS` edge, so none of them was ever measured. The graph
therefore could not answer "how much is left to port" for them, which
`CLAUDE.md` 6.1 requires it to answer, and the shortfall came from the reader,
not from the port.

This loader closes that by taking the same primary source `audit_kg.py` already
trusts -- `cargo +nightly rustc --lib -- -Zunpretty=expanded`, the crate as the
compiler itself sees it (`CLAUDE.md` 16.4) -- and reading the doc comment the
expander prints above each generated item. What counts as a claim is unchanged:
`claim_in` from `port_evidence_kg.py` decides, so an expansion claim has to meet
exactly the bar a hand-written one meets, and a bare mention is still ignored.

Two properties keep this honest and tellable apart from the literal reading:

* the node carries `origin: "macro"` (set by `extract_rucene_kg.py --expand`
  and `load_rucene_kg.py`), which says no source line declares it;
* the edge carries `declarationSource: "macro-expansion"`, which says the claim
  itself was read from the compiler's expansion rather than from a source line.
  `evidence` stays `"doc-comment"`, because that is what it is.

Scope guard: only items the *literal* parser did not record are considered, so
this loader can never restate, override or contradict what
`port_evidence_kg.py` reads. It adds nothing but the claims that were invisible.

Run it after `port_evidence_kg.py --phase load` and before `--phase depth`, so
the depth measurement sees the edges this writes.

Usage:
    python3 tools/kg/expanded_evidence_kg.py rucene --source-root . \\
        --commit "$COMMIT" --date "$DATE" [--expanded-file FILE] [--dry-run]
"""
import argparse
import json
import os
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import extract_rucene_kg as ex
import port_evidence_kg as pe
from kgio import PER_ROW_SCAN_BATCH, read as kg_read, unwind

# `#[doc = "…"]`, as the pretty-printer prints it: it wraps after the `=`, so
# the string routinely starts on the next line, and `concat!` has already been
# folded into a single literal. The body accepts any escape, since a doc that
# contains a quote is escaped rather than terminated.
RE_DOC_ATTR = re.compile(r"#\[\s*doc\s*=\s*\"((?:[^\"\\]|\\.)*)\"\s*\]")
# A literal `///` line, which survives expansion verbatim. `////` is a plain
# comment and `//!` documents the enclosing module, not this item.
RE_DOC_LINE = re.compile(r"^[ \t]*///(?!/)(.*)$", re.M)
RE_ESCAPE = re.compile(r"\\(u\{[0-9a-fA-F]{1,6}\}|x[0-9a-fA-F]{2}|.)", re.S)

TYPE_LABEL = {"struct": "RustStruct", "union": "RustStruct",
              "enum": "RustEnum", "trait": "RustTrait"}


def unescape(s: str) -> str:
    """A Rust string literal's value, from the escaped form rustc prints."""

    def sub(m):
        tok = m.group(1)
        if tok.startswith("u{"):
            return chr(int(tok[2:-1], 16))
        if tok.startswith("x"):
            return chr(int(tok[1:], 16))
        return {"n": "\n", "t": "\t", "r": "\r", "0": "\0",
                "\\": "\\", '"': '"', "'": "'", "\n": ""}.get(tok, tok)

    return RE_ESCAPE.sub(sub, s)


def doc_lines(header: str):
    """The doc comment of one expanded item, as lines, in source order.

    An item's doc reaches the expansion in two shapes at once -- `#[doc = "…"]`
    for anything a macro built, `///` for anything written literally -- and a
    macro-generated item usually carries both, interleaved. Reading them in
    positional order reconstructs the doc the compiler actually attached.
    """
    parts = []
    for m in RE_DOC_ATTR.finditer(header):
        parts.append((m.start(), unescape(m.group(1))))
    for m in RE_DOC_LINE.finditer(header):
        parts.append((m.start(), m.group(1)))
    out = []
    for _pos, text in sorted(parts):
        out.extend(text.split("\n"))
    return [line.strip() for line in out]


def literal_declarations(root: Path):
    """`(mod_to_file, declared)` from the crate's literal source lines.

    `declared` is every `(name, file)` the line-oriented parser records, and it
    is the guard this loader runs under: an item in it is `port_evidence_kg.py`'s
    to read, never this one's.
    """
    mod_to_file, declared = {}, set()
    for dirpath, _dirs, names in os.walk(root / "src"):
        for fn in sorted(names):
            if not fn.endswith(".rs"):
                continue
            rel = str(Path(dirpath, fn).relative_to(root)).replace(os.sep, "/")
            mp = ex.module_path_for(rel)
            if mp:
                mod_to_file[mp] = rel
            text = (root / rel).read_text(encoding="utf-8", errors="ignore")
            types, _fns, _impls, _mods, _uses = ex.parse_file(rel, text)
            declared.update((t["name"], rel) for t in types)
    return mod_to_file, declared


def extract(root: Path, expanded: str):
    """Equivalence claims of the types no source line declares."""
    mod_to_file, declared = literal_declarations(root)
    claims, seen_types = [], set()
    for rec in ex.parse_expanded_records(expanded):
        if rec["kind"] != "type":
            continue
        rel, _suffix = ex.resolve_module_file(rec["mods"], mod_to_file)
        if rel is None or (rec["name"], rel) in declared:
            continue
        seen_types.add((rec["name"], rel))
        for line in doc_lines(rec["header"]):
            fqn, note = pe.claim_in(line)
            if fqn:
                claims.append({"name": rec["name"], "file": rel,
                               "label": TYPE_LABEL[rec["decl"]],
                               "lucene": fqn, "note": note})
    # One claim per (rust item, lucene type): a doc may restate its equivalence
    # in more than one paragraph, and the graph holds one edge either way.
    uniq, seen = [], set()
    for c in claims:
        k = (c["name"], c["file"], c["lucene"])
        if k not in seen:
            seen.add(k)
            uniq.append(c)
    print(f"  macro-generated types seen: {len(seen_types)}", file=sys.stderr)
    print(f"  equivalence claims among them: {len(uniq)}", file=sys.stderr)
    return uniq


def load(roadmap, claims, commit, date):
    """Write the claims as `PORTS` edges, on nodes with the model's identity."""
    all_classes = [r[0] for r in kg_read(roadmap, "MATCH (c:Class) RETURN c.qualifiedName")
                   if r[0]]
    # The same index `port_evidence_kg.py` resolves against, for the same reason:
    # a macro writes the nested name exactly as a hand-written doc does. The hppc
    # templates say `Port of `IntObjectHashMap.KeysContainer``, with the package
    # left to the module around them, so an exact-name index alone dropped every
    # one of those claims on the floor.
    classes, short_classes = pe.build_class_index(all_classes)

    rows, unresolved = [], set()
    for c in claims:
        target = pe.resolve_type(c["lucene"], classes, short_classes)
        if target is None:
            unresolved.add(c["lucene"])
            continue
        rows.append({"name": c["name"], "file": c["file"], "label": c["label"],
                     "lucene": target, "note": c.get("note"),
                     "commit": commit, "date": date})
    if unresolved:
        print(f"  claims naming no known Lucene type: {len(unresolved)} "
              f"{sorted(unresolved)[:3]}", file=sys.stderr)

    # The nodes. `extract_rucene_kg.py --expand` and `load_rucene_kg.py` already
    # create them from the same expansion, so this is normally a no-op -- but it
    # is written here too, with the same labels and the same `(name, file)`
    # identity, so this loader does not depend on the order of a pipeline it
    # does not control. A count above zero means the survey was taken without
    # `--expand` and should be retaken.
    before = len(kg_read(roadmap, "MATCH (t) WHERE t.origin = 'macro' RETURN t.name"))
    for label in sorted({r["label"] for r in rows}):
        sel = [r for r in rows if r["label"] == label]
        unwind(roadmap, "create", [{"name": r["name"], "file": r["file"]} for r in sel],
               f"MERGE (t:{label} {{name:row.name, file:row.file}})",
               f"{label} nodes")
        unwind(roadmap, "create", [{"name": r["name"], "file": r["file"]} for r in sel],
               f"MATCH (f:File {{path:row.file}}), "
               f"(t:{label} {{name:row.name, file:row.file}}) "
               "MERGE (f)-[:DECLARES]->(t)",
               f"File DECLARES {label}")
    unwind(roadmap, "update", rows,
           "MATCH (t {name:row.name, file:row.file}) "
           "SET t.origin = 'macro', t.scope = coalesce(t.scope, 'crate'), "
           "t.gitCommit = row.commit, t.gitDate = row.date",
           "node provenance", batch=PER_ROW_SCAN_BATCH)
    after = len(kg_read(roadmap, "MATCH (t) WHERE t.origin = 'macro' RETURN t.name"))
    print(f"  macro-origin nodes: {before} -> {after}", file=sys.stderr)

    # Retire the macro-expansion edges this expansion no longer declares. The
    # other half of the pipeline cannot do it: `load_rucene_kg.py --phase prune`
    # deliberately spares a `declarationSource: "macro-expansion"` edge, because
    # a survey taken without `--expand` cannot see the type it starts from, and
    # `port_evidence_kg.py` never reads the dump. This loader does, so it is the
    # only place that knows such a claim has gone -- and without this, a
    # generated type the crate stops generating keeps its port for ever, the
    # same coverage inflation `CLAUDE.md` 6.1 forbids. An expansion that yielded
    # no claim at all retires nothing, for the same reason an empty survey
    # prunes nothing: a failed run is not evidence.
    if rows:
        live = {(r["name"], r["file"], r["lucene"]) for r in rows}
        dead = [{"name": n, "file": f, "lucene": l} for n, f, l in kg_read(
            roadmap,
            "MATCH (t)-[e:PORTS]->(c:Class) "
            "WHERE e.declarationSource = 'macro-expansion' "
            "AND e.evidence = 'doc-comment' "
            "RETURN t.name, t.file, c.qualifiedName")
            if (n, f, l) not in live]
        for d in dead:
            print(f"  RETIRED (no live expansion claim): {d['name']} @ "
                  f"{d['file']} -> {d['lucene']}", file=sys.stderr)
        unwind(roadmap, "delete", dead,
               "MATCH (t {name:row.name, file:row.file})-[e:PORTS]->"
               "(c:Class {qualifiedName:row.lucene}) DELETE e",
               "PORTS edges retired", batch=PER_ROW_SCAN_BATCH)

    # `MERGE (a)-[:PORTS]->(b)` silently matches a relationship of a *different*
    # type between the same ordered pair (see `knowledge-model.md`), so a pair
    # already joined by `PORTS_CANDIDATE` would never gain its `PORTS` edge.
    # Drop the candidate first: a declared equivalence supersedes a name guess.
    unwind(roadmap, "delete", rows,
           "MATCH (t {name:row.name, file:row.file})-[e:PORTS_CANDIDATE]->"
           "(c:Class {qualifiedName:row.lucene}) DELETE e",
           "superseded PORTS_CANDIDATE removed", batch=PER_ROW_SCAN_BATCH)
    unwind(roadmap, "create", rows,
           "MATCH (t {name:row.name, file:row.file}) "
           "WITH t, row MATCH (c:Class {qualifiedName:row.lucene}) "
           "MERGE (t)-[:PORTS]->(c)",
           "PORTS edges", batch=PER_ROW_SCAN_BATCH)
    # `declaredAt` names the file the compiler places the generated item in --
    # the macro's invocation site, not the file that declares the `macro_rules!`
    # (see `resolve_module_file`). There is no line to quote: the declaration
    # this claim documents exists only after expansion, which is exactly what
    # `declarationSource` records.
    unwind(roadmap, "update", rows,
           "MATCH (t {name:row.name, file:row.file})-[e:PORTS]->"
           "(c:Class {qualifiedName:row.lucene}) "
           "SET e.evidence = coalesce(e.evidence, 'doc-comment'), "
           "e.declarationSource = 'macro-expansion', "
           "e.declaredAt = row.file, e.note = coalesce(row.note, e.note), "
           "e.gitCommit = row.commit, e.gitDate = row.date",
           "PORTS evidence stamped", batch=PER_ROW_SCAN_BATCH)
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("roadmap")
    ap.add_argument("--source-root", default=".")
    ap.add_argument("--expanded-file",
                    help="a captured `-Zunpretty=expanded` dump; without it the "
                         "compiler is run, which needs the nightly toolchain")
    ap.add_argument("--out", default="/tmp/rucene_kg/expanded_evidence.json")
    ap.add_argument("--commit")
    ap.add_argument("--date")
    ap.add_argument("--dry-run", action="store_true",
                    help="extract and report, write nothing to the graph")
    args = ap.parse_args()

    root = Path(args.source_root).resolve()
    expanded = (Path(args.expanded_file).read_text(encoding="utf-8")
                if args.expanded_file else ex.run_macro_expansion(root))
    claims = extract(root, expanded)
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    Path(args.out).write_text(json.dumps(claims, indent=1), encoding="utf-8")

    if args.dry_run:
        return
    if not (args.commit and args.date):
        raise SystemExit("--commit and --date are required to load")
    load(args.roadmap, claims, args.commit, args.date)


if __name__ == "__main__":
    main()
