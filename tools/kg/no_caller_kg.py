#!/usr/bin/env python3
"""Ported `pub` items no production code reaches, and who calls their Lucene
counterpart.

The last link of the chain `knowledge-model.md` documents under *Who reaches
a ported item*:

* `rust_refs_kg.py` writes the Rust `REFERENCES` edges (rust-analyzer SCIP);
* `lucene_calls_kg.py` writes the Lucene `CALLS` / `OVERRIDES` edges (the
  10.5.0 bytecode);
* this tool writes `PORTS_MEMBER` -- which Lucene member a Rust fn is the
  port of -- and runs the recorded queries.

Phases
------
``members``
    Replaces every ``PORTS_MEMBER`` edge: ``RustFn`` -> ``Method``, by the same
    name mapping `port_evidence_kg.measure_depth` measures depth with (a Java
    member ``fooBar`` is ported as ``foo_bar``, ``bar`` for a ``getBar``/
    ``isBar``/``hasBar``/``setBar``, ``new`` for a constructor). ``via`` says
    where the Java member was looked for:

    * ``declared`` -- a member-level doc-comment claim on the fn itself
      (``PORTS {claimLevel:'member', declaredMember}``);
    * ``owner`` -- the class the fn's declaring Rust type ``PORTS``;
    * ``inherited`` -- an ancestor of that class (``EXTENDS``/``IMPLEMENTS``),
      when the class itself has no such member;
    * ``free-fn`` -- for a free fn, the classes ported by the types of its own
      file (Lucene's static utilities become module functions).
``report``
    Runs `QUERY_FNS` and `QUERY_TYPES` -- the queries recorded in
    ``knowledge-model.md`` -- writes every row to
    ``<kg-target>/no_caller/list_<commit>.jsonl`` and prints the count table by
    subsystem and class, which `QUERY_BY_SUBSYSTEM` also gives in Cypher.

The classes are a triage aid, not a verdict: the name mapping is evidence,
not proof (one Rust fn can discharge several Java members, a faithful port can
rename), and a reference can hide behind trait dispatch on either side.
"""
import argparse
import json
import re
import subprocess
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from kgio import client, counters_of, esc, kg_target, read, read_dicts  # noqa: E402
from port_evidence_kg import rust_candidates  # noqa: E402

EVIDENCE = "name-mapping"
TYPE_LABELS = ("RustStruct", "RustEnum", "RustTrait", "RustAlias")


def acronym_snake(name: str) -> str:
    """`getUTF8Length` -> `get_utf8_length` (an acronym stays one word)."""
    s = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", name)
    s = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s)
    return s.lower()


def candidates(name: str, kind: str):
    """Rust fn names a Lucene member could have been ported to."""
    if kind == "constructor":
        return {"new"}
    out = set(rust_candidates(name))
    a = acronym_snake(name)
    out.add(a)
    for prefix in ("get_", "set_", "is_", "has_"):
        if a.startswith(prefix):
            out.add(a[len(prefix):])
    if kind in ("field", "enumConstant"):
        out = {acronym_snake(name), name.lower()}
    return out


def map_members(fns, owner_ports, file_ports, fn_ports, members, ancestors):
    """`[(fnQn, methodQn, via)]`.

    `fns`: `(qn, name, file, ownerTypeKey-or-None)`; `owner_ports`:
    `{typeKey: [classQn]}`; `file_ports`: `{file: [classQn]}`; `fn_ports`:
    `{fnQn: [(classQn, claimLevel, declaredMember)]}`; `members`:
    `{classQn: [(qn, name, kind)]}`; `ancestors`: `{classQn: [classQn]}`.
    """
    index = {}
    for cls, ms in members.items():
        by = defaultdict(list)
        for qn, name, kind in ms:
            for c in candidates(name, kind):
                by[c].append((qn, name))
        index[cls] = by

    out = []
    for qn, name, file, owner in fns:
        hits = []
        for cls, level, declared in fn_ports.get(qn, []):
            if level == "member" and declared:
                hits += [(m, "declared") for m, n, _k in members.get(cls, []) if n == declared]
        if not hits:
            classes = owner_ports.get(owner, []) if owner else file_ports.get(file, [])
            via = "owner" if owner else "free-fn"
            for cls in classes:
                hits += [(m, via) for m, _n in index.get(cls, {}).get(name, [])]
            if not hits and owner:
                for cls in classes:
                    for anc in ancestors.get(cls, []):
                        hits += [(m, "inherited") for m, _n in index.get(anc, {}).get(name, [])]
        seen = set()
        for m, via in hits:
            if m not in seen:
                seen.add(m)
                out.append((qn, m, via))
    return out


def lucene_ancestors(edges):
    parents = defaultdict(set)
    for a, b in edges:
        parents[a].add(b)
    out = {}
    for c in list(parents):
        seen, todo = [], list(parents[c])
        while todo:
            p = todo.pop(0)
            if p in seen or p == c:
                continue
            seen.append(p)
            todo.extend(parents.get(p, ()))
        out[c] = seen
    return out


def phase_members(roadmap, commit, date):
    owner_ports = defaultdict(list)
    file_ports = defaultdict(list)
    for label in TYPE_LABELS:
        for name, file, cls, level in read(
                roadmap, f"MATCH (t:{label})-[e:PORTS]->(c:Class) "
                         "RETURN t.name, t.file, c.qualifiedName, e.claimLevel"):
            owner_ports[(label, name, file)].append(cls)
            if level != "member":
                file_ports[file].append(cls)
    fn_ports = defaultdict(list)
    for qn, cls, level, declared in read(
            roadmap, "MATCH (f:RustFn)-[e:PORTS]->(c:Class) "
                     "RETURN f.qualifiedName, c.qualifiedName, e.claimLevel, e.declaredMember"):
        fn_ports[qn].append((cls, level, declared))
    owner_of = {}
    for label in TYPE_LABELS:
        for name, file, fqn in read(roadmap, f"MATCH (t:{label})-[:DECLARES]->(f:RustFn) "
                                             "RETURN t.name, t.file, f.qualifiedName"):
            owner_of[fqn] = (label, name, file)
    fns = [(qn, name, file, owner_of.get(qn))
           for qn, name, file in read(roadmap, "MATCH (f:RustFn) WHERE f.scope = 'crate' "
                                               "AND f.kind <> 'test' "
                                               "RETURN f.qualifiedName, f.name, f.file")]
    members = defaultdict(list)
    for qn, parent, name, kind in read(roadmap, "MATCH (m:Method) RETURN m.qualifiedName, "
                                                "m.parentQualifiedName, m.name, m.kind"):
        members[parent].append((qn, name, kind))
    edges = read(roadmap, "MATCH (a:Class)-[:EXTENDS]->(b:Class) RETURN a.qualifiedName, b.qualifiedName")
    edges += read(roadmap, "MATCH (a:Class)-[:IMPLEMENTS]->(b:Class) RETURN a.qualifiedName, b.qualifiedName")
    rows = map_members(fns, owner_ports, file_ports, fn_ports, members, lucene_ancestors(edges))

    total = 0
    while True:
        c = counters_of(client(roadmap, "MATCH ()-[e:PORTS_MEMBER]->() WITH e LIMIT 5000 DELETE e",
                               label="delete PORTS_MEMBER"))
        if not c.get("relationshipsDeleted"):
            break
        total += c["relationshipsDeleted"]
    print(f"  retired PORTS_MEMBER: {total}", file=sys.stderr)
    created = 0
    for i in range(0, len(rows), 300):
        chunk = rows[i:i + 300]
        maps = ", ".join("{" + f"f:{esc(f)}, m:{esc(m)}, v:{esc(v)}" + "}" for f, m, v in chunk)
        c = counters_of(client(
            roadmap,
            f"UNWIND [{maps}] AS row MATCH (f:RustFn {{qualifiedName: row.f}}) "
            "MATCH (m:Method {qualifiedName: row.m}) "
            f"CREATE (f)-[:PORTS_MEMBER {{evidence: '{EVIDENCE}', via: row.v, "
            f"gitCommit: {esc(commit)}, gitDate: {esc(date)}}}]->(m)",
            label="PORTS_MEMBER"))
        if c.get("nodesCreated"):
            raise SystemExit(f"PORTS_MEMBER created nodes: {c}")
        created += c.get("relationshipsCreated", 0)
    got = read(roadmap, "MATCH ()-[e:PORTS_MEMBER]->() RETURN count(e)")[0][0]
    print(f"  PORTS_MEMBER: {created} created of {len(rows)}; graph holds {got}; "
          f"{dict(Counter(v for _f, _m, v in rows))}", file=sys.stderr)
    if created != len(rows) or got != len(rows):
        raise SystemExit("PORTS_MEMBER in the graph differ from the mapping")


# ---------------------------------------------------------------------------
# The recorded queries (copied verbatim into knowledge-model.md)
# ---------------------------------------------------------------------------

# A Lucene member is public API when it is public or protected -- or declared
# by an interface and not private -- and its class is published: a top-level
# public type, or a nested type whose `javaPublished` is true.
QUERY_FNS = """
MATCH (x:RustFn)
WHERE x.visibility = 'pub' AND x.scope = 'crate' AND x.kind <> 'test'
  AND x.file STARTS WITH 'src/'
OPTIONAL MATCH (x)<-[e:REFERENCES]-() WHERE e.evidence = 'rust-analyzer-scip' AND e.prodRefs > 0
WITH x, count(e) AS prodReferrers
WHERE prodReferrers = 0
OPTIONAL MATCH (x)<-[lx:REFERENCES]-() WHERE lx.evidence = 'macro-body-lexical' AND lx.prodRefs > 0
WITH x, count(lx) AS macroBodyMentions
OPTIONAL MATCH (x)-[:PORTS_MEMBER]->(m:Method)<-[:DECLARES]-(mc:Class)
OPTIONAL MATCH (m)<-[:CALLS]-(c1)
OPTIONAL MATCH (m)-[:OVERRIDES]->(:Method)<-[:CALLS]-(c2)
WITH x, macroBodyMentions, m, mc, collect(DISTINCT c1) + collect(DISTINCT c2) AS cs
OPTIONAL MATCH (m)-[ov:OVERRIDES]->(:Method)
WITH x, macroBodyMentions, m, mc, cs, count(ov) AS overrides
WITH x, macroBodyMentions, collect(CASE WHEN m IS NULL THEN null ELSE {
       member: m.qualifiedName,
       api: (m.modifiers CONTAINS 'public' OR m.modifiers CONTAINS 'protected'
             OR (mc.kind = 'interface' AND NOT m.modifiers CONTAINS 'private'))
            AND (mc.javaPublished = true
                 OR (mc.javaPublished IS NULL AND mc.javaVisibility = 'public')),
       inherited: overrides > 0 OR m.overridesExternal IS NOT NULL,
       callers: [c IN cs | c.qualifiedName],
       callerClasses: [c IN cs | coalesce(c.parentQualifiedName, c.qualifiedName)]
     } END) AS cps
OPTIONAL MATCH (x)-[:PORTS]->(pc:Class)
WITH x, macroBodyMentions, cps, collect(pc.qualifiedName) AS portsClass
WITH x, macroBodyMentions, cps, portsClass,
     reduce(a = [], cp IN cps | a + cp.callers) AS callers,
     reduce(a = [], cp IN cps | a + cp.callerClasses) AS callerClasses
RETURN x.qualifiedName AS item, 'RustFn' AS label, x.file AS file,
       split(x.qualifiedName, '::')[1] AS subsystem,
       [cp IN cps | cp.member] + portsClass AS counterparts,
       callers, callerClasses, macroBodyMentions,
       CASE
         WHEN size(cps) = 0 AND size(portsClass) = 0 THEN 'D?'
         WHEN size(callers) > 0 THEN 'W?'
         WHEN any(cp IN cps WHERE cp.api) OR size(portsClass) > 0 THEN 'P?'
         ELSE 'D?'
       END AS class,
       CASE
         WHEN size(cps) = 0 AND size(portsClass) = 0 THEN 'no-counterpart'
         WHEN size(callers) > 0 THEN 'lucene-calls-counterpart'
         WHEN any(cp IN cps WHERE cp.api AND cp.inherited) THEN 'inherited-api'
         WHEN any(cp IN cps WHERE cp.api) OR size(portsClass) > 0 THEN 'public-api'
         ELSE 'counterpart-not-api-and-uncalled'
       END AS reason
"""

# A type's counterpart is the class it PORTS; its Lucene callers are the other
# classes that DEPENDS_ON that class (imports plus same-package references).
QUERY_TYPES = """
MATCH (x:{label})
WHERE x.visibility = 'pub' AND x.scope = 'crate' AND x.file STARTS WITH 'src/'
OPTIONAL MATCH (x)<-[e:REFERENCES]-(r)
  WHERE e.evidence = 'rust-analyzer-scip' AND e.prodRefs > 0 AND NOT (x)-[:DECLARES]->(r)
WITH x, count(e) AS prodReferrers
WHERE prodReferrers = 0
OPTIONAL MATCH (x)<-[lx:REFERENCES]-() WHERE lx.evidence = 'macro-body-lexical' AND lx.prodRefs > 0
WITH x, count(lx) AS macroBodyMentions
OPTIONAL MATCH (x)-[:PORTS]->(pc:Class)
OPTIONAL MATCH (pc)<-[:DEPENDS_ON]-(dc:Class) WHERE dc <> pc
WITH x, macroBodyMentions, pc, collect(DISTINCT dc.qualifiedName) AS deps
WITH x, macroBodyMentions, collect(CASE WHEN pc IS NULL THEN null ELSE {
       cls: pc.qualifiedName,
       api: pc.javaPublished = true
            OR (pc.javaPublished IS NULL AND pc.javaVisibility = 'public'),
       callers: deps } END) AS cps
WITH x, macroBodyMentions, cps, reduce(a = [], cp IN cps | a + cp.callers) AS callers
RETURN x.qualifiedName AS item, '{label}' AS label, x.file AS file,
       split(x.qualifiedName, '::')[1] AS subsystem,
       [cp IN cps | cp.cls] AS counterparts,
       callers, callers AS callerClasses, macroBodyMentions,
       CASE
         WHEN size(cps) = 0 THEN 'D?'
         WHEN size(callers) > 0 THEN 'W?'
         WHEN any(cp IN cps WHERE cp.api) THEN 'P?'
         ELSE 'D?'
       END AS class,
       CASE
         WHEN size(cps) = 0 THEN 'no-counterpart'
         WHEN size(callers) > 0 THEN 'lucene-depends-on-counterpart'
         WHEN any(cp IN cps WHERE cp.api) THEN 'public-api'
         ELSE 'counterpart-not-api-and-unused'
       END AS reason
"""

# The same list counted by subsystem and class: wrap either query.
QUERY_BY_SUBSYSTEM_SUFFIX = """
WITH subsystem, class, count(*) AS items
RETURN subsystem, class, items ORDER BY subsystem, class
"""


def by_subsystem(q: str) -> str:
    """`q` grouped by subsystem and class: its final RETURN becomes a WITH."""
    head, _, tail = q.rpartition("RETURN ")
    return head + "WITH " + tail + QUERY_BY_SUBSYSTEM_SUFFIX


def grouped_counts(roadmap):
    """`{(subsystem, class): items}` from the grouped variants, in Cypher."""
    out = Counter()
    for q in [QUERY_FNS] + [QUERY_TYPES.replace("{label}", lb) for lb in TYPE_LABELS]:
        for sub, cls, n in read(roadmap, by_subsystem(q)):
            out[(sub, cls)] += n
    return out


def phase_report(roadmap, commit, out: Path):
    t = time.time()
    rows = read_dicts(roadmap, QUERY_FNS)
    for label in TYPE_LABELS:
        rows += read_dicts(roadmap, QUERY_TYPES.replace("{label}", label))
    elapsed = time.time() - t
    # A W? whose Lucene callers sit only in classes the port has not ported is
    # "blocked": the link cannot exist until a caller is ported.
    ported = {r[0] for r in read(roadmap, "MATCH (c:Class) WHERE c.portState IN "
                                          "['ported', 'absorbed'] RETURN c.qualifiedName")}
    for r in rows:
        r["callerClasses"] = sorted(set(r["callerClasses"]))
        r["callers"] = sorted(set(r["callers"]))
        r["blocked"] = r["class"] == "W?" and not any(c in ported for c in r["callerClasses"])
        r["commit"] = commit
    rows.sort(key=lambda r: (r["subsystem"] or "", r["item"]))
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w") as fh:
        for r in rows:
            fh.write(json.dumps(r) + "\n")
    print(f"  {len(rows)} items in {elapsed:.1f}s -> {out}", file=sys.stderr)
    return rows


def table(rows):
    classes = ["W?", "W? blocked", "P?", "D?"]
    by = defaultdict(Counter)
    for r in rows:
        k = r["class"] + (" blocked" if r["blocked"] else "")
        by[r["subsystem"] or "(root)"][k] += 1
    lines = ["| subsystem | " + " | ".join(classes) + " | total |",
             "|---|" + "---:|" * (len(classes) + 1)]
    tot = Counter()
    for sub in sorted(by):
        c = by[sub]
        tot.update(c)
        lines.append(f"| {sub} | " + " | ".join(str(c[k]) for k in classes)
                     + f" | {sum(c.values())} |")
    lines.append("| **total** | " + " | ".join(str(tot[k]) for k in classes)
                 + f" | {sum(tot.values())} |")
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("roadmap")
    ap.add_argument("--phase", choices=["members", "report", "all"], default="all")
    ap.add_argument("--commit", help="provenance commit (default: HEAD of this checkout)")
    ap.add_argument("--out", help="the list (JSONL); default <kg-target>/no_caller/list_<commit>.jsonl")
    args = ap.parse_args()
    root = Path(__file__).resolve().parents[2]
    commit = subprocess.run(["git", "-C", str(root), "rev-parse", args.commit or "HEAD"],
                            capture_output=True, text=True, check=True).stdout.strip()
    date = subprocess.run(["git", "-C", str(root), "show", "-s", "--format=%cs", commit],
                          capture_output=True, text=True, check=True).stdout.strip()
    if args.phase in ("members", "all"):
        phase_members(args.roadmap, commit, date)
    if args.phase in ("report", "all"):
        out = Path(args.out) if args.out else kg_target() / "no_caller" / f"list_{commit[:7]}.jsonl"
        rows = phase_report(args.roadmap, commit, out)
        print(table(rows))
        mine = Counter((r["subsystem"], r["class"]) for r in rows)
        theirs = grouped_counts(args.roadmap)
        print(f"\ngrouped Cypher variant agrees with the list: {mine == theirs}")
        if mine != theirs:
            raise SystemExit(f"grouped variant differs: {mine - theirs} / {theirs - mine}")
        print("\nreasons: " + json.dumps(Counter(r["reason"] for r in rows)))
        print("named by a macro_rules! body (lexical): "
              + json.dumps(Counter(r["class"] for r in rows if r["macroBodyMentions"])))


if __name__ == "__main__":
    main()
