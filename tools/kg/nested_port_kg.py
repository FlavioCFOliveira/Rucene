#!/usr/bin/env python3
"""What became of Lucene's 908 nested types.

`portScope = 'nested'` marks the 908 inner types of `lucene/core` 10.5.0 and
keeps them out of the coverage denominator, on the rule
`nested-in-enclosing-type`: an inner type is ported with the type that encloses
it, not independently. That rule was never measured. `CLAUDE.md` 6.1 requires
coverage to be answerable and defensible, and an exclusion that rests on an
assumption is neither -- the more so because the port already contains
counter-examples, `TotalHits.Relation` and `Sorter.DocMap` among them, nested
types that needed a declared port of their own.

This tool measures it. Every nested type is put in exactly one of three states,
each with the evidence that put it there:

* **`ported`** -- something *asserts* that a Rucene item is this type: a `PORTS`
  edge, whose `evidence` says on what basis. Most are the crate's own doc
  comments, read by `port_evidence_kg.py`; the rest are hand verifications
  carried in `nested_port_verdicts.json`.
* **`absorbed`** -- no independent Rust item, and none is owed. Two grounds, and
  never a bare assumption:
  - *measured* -- the type is not **nameable** by any client (see below), so no
    contract of Lucene 10.5.0 mentions it and nothing can depend on it except
    through the type that encloses it, whose port exists;
  - *verified* -- it is nameable, but reading the crate shows where its work
    went and why that is faithful. Those cases are named one by one in
    `nested_port_verdicts.json`, with the file and line that show it.
* **`unported`** -- neither. Real work, listed with a destination.

**Nameable** is the decisive Java-side fact, and it is derived from the clone:

* *published* -- `public` or `protected` with every enclosing type likewise, so a
  downstream caller can write `Outer.Inner`. JLS 9.5: a member type of an
  interface or annotation is implicitly `public`, whatever the source says.
* *used across files* -- some other `lucene/core` file names it as
  `Outer.Inner`, which is what a package-private inner type used by its own
  package looks like.

A type that is neither cannot be named outside the file that declares it. That
is not a guess about the port; it is a fact about Lucene, and it is what makes
`absorbed` a measurement rather than the assumption it replaces.

Usage:
    python3 tools/kg/nested_port_kg.py rucene --lucene-root /tmp/lucene1050 \\
        --commit "$COMMIT" --date "$DATE" [--dry-run]
"""
import argparse
import json
import os
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from javatext import strip_java
from kgio import PER_ROW_SCAN_BATCH, read as kg_read, unwind, rmp, esc

VERDICTS = Path(__file__).parent / "nested_port_verdicts.json"

# A brace-depth scan of the clone. The declaration regex keeps the modifiers,
# which the loaders' own scanners discard, because visibility is the whole
# question here.
DECL = re.compile(
    r"[{}]"
    r"|(?P<mods>(?:(?:public|protected|private|static|final|abstract|sealed"
    r"|non-sealed|strictfp)\s+)*)"
    r"\b(?P<kw>class|interface|enum|record)\s+(?P<nm>[A-Za-z_$][\w$]*)"
    r"|@interface\s+(?P<nm2>[A-Za-z_$][\w$]*)")


def scan_java(lucene_root: Path):
    """Every type of `lucene/core`, with its modifiers, kind and nesting depth.

    Returns `(types, sources)`: a list of records keyed `(name, file)` -- the
    identity `audit_kg.py` also uses, and the one that agrees with the graph on
    all 908 -- and the stripped source of every file, for the reference census.
    """
    types, sources = [], {}
    core = lucene_root / "lucene/core"
    for base in ("src/java", "src/java21"):
        d = core / base
        if not d.is_dir():
            continue
        for dirpath, _dirs, names in os.walk(d):
            for name in sorted(names):
                if not name.endswith(".java"):
                    continue
                path = Path(dirpath, name)
                rel = str(path.relative_to(lucene_root))
                src = strip_java(path.read_text(encoding="utf-8", errors="replace"))
                sources[rel] = src
                depth, stack, pending = 0, [], None
                for tok in DECL.finditer(src):
                    t = tok.group(0)
                    if t == "{":
                        depth += 1
                        if pending:
                            stack.append((pending, depth))
                            pending = None
                        continue
                    if t == "}":
                        if stack and stack[-1][1] == depth:
                            (nm, kind, mods), _ = stack.pop()
                            types.append({
                                "name": nm, "file": rel, "kind": kind,
                                "mods": mods, "depth": len(stack),
                                "chain": [s[0][0] for s in stack]})
                        depth -= 1
                        continue
                    nm = tok.group("nm") or tok.group("nm2")
                    pending = (nm, tok.group("kw") or "annotation",
                               (tok.group("mods") or "").split())
    return types, sources


def nameability(types, sources):
    """`(name, file)` -> whether a client outside the declaring file can name it."""
    by_key = {(t["name"], t["file"]): t for t in types}
    top_by_name = {}
    for t in types:
        if t["depth"] == 0:
            top_by_name.setdefault((t["name"], t["file"]), t)

    def visibility(t):
        mods = set(t["mods"])
        for m in ("private", "protected", "public"):
            if m in mods:
                return m
        if t["chain"]:
            parent = by_key.get((t["chain"][-1], t["file"]))
            if parent is not None and parent["kind"] in ("interface", "annotation"):
                return "public"          # JLS 9.5
        return "package"

    # Which other files mention the `Outer.Inner` spelling.
    dotted = {rel: set(re.findall(r"[A-Za-z_$][\w$]*\.[A-Za-z_$][\w$]*", src))
              for rel, src in sources.items()}

    out = {}
    for t in types:
        if t["depth"] == 0:
            continue
        vis = visibility(t)
        published = vis in ("public", "protected")
        for i, encl in enumerate(t["chain"]):
            parent = by_key.get((encl, t["file"]))
            if parent is None:
                published = False
                break
            if visibility(parent) not in ("public", "protected"):
                published = False
                break
        pair = f"{t['chain'][-1]}.{t['name']}" if t["chain"] else t["name"]
        used = any(rel != t["file"] and pair in ds for rel, ds in dotted.items())
        out[(t["name"], t["file"])] = {
            "visibility": vis, "published": published, "usedAcrossFiles": used,
            "nameable": published or used}
    return out


def q(roadmap, cypher):
    return kg_read(roadmap, cypher)


def classify(roadmap, lucene_root):
    verdicts = json.loads(VERDICTS.read_text(encoding="utf-8"))
    types, sources = scan_java(lucene_root)
    named = nameability(types, sources)

    nested = {}
    for qn, name, file in q(roadmap, "MATCH (c:Class) WHERE c.portScope = 'nested' "
                                     "RETURN c.qualifiedName, c.name, c.file"):
        nested[qn] = {"qn": qn, "name": name, "file": file}

    ports = {}
    for qn, rn, rf, ev in q(roadmap,
                            "MATCH (t)-[p:PORTS]->(c:Class) WHERE c.portScope = 'nested' "
                            "RETURN c.qualifiedName, t.name, t.file, p.evidence"):
        ports.setdefault(qn, []).append((rn, rf, ev))
    # Where the enclosing type's port lives, for the `absorbed` host.
    host_ports = {}
    for qn, rn, rf in q(roadmap, "MATCH (t)-[:PORTS]->(c:Class) "
                                 "RETURN c.qualifiedName, t.name, t.file"):
        host_ports.setdefault(qn, []).append((rn, rf))

    def enclosing_chain(qn):
        head, _, rest = qn.partition("$")
        if not rest:
            return []
        segs = rest.split("$")
        return ["$".join([head] + segs[:k]) if k else head
                for k in range(len(segs) - 1, -1, -1)]

    rows, unresolved = [], []
    for qn, n in sorted(nested.items()):
        key = (n["name"], n["file"])
        info = named.get(key)
        if info is None:
            unresolved.append(qn)
            continue
        chain = enclosing_chain(qn)
        host = next((c for c in chain if c in host_ports), None)
        host_files = sorted({f for _n, f in host_ports.get(host, [])})
        row = {"qn": qn, "commit": None, "date": None,
               "visibility": info["visibility"], "published": info["published"],
               "usedAcrossFiles": info["usedAcrossFiles"],
               "host": host, "hostFiles": ", ".join(host_files),
               "destination": None}

        if qn in ports:
            rn, rf, ev = sorted(ports[qn])[0]
            row.update(state="ported", evidence=ev,
                       note=f"{rn} ({rf}) declares the port; {len(ports[qn])} edge(s).")
        elif qn in verdicts["curated"]:
            v = verdicts["curated"][qn]
            row.update(state="ported", evidence="curated",
                       note=f"{v['rustName']} ({v['rustFile']}): {v['evidence']}")
        elif qn in verdicts["absorbed"]:
            v = verdicts["absorbed"][qn]
            row.update(state="absorbed", evidence="verified-absorption",
                       note=f"absorbed by {v['host']} at {v['where']}: {v['why']}")
        elif qn in verdicts["unported"]:
            v = verdicts["unported"][qn]
            row.update(state="unported", evidence="verified-absent",
                       destination=v["destination"], note=v["reason"])
        elif not info["nameable"]:
            row.update(
                state="absorbed", evidence="not-nameable",
                note=(f"{info['visibility']} and named by no other lucene/core file, so no "
                      f"client can reach it except through {host}, whose port is "
                      f"{row['hostFiles'] or '(none)'}."))
        else:
            # Nameable, unclaimed and unverified: refuse to guess (`CLAUDE.md` 7).
            unresolved.append(qn)
            continue
        rows.append(row)
    return rows, unresolved


DECISION = "Nested types are excluded from the denominator by measurement, not by assumption"


def write_decision(roadmap, rows, commit, date):
    """Record why the 908 stay out of the denominator, now that it is measured."""
    import collections
    n = collections.Counter(r["state"] for r in rows)
    ev = collections.Counter(r["evidence"] for r in rows)
    summary = (
        f"The {len(rows)} nested types of lucene/core 10.5.0 keep portScope='nested' and stay "
        "out of the coverage denominator, but the exclusion no longer rests on the rule "
        "'a nested type is ported with the type that encloses it'. Each carries a measured "
        f"portState of its own: ported {n['ported']}, absorbed {n['absorbed']}, unported "
        f"{n['unported']}, with portStateEvidence saying on what basis and portStateNote saying "
        "where. Coverage of lucene/core's whole type surface is therefore answerable in one "
        f"query: {1196 + n['ported'] + n['absorbed']:,} of 2,104 accounted for, {n['unported']} "
        "named as missing work, each with a portDestination."
    )
    rationale = (
        "CLAUDE.md 6.1 requires coverage to be answerable and defensible, and a denominator "
        "that drops 908 types on an unverified rule is neither. The rule turned out to be "
        f"true for {n['ported'] + n['absorbed']} of them and false for {n['unported']}, so it "
        "could not have been left standing. The denominator itself does not change: a nested "
        "type is still not an independent port unit, and forcing one to be would make the "
        "ratio wrong in the other direction. What changes is that the exclusion is now a "
        "measurement with its evidence attached, and the gaps it was hiding are named."
    )
    alternatives = (
        "(a) Widen portScope to 'in' for all 2,104 types -- rejected: a private inner class "
        "that a faithful Rust port folds into three fields has no item for a PORTS edge to "
        "start at, so the numerator could never reach the denominator. "
        "(b) Add a fourth portScope for the 357 nested types a client can name -- rejected: "
        "the same answer already falls out of portState over portScope='nested', and a new "
        "scope value would break every coverage query for no new information. "
        "(c) Leave the exclusion as it was -- rejected by 6.1, and by the 27 gaps the "
        "measurement found. "
        "(d) Read all 908 by hand -- rejected on cost, not on principle: the 551 that no "
        "client can name carry no contract of their own, so reading each buys nothing the "
        "visibility measurement does not already give. Every nameable type still without a "
        "declared claim after the recogniser was widened -- 95 of the 357 -- was read "
        "individually, and so were the not-nameable ones whose enclosing type's port measures "
        "under 50% member coverage, which is where ten of the 27 gaps were found."
    )
    evidence = (
        "tools/kg/nested_port_kg.py re-derives each nested type's Java modifiers and its "
        "cross-file references from /tmp/lucene1050 at releases/lucene/10.5.0, and classifies "
        f"all {len(rows)}: {ev['doc-comment']} carry a PORTS edge from a claim the crate makes "
        f"in its own doc comments, {ev['curated']} a hand-verified edge, "
        f"{ev['not-nameable']} are absorbed because neither a published signature nor another "
        f"lucene/core file can name them, {ev['verified-absorption']} are absorbed with the "
        f"file and line that show where their work went, and {ev['verified-absent']} are "
        "unported with a destination. audit_kg.py fails if any nested type has no state, or if "
        "an unported one has no destination. Reading the crate's own short 'Outer.Inner' "
        "spelling, its quoted Java modifiers, its const and static items and its canonical "
        "'| Rucene | Apache Lucene Core 10.5.0 |' mapping table took the declared nested ports "
        "from 94 to 482 without admitting any new kind of inference."
    )
    rmp("create", roadmap, f"MERGE (d:Decision {{name:{esc(DECISION)}}})")
    rmp("update", roadmap,
        f"MATCH (d:Decision {{name:{esc(DECISION)}}}) SET d.kind = 'principle', "
        f"d.summary = {esc(summary)}, d.rationale = {esc(rationale)}, "
        f"d.alternatives = {esc(alternatives)}, d.evidence = {esc(evidence)}, "
        f"d.gitCommit = {esc(commit)}, d.gitDate = {esc(date)}")
    # The tool the decision rests on, so `IMPLEMENTED_IN` has a target. Other
    # loaders under `tools/kg/` are registered the same way, with `kind: "build"`.
    rmp("create", roadmap,
        f"MERGE (f:File {{path:'tools/kg/nested_port_kg.py'}})")
    rmp("update", roadmap,
        f"MATCH (f:File {{path:'tools/kg/nested_port_kg.py'}}) "
        f"SET f.name = 'nested_port_kg.py', f.kind = 'build', f.language = 'Python', "
        f"f.gitCommit = {esc(commit)}, f.gitDate = {esc(date)}")
    rmp("create", roadmap,
        f"MATCH (d:Decision {{name:{esc(DECISION)}}}) "
        f"MATCH (f:File {{path:'tools/kg/nested_port_kg.py'}}) "
        f"MERGE (d)-[:IMPLEMENTED_IN]->(f)")
    print("  Decision node written", file=sys.stderr)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("roadmap")
    ap.add_argument("--lucene-root", default="/tmp/lucene1050")
    ap.add_argument("--commit")
    ap.add_argument("--date")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    rows, unresolved = classify(args.roadmap, Path(args.lucene_root))
    import collections
    counts = collections.Counter(r["state"] for r in rows)
    ev = collections.Counter((r["state"], r["evidence"]) for r in rows)
    print(f"  nested types classified: {len(rows)}", file=sys.stderr)
    for k, n in sorted(counts.items()):
        print(f"    {k:<10} {n}", file=sys.stderr)
    for k, n in sorted(ev.items()):
        print(f"      {k[0]:<10} {k[1]:<22} {n}", file=sys.stderr)
    if unresolved:
        print(f"  UNCLASSIFIED (nameable, unclaimed, unverified): {len(unresolved)}",
              file=sys.stderr)
        for qn in unresolved[:20]:
            print("    -", qn, file=sys.stderr)
    if args.dry_run:
        return 1 if unresolved else 0

    for r in rows:
        r["commit"], r["date"] = args.commit, args.date

    # The hand-verified ports become real `PORTS` edges carrying
    # `evidence: "curated"`, so a coverage query reaches them the same way it
    # reaches a declared one, and `audit_kg.py` can check their Rust end exists.
    verdicts = json.loads(VERDICTS.read_text(encoding="utf-8"))
    edges = [{"name": d["rustName"], "file": d["rustFile"], "lucene": qn,
              "commit": args.commit, "date": args.date}
             for qn, d in sorted(verdicts["curated"].items())
             if any(r["qn"] == qn and r["evidence"] == "curated" for r in rows)]
    unwind(args.roadmap, "create", edges,
           "MATCH (t {name:row.name, file:row.file}) "
           "WITH t, row MATCH (c:Class {qualifiedName:row.lucene}) "
           "MERGE (t)-[:PORTS]->(c)", "curated PORTS edges", batch=PER_ROW_SCAN_BATCH)
    unwind(args.roadmap, "update", edges,
           "MATCH (t {name:row.name, file:row.file})-[e:PORTS]->"
           "(c:Class {qualifiedName:row.lucene}) "
           "SET e.evidence = 'curated', e.claimLevel = 'type', "
           "e.gitCommit = row.commit, e.gitDate = row.date",
           "curated PORTS evidence", batch=PER_ROW_SCAN_BATCH)

    unwind(args.roadmap, "update", rows,
           "MATCH (c:Class {qualifiedName:row.qn}) "
           "SET c.portState = row.state, c.portStateEvidence = row.evidence, "
           "c.portStateNote = row.note, c.nestedHost = row.host, "
           "c.nestedHostFiles = row.hostFiles, c.javaVisibility = row.visibility, "
           "c.javaPublished = row.published, c.javaUsedAcrossFiles = row.usedAcrossFiles, "
           "c.portDestination = row.destination, "
           "c.gitCommit = row.commit, c.gitDate = row.date",
           "nested portState", batch=150)
    write_decision(args.roadmap, rows, args.commit, args.date)
    return 1 if unresolved else 0


if __name__ == "__main__":
    sys.exit(main())
