#!/usr/bin/env python3
"""Mirror the crate's declared divergences from Lucene 10.5.0 into the KG.

`CLAUDE.md` §14.5 requires every divergence to be declared at its point (item or
module docs) and recorded in the Knowledge Graph with the Lucene source that
defines the original behaviour. The crate writes each one as a doc-comment
paragraph led by a bold `**Divergence…**` (or `**Divergences…**`). This script
reads those paragraphs from a survey root (a `git archive` of the commit) and
writes:

* one `Divergence` node per paragraph, identity `id` = `"<file>:<line>"`;
* `DECLARED_IN` -> the `File` that declares it;
* `CONCERNS` -> the Rust type or function the doc comment documents, when the
  declaration resolves to exactly one node;
* `DIVERGES_FROM` -> each Lucene `Class` whose source file the paragraph cites
  (`IndexWriter.java:120-140`), when exactly one top-level `lucene/core` type is
  declared by a file of that name, carrying the cited `lines`.

The script owns these nodes and edges: it deletes every `Divergence` the scan no
longer finds and rewrites the three edge types, so the graph never keeps a
divergence the code removed.

    python3 tools/kg/divergences_kg.py rucene --source-root <survey> \\
        --commit <full hash> --date <YYYY-MM-DD>
"""
import argparse
import re
import sys
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from kgio import client, counters_of, esc, format_counters, read, unwind  # noqa: E402

LEAD = re.compile(r"^\*\*(Divergences?\b[^*]*)\*\*")
JAVA_REF = re.compile(
    r"\b([A-Z][A-Za-z0-9_]*\.java)(?::(\d+(?:\s*[-–]\s*\d+)?(?:\s*,\s*\d+(?:\s*[-–]\s*\d+)?)*))?")
ATTR = re.compile(r"^\s*#!?\[")
DECL = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:default\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?"
    r"(?:extern\s+\"[^\"]*\"\s+)?"
    r"(fn|struct|enum|trait|type|union|const|static|mod|impl|macro_rules!)\b\s*(.*)$")
FIELD_OR_VARIANT = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*[:({,]?")
IMPL_HEAD = re.compile(r"^(\s*)(?:unsafe\s+)?impl\b(.*)$")
TRAIT_HEAD = re.compile(r"^(\s*)(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?trait\s+([A-Za-z_][A-Za-z0-9_]*)")
NAME = re.compile(r"([A-Za-z_][A-Za-z0-9_]*)")
TEXT_MAX = 1500


def _impl_owner(rest: str):
    """`(owner, is_trait_impl)` of an `impl` header's remainder, or `None`."""
    rest = rest.split("{")[0].split(" where ")[0].strip()
    rest = re.sub(r"^<.*?>\s*", "", rest)  # generic parameters of the impl
    if " for " in f" {rest} ":
        target = rest.split(" for ", 1)[1]
        trait = True
    else:
        target = rest
        trait = False
    target = re.sub(r"<.*", "", target).strip().lstrip("&").strip()
    target = target.split("::")[-1]
    m = NAME.match(target)
    return (m.group(1), trait) if m else None


def _declaration(lines, j):
    """`(kind, name)` of the item a `///` block ending before line `j` documents."""
    n = len(lines)
    while j < n and (ATTR.match(lines[j]) or not lines[j].strip()):
        # a multi-line attribute runs until its closing bracket
        if ATTR.match(lines[j]) and lines[j].count("[") > lines[j].count("]"):
            depth = lines[j].count("[") - lines[j].count("]")
            j += 1
            while j < n and depth > 0:
                depth += lines[j].count("[") - lines[j].count("]")
                j += 1
            continue
        j += 1
    if j >= n:
        return ("unknown", None)
    line = lines[j]
    m = DECL.match(line)
    if m:
        kind, rest = m.group(1), m.group(2)
        if kind == "impl":
            owner = _impl_owner(rest)
            return ("impl", owner[0] if owner else None)
        if kind == "macro_rules!":
            nm = NAME.match(rest.strip())
            return ("macro", nm.group(1) if nm else None)
        nm = NAME.match(rest.strip())
        return (kind, nm.group(1) if nm else None)
    m = FIELD_OR_VARIANT.match(line)
    if m:
        return ("member", m.group(1))
    return ("unknown", None)


def scan_text(path: str, text: str):
    """Every divergence paragraph of one Rust file.

    Returns dicts with `id`, `file`, `line` (1-based, the lead line), `title`,
    `text`, `itemKind`, `item`, `owner` (the inherent `impl` type or trait the
    documented fn sits in, else `None`), `traitImpl`, and `javaRefs`
    (`[(file, lines)]`).
    """
    lines = text.split("\n")
    out = []
    owners = []  # stack of (indent, owner, is_trait_impl)
    i = 0
    n = len(lines)
    while i < n:
        raw = lines[i]
        stripped = raw.lstrip()
        indent = len(raw) - len(stripped)
        while owners and stripped.startswith("}") and indent <= owners[-1][0]:
            owners.pop()
        if stripped.startswith("///") or stripped.startswith("//!"):
            marker = stripped[:3]
            start = i
            block = []
            while i < n and lines[i].lstrip().startswith(marker) and not lines[i].lstrip().startswith(marker + "/"):
                content = lines[i].lstrip()[3:]
                block.append(content[1:] if content.startswith(" ") else content)
                i += 1
            if marker == "///":
                kind, item = _declaration(lines, i)
            else:
                kind, item = "module", None
            owner = owners[-1] if owners and marker == "///" else None
            k = 0
            while k < len(block):
                lead = LEAD.match(block[k].strip())
                if not lead:
                    k += 1
                    continue
                para_start = k
                para = []
                while k < len(block) and block[k].strip():
                    para.append(block[k].strip())
                    k += 1
                body = " ".join(para)
                refs = []
                for jm in JAVA_REF.finditer(body):
                    lines_cited = re.sub(r"\s+", "", jm.group(2) or "")
                    if (jm.group(1), lines_cited) not in refs:
                        refs.append((jm.group(1), lines_cited))
                line_no = start + para_start + 1
                out.append({
                    "id": f"{path}:{line_no}",
                    "file": path,
                    "line": line_no,
                    "title": lead.group(1).strip().rstrip("."),
                    "text": body[:TEXT_MAX],
                    "itemKind": kind,
                    "item": item,
                    "owner": owner[1] if owner and kind == "fn" else None,
                    "traitImpl": bool(owner[2]) if owner and kind == "fn" else False,
                    "javaRefs": refs,
                })
            continue
        m = IMPL_HEAD.match(raw)
        if m and raw.rstrip().endswith("{") and not raw.rstrip().endswith("}"):
            own = _impl_owner(m.group(2))
            if own:
                owners.append((len(m.group(1)), own[0], own[1]))
        else:
            t = TRAIT_HEAD.match(raw)
            if t and raw.rstrip().endswith("{"):
                owners.append((len(t.group(1)), t.group(2), False))
        i += 1
    return out


def scan_tree(root: Path):
    """Every divergence paragraph under `<root>/src`, in path order."""
    found = []
    for p in sorted((root / "src").rglob("*.rs")):
        rel = p.relative_to(root).as_posix()
        found.extend(scan_text(rel, p.read_text(encoding="utf-8", errors="replace")))
    return found


TYPE_KINDS = {"struct": "RustStruct", "union": "RustStruct", "enum": "RustEnum",
              "trait": "RustTrait", "type": "RustAlias"}


def resolve_items(divs, types, fns):
    """`[(divergence id, label, qualifiedName)]` for the `CONCERNS` edges.

    `types` maps `(label, name, file)` to the qualified name; `fns` maps
    `(name, file)` to `[(owner, qualifiedName)]`. A fn resolves when exactly one
    node matches its name, file and owner; a method of an `impl Trait for Type`
    block is not modelled, so it resolves to nothing. An `impl` block's doc
    resolves to the type it implements.
    """
    edges = []
    for d in divs:
        kind, item = d["itemKind"], d["item"]
        if not item:
            continue
        if kind in TYPE_KINDS:
            q = types.get((TYPE_KINDS[kind], item, d["file"]))
            if q:
                edges.append((d["id"], TYPE_KINDS[kind], q))
        elif kind == "impl":
            hits = [(lab, q) for (lab, nm, f), q in types.items() if nm == item and f == d["file"]]
            if len(hits) == 1:
                edges.append((d["id"], hits[0][0], hits[0][1]))
        elif kind == "fn" and not d["traitImpl"]:
            cands = [q for owner, q in fns.get((item, d["file"]), []) if owner == d["owner"]]
            if len(cands) == 1:
                edges.append((d["id"], "RustFn", cands[0]))
    return edges


def resolve_java(divs, classes):
    """`[(divergence id, class qualifiedName, lines)]` for `DIVERGES_FROM`.

    `classes` maps a Java file name (`IndexWriter.java`) to the qualified names
    of the top-level `lucene/core` types declared by files of that name; a file
    name declared by more than one file resolves to nothing.
    """
    edges = []
    for d in divs:
        seen = set()
        for jfile, lines in d["javaRefs"]:
            hits = classes.get(jfile, [])
            if len(hits) == 1:
                key = (hits[0], lines)
                if key not in seen:
                    seen.add(key)
                    edges.append((d["id"], hits[0], lines))
    return edges


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("roadmap")
    ap.add_argument("--source-root", required=True, type=Path)
    ap.add_argument("--commit", required=True)
    ap.add_argument("--date", required=True)
    a = ap.parse_args()
    R, C, D = a.roadmap, a.commit, a.date

    divs = scan_tree(a.source_root)
    print(f"divergences: {len(divs)} paragraphs in {len({d['file'] for d in divs})} files",
          file=sys.stderr)

    client(R, "CREATE INDEX idx_divergence_id IF NOT EXISTS FOR (n:Divergence) ON (n.id)")

    files = sorted({d["file"] for d in divs})
    flist = "[" + ", ".join(esc(f) for f in files) + "]"
    types = {}
    for label in ("RustStruct", "RustEnum", "RustTrait", "RustAlias"):
        for name, file, q in read(R, f"MATCH (t:{label}) WHERE t.file IN {flist} "
                                     "RETURN t.name, t.file, t.qualifiedName"):
            types[(label, name, file)] = q
    fns = defaultdict(list)
    for name, file, owner, q in read(R, f"MATCH (f:RustFn) WHERE f.file IN {flist} "
                                        "RETURN f.name, f.file, f.owner, f.qualifiedName"):
        fns[(name, file)].append((owner, q))
    by_file = defaultdict(set)
    for q, f in read(R, "MATCH (c:Class) WHERE c.portScope = 'in' RETURN c.qualifiedName, c.file"):
        if f:
            by_file[f.rsplit("/", 1)[-1]].add(f)
    file_to_class = {}
    for q, f in read(R, "MATCH (c:Class) WHERE c.portScope = 'in' RETURN c.qualifiedName, c.file"):
        if f:
            file_to_class.setdefault(f, []).append(q)
    classes = {}
    for base, paths in by_file.items():
        if len(paths) == 1:
            qs = file_to_class[next(iter(paths))]
            stem = base[:-5]
            top = [q for q in qs if q.rsplit(".", 1)[-1] == stem]
            classes[base] = top if len(top) == 1 else []

    # Nodes: upsert the current set, then drop what the scan no longer finds.
    rows = [{"id": d["id"], "file": d["file"], "line": d["line"], "title": d["title"],
             "text": d["text"], "itemKind": d["itemKind"], "item": d["item"],
             "javaRefs": "; ".join(f"{f}:{l}" if l else f for f, l in d["javaRefs"]),
             "c": C, "d": D} for d in divs]
    unwind(R, "upsert", rows,
           "MERGE (d:Divergence {id: row.id}) SET d.file = row.file, d.line = row.line, "
           "d.title = row.title, d.text = row.text, d.itemKind = row.itemKind, "
           "d.item = row.item, d.javaRefs = row.javaRefs, d.gitCommit = row.c, d.gitDate = row.d",
           "Divergence nodes", batch=200)
    ids = {d["id"] for d in divs}
    stale = [r[0] for r in read(R, "MATCH (d:Divergence) RETURN d.id") if r[0] not in ids]
    if stale:
        unwind(R, "delete", [{"id": s} for s in stale],
               "MATCH (d:Divergence {id: row.id}) DETACH DELETE d", "stale Divergence nodes")

    # Edges: rewritten whole, so a moved or deleted reference never lingers.
    out = client(R, "MATCH (d:Divergence)-[r:DECLARED_IN|CONCERNS|DIVERGES_FROM]->() DELETE r")
    print(f"  old Divergence edges: {format_counters(counters_of(out))}", file=sys.stderr)
    unwind(R, "create", [{"id": d["id"], "file": d["file"], "c": C, "d": D} for d in divs],
           "MATCH (d:Divergence {id: row.id}) MATCH (f:File {path: row.file}) "
           "CREATE (d)-[:DECLARED_IN {gitCommit: row.c, gitDate: row.d}]->(f)",
           "DECLARED_IN", batch=200)
    items = resolve_items(divs, types, fns)
    for label in ("RustStruct", "RustEnum", "RustTrait", "RustAlias", "RustFn"):
        rows = [{"id": i, "q": q, "c": C, "d": D} for i, lab, q in items if lab == label]
        unwind(R, "create", rows,
               f"MATCH (d:Divergence {{id: row.id}}) MATCH (t:{label} {{qualifiedName: row.q}}) "
               "CREATE (d)-[:CONCERNS {gitCommit: row.c, gitDate: row.d}]->(t)",
               f"CONCERNS -> {label}", batch=200)
    java = resolve_java(divs, classes)
    unwind(R, "create", [{"id": i, "q": q, "lines": l, "c": C, "d": D} for i, q, l in java],
           "MATCH (d:Divergence {id: row.id}) MATCH (c:Class {qualifiedName: row.q}) "
           "CREATE (d)-[:DIVERGES_FROM {lines: row.lines, gitCommit: row.c, gitDate: row.d}]->(c)",
           "DIVERGES_FROM", batch=200)
    with_item = len({i for i, _, _ in items})
    with_java = len({i for i, _, _ in java})
    print(f"divergences: {len(divs)} nodes, {with_item} with a resolved item, "
          f"{with_java} citing a resolved Lucene type", file=sys.stderr)


if __name__ == "__main__":
    main()
