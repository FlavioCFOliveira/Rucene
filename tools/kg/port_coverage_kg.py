#!/usr/bin/env python3
"""
Torna a cobertura do porte respondivel pelo grafo.

Escreve, sobre a estrutura ja levantada pelos outros extractores:

  scope       `portScope` e `portScopeRule` em cada tipo do Apache Lucene Core
              10.5.0, tornando explicito no grafo qual e a superficie que o
              porte tem de cobrir, e `portState` (`ported` / `unported`).
  deps        arestas `DEPENDS_ON` tipo->tipo do lado Lucene, derivadas dos
              `import` e das referencias dentro do mesmo pacote. Sao estas que
              permitem calcular alcancabilidade ("que peca por portar
              desbloqueia mais trabalho").
  candidates  liga, por `PORTS_CANDIDATE`, cada tipo Lucene ainda sem `PORTS`
              a um tipo Rucene do mesmo nome, quando existe exactamente um.
              Nao afirma que o porte esta feito - afirma que ha um candidato
              por confirmar, e poe `portState = 'candidate'`.
  tasks       espelha as tarefas `rmp` abertas e liga cada uma, por
              `REQUIRES_PORT`, aos tipos Lucene que o seu enunciado nomeia.
  components  preenche `Component.status`.
  decision    regista a regra de ambito como um no `Decision`, auditavel.

A regra de ambito e deliberadamente simples e mecanica, para ser defensavel e
reproduzivel: **todo o tipo de topo declarado por um ficheiro do modulo
`lucene/core` esta no ambito do porte**, porque e exactamente isso que o
`CLAUDE.md` (1 e 16.1) define como alvo - paridade funcional e compatibilidade
de indice com o Apache Lucene Core 10.5.0. Os tipos aninhados ficam marcados
`nested`: sao portados com o tipo que os envolve, nao de forma independente, e
por isso nao entram no denominador.

Desde as decisoes do utilizador de 2026-11-03 (sprint 9), de 2026-11-13
(sprint 12) e de 2026-11-14 (sprint 13), a mesma regra cobre todos os modulos que
`port_modules_kg.PORT_MODULES` nomeia para la de `lucene/core`
(`port_modules_kg.ADDED_MODULES`: `lucene/codecs`, `lucene/test-framework`,
randomizedtesting 2.8.1 `randomized-runner`, `lucene/analysis/common`,
`lucene/queries`, `lucene/queryparser` e `lucene/sandbox`), com o `classRule` de
cada um (`sprint-9-module-top-level`, `sprint-12-module-top-level`,
`sprint-13-module-top-level`, `sprint-13-listed-file-top-level`), e cada `Class`
leva o seu `module`. Um modulo com `files` (`lucene/sandbox`: so
`FuzzyLikeThisQuery.java`) so contribui os ficheiros listados
(`port_modules_kg.file_in_scope`). Para esses
modulos, uma reivindicacao so de membro (`claimLevel = 'member'`) nao torna o
tipo `ported`: e a regra de ciclo do sprint, que porta um membro antes do tipo
que o contem; os seus aninhados recebem um `portState` mecanico (`ported` com
`PORTS`, `skeleton` com `SKELETON_OF`, senao `unported` com o destino no crate
do modulo), porque `nested_port_kg.py` mede apenas os de `lucene/core`.

`portState = 'skeleton'` (qualquer modulo): o tipo nao tem `PORTS` ao nivel do
tipo, mas um item Rust declara-se o seu esqueleto (`SKELETON_OF`, escrito por
`port_evidence_kg.py`). A passagem `skeleton -> ported` so acontece com um
`PORTS`.

Uso:
    python3 tools/kg/port_coverage_kg.py rucene \\
        --lucene-root /tmp/lucene1050 --rt-root <randomizedtesting 2.8.1> \\
        --commit <sha> --date <YYYY-MM-DD>
"""

import argparse
import datetime
import json
import os
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from extract_lucene_kg import DECL_HEAD, clean_for_brace_count  # noqa: E402
from load_rucene_kg import esc, read, rmp, run_unwind  # noqa: E402
import port_modules_kg as pm  # noqa: E402

CORE_PREFIX = "org.apache.lucene"
JAVA_PREFIXES = pm.JAVA_PREFIXES
SCOPE_RULE_IN = "lucene-core-top-level"
SCOPE_RULE_NESTED = "nested-in-enclosing-type"
SCOPE_RULE_OUT = "not-a-lucene-core-type"

# `DECL_HEAD` admits annotations among the modifiers (JavaCC's generated parsers
# put `@SuppressWarnings(...)` on the same line as the declaration).
DECL_RE = re.compile(
    r"^\s*" + DECL_HEAD +
    r"(class|interface|enum|record|@interface)\s+([A-Za-z_$][A-Za-z0-9_$]*)",
    re.M,
)
PKG_RE = re.compile(r"(?:\A|\n)\s*package\s+([a-zA-Z0-9_.]+)\s*;")
IMPORT_RE = re.compile(r"\n\s*import\s+(?:static\s+)?([a-zA-Z0-9_.$]+)\s*;")
WORD_RE = re.compile(r"\b[A-Z][A-Za-z0-9_$]*\b")


# ---------------------------------------------------------------------------
# Leitura do codigo-fonte de referencia
# ---------------------------------------------------------------------------


def scan_lucene(lucene_root: str, rt_root=None):
    """Devolve (top_level, deps).

    top_level: qualifiedName -> ficheiro relativo a raiz do seu clone
    deps:      lista de (from, to) entre tipos de topo dos modulos do porte
               (`port_modules_kg.PORT_MODULES`), dentro e entre modulos
    """
    roots = pm.repo_roots(lucene_root, rt_root)
    files = []
    for m, base, r in pm.source_dirs(roots):
        for dp, _dirs, fns in os.walk(r):
            for fn in fns:
                if fn.endswith(".java") and fn != "module-info.java":
                    path = os.path.join(dp, fn)
                    # A module scoped to listed files contributes those alone.
                    if pm.file_in_scope(m["name"], os.path.relpath(path, base)):
                        files.append((str(base), path))
    files.sort()

    info = {}
    pkg_types = {}
    for base, f in files:
        txt = open(f, encoding="utf-8", errors="ignore").read()
        m = PKG_RE.search(txt)
        if not m:
            continue
        pkg = m.group(1)
        clean = clean_for_brace_count(txt)
        tops = []
        for dm in DECL_RE.finditer(txt):
            pre = clean[: dm.start()]
            if pre.count("{") - pre.count("}") != 0:
                continue
            tops.append(dm.group(2))
        rel = os.path.relpath(f, base)
        info[f] = (pkg, tops, txt, clean, rel)
        pkg_types.setdefault(pkg, set()).update(tops)

    top_level = {}
    for f, (pkg, tops, _t, _c, rel) in info.items():
        for t in tops:
            top_level[f"{pkg}.{t}"] = rel

    deps = set()
    for f, (pkg, tops, txt, clean, _rel) in info.items():
        targets = set()
        for imp in IMPORT_RE.findall(txt):
            if not imp.startswith(JAVA_PREFIXES):
                continue
            parts = imp.split(".")
            for k in range(len(parts), 0, -1):
                cand = ".".join(parts[:k])
                if cand in top_level:
                    targets.add(cand)
                    break
        # Referencias dentro do mesmo pacote nao exigem `import` em Java.
        words = set(WORD_RE.findall(clean))
        for t in pkg_types.get(pkg, ()):
            if t in words:
                targets.add(f"{pkg}.{t}")
        for top in tops:
            src = f"{pkg}.{top}"
            for tgt in targets:
                if tgt != src:
                    deps.add((src, tgt))
    return top_level, sorted(deps)


# ---------------------------------------------------------------------------
# Fases
# ---------------------------------------------------------------------------


# Classes in scope that have no faithful Rust form and owe no port: the
# version-controlled hand verdicts, each with its Lucene 10.5.0 and JDK 25
# sources (rmp #508). `phase_scope` writes them as `portState =
# 'notApplicable'` with the reason in `portStateNote`, so the mechanical pass
# that follows keeps them (`overridden_set`).
NA_VERDICTS = Path(__file__).resolve().parent / "not_applicable_verdicts.json"
NA_STATE = "notApplicable"
NA_EVIDENCE = "curated-not-applicable"
NA_REQUIRED = ("reason", "lucene", "jdk")


def load_not_applicable(path=NA_VERDICTS):
    """`{qualified name: verdict}` of the not-applicable verdict file."""
    return json.loads(Path(path).read_text(encoding="utf-8")).get("notApplicable", {})


def verdict_problems(verdicts):
    """`[(class, field)]` for every verdict missing a required justification:
    its reason, its Lucene sources or its JDK sources."""
    return [(qn, field) for qn, v in sorted(verdicts.items())
            for field in NA_REQUIRED if not v.get(field)]


def verdict_sources(v):
    """Every citation of a verdict, as the one string `portStateSources` holds."""
    return " | ".join([*v.get("lucene", ()), *v.get("jdk", ()), *v.get("rucene", ())])


def not_applicable_rows(verdicts, in_scope):
    """`(rows, unknown)`: the update rows for the verdicts naming an in-scope
    class, and the verdicts naming none."""
    rows, unknown = [], []
    for qn, v in sorted(verdicts.items()):
        if qn not in in_scope:
            unknown.append(qn)
            continue
        rows.append({"qn": qn, "note": v["reason"], "sources": verdict_sources(v)})
    return rows, unknown


def apply_not_applicable(roadmap, in_scope, stamp, batch_size, path=NA_VERDICTS):
    """Write the not-applicable verdicts, and return their classes.

    A class that was `notApplicable` and is no longer listed loses the state
    and its note, so the mechanical pass after this decides it again: a
    verdict cannot outlive its file entry. A verdict without its justification
    stops the sync; one naming no in-scope class is reported (and failed by the
    audit).
    """
    verdicts = load_not_applicable(path)
    bad = verdict_problems(verdicts)
    if bad:
        raise SystemExit(f"not-applicable verdicts without a justification: {bad}")
    rows, unknown = not_applicable_rows(verdicts, in_scope)
    for qn in unknown:
        print(f"  NOT-APPLICABLE VERDICT NAMES NO IN-SCOPE CLASS: {qn}", file=sys.stderr)
    listed = {r["qn"] for r in rows}
    stale = [{"qn": r["qn"]} for r in read(
        roadmap,
        f"MATCH (c:Class) WHERE c.portState = '{NA_STATE}' RETURN c.qualifiedName AS qn")
        if r["qn"] not in listed]
    run_unwind(
        "update", roadmap, stale,
        "MATCH (c:Class {qualifiedName:row.qn}) "
        "REMOVE c.portStateNote, c.portStateEvidence, c.portStateSources "
        f"SET c.portState = 'unported', {stamp}",
        batch_size, "notApplicable retired")
    run_unwind(
        "update", roadmap, rows,
        "MATCH (c:Class {qualifiedName:row.qn}) "
        f"SET c.portState = '{NA_STATE}', c.portStateNote = row.note, "
        f"c.portStateEvidence = '{NA_EVIDENCE}', c.portStateSources = row.sources, "
        f"{stamp}",
        batch_size, "portState = notApplicable")
    return listed


def overridden_set(roadmap):
    """Os tipos cujo `portState` foi corrigido a mao, identificados por
    `portStateNote`.

    Ver `knowledge-model.md`: `portStateNote` existe exactamente para registar
    porque e que o `portState` mecanico foi substituido. Enquanto a nota estiver
    la, a passagem automatica respeita-a.
    """
    rows = read(
        roadmap,
        "MATCH (c:Class) WHERE c.portScope = 'in' AND c.portStateNote IS NOT NULL "
        "RETURN c.qualifiedName AS qn",
    )
    return {r["qn"] for r in rows}


def phase_scope(roadmap, top_level, commit, date, batch_size, source_root=None):
    print("phase: scope", file=sys.stderr)
    nodes = read(
        roadmap,
        "MATCH (c:Class) RETURN c.qualifiedName AS qn, c.file AS file, "
        "c.package AS pkg",
    )
    rows_in, rows_nested, rows_out = [], [], []
    for n in nodes:
        qn = n["qn"]
        file = n["file"] or ""
        if qn and qn in top_level:
            module = pm.module_of_file(top_level[qn])
            rows_in.append({"qn": qn, "file": top_level[qn], "module": module,
                            "rule": SCOPE_RULE_IN if module == "lucene/core"
                            else pm.BY_NAME[module]["classRule"]})
        elif pm.module_of_file(file):
            rows_nested.append({"qn": qn, "module": pm.module_of_file(file),
                                "pkg": n["pkg"] or ""})
        elif qn and file:
            # The declaring file decides, not the package name. Rucene's own
            # Java test fixtures are declared in `org.apache.lucene.rucene.codec`
            # under `tests/fixtures/`, so a name-prefix rule counted all five of
            # them as Lucene inner types and inflated the `nested` bucket.
            rows_out.append({"qn": qn})
        elif qn and qn.startswith(CORE_PREFIX):
            # No file recorded: a stub some edge referenced by name. Keep it in
            # the nested bucket only while it is plausibly a Lucene inner type.
            rows_nested.append({"qn": qn, "module": "lucene/core", "pkg": n["pkg"] or ""})
        else:
            rows_out.append({"qn": qn})
    stamp = f"c.gitCommit = {esc(commit)}, c.gitDate = {esc(date)}"
    run_unwind(
        "update",
        roadmap,
        rows_in,
        "MATCH (c:Class {qualifiedName:row.qn}) "
        "SET c.portScope = 'in', c.portScopeRule = row.rule, c.module = row.module, "
        f"c.file = coalesce(c.file, row.file), {stamp}",
        batch_size,
        "portScope = in",
    )
    run_unwind(
        "update",
        roadmap,
        [{"qn": r["qn"], "module": r["module"]} for r in rows_nested if r["qn"]],
        "MATCH (c:Class {qualifiedName:row.qn}) "
        f"SET c.portScope = 'nested', c.portScopeRule = '{SCOPE_RULE_NESTED}', "
        f"c.module = row.module, {stamp}",
        batch_size,
        "portScope = nested",
    )
    run_unwind(
        "update",
        roadmap,
        [r for r in rows_out if r["qn"]],
        "MATCH (c:Class {qualifiedName:row.qn}) "
        f"SET c.portScope = 'out', c.portScopeRule = '{SCOPE_RULE_OUT}', {stamp}",
        batch_size,
        "portScope = out",
    )
    missing = sorted(set(top_level) - {r["qn"] for r in rows_in})
    for qn in missing:
        print(f"  MISSING Lucene type node: {qn}", file=sys.stderr)
    print(
        f"  in={len(rows_in)} nested={len(rows_nested)} out={len(rows_out)} "
        f"missing={len(missing)}",
        file=sys.stderr,
    )

    # portState: tem, ou nao, um porte registado.
    #
    # Um tipo que carregue `portStateNote` foi verificado a mao: a nota diz
    # porque e que a heuristica errou (tipo gerado por macro, declaracao com
    # parametro de tempo de vida, classe Java sem membros). Essa verificacao e
    # evidencia mais forte do que a regra mecanica, por isso a passagem
    # automatica nao lhe toca -- sem isto, cada sincronizacao apagava-a e o
    # grafo voltava a mentir sobre o que esta portado.
    apply_not_applicable(roadmap, {r["qn"] for r in rows_in}, stamp, batch_size)
    overridden = overridden_set(roadmap)

    # The sprint's cycle rule ports a member before the type that holds it, so
    # in a module beyond lucene/core a claim naming only a member is not a port
    # of the type: the member is recorded (`PORTS_EARLY`) and the type stays
    # missing.
    ported = read(
        roadmap,
        "MATCH (x)-[e:PORTS]->(c:Class) WHERE c.portScope = 'in' "
        "RETURN c.qualifiedName AS qn, c.module AS module, "
        "collect(coalesce(e.claimLevel, 'type')) AS levels",
    )
    ported_set = {r["qn"] for r in ported
                  if r["module"] == "lucene/core" or "type" in r["levels"]}
    run_unwind(
        "update",
        roadmap,
        [{"qn": qn} for qn in sorted(ported_set)],
        f"MATCH (c:Class {{qualifiedName:row.qn}}) SET c.portState = 'ported', {stamp}",
        batch_size,
        "portState = ported",
    )
    # A Rust skeleton without a port: the item exists, the port is not done.
    skeletons = skeleton_targets(roadmap)
    run_unwind(
        "update",
        roadmap,
        [
            {"qn": r["qn"]}
            for r in rows_in
            if r["qn"] not in ported_set and r["qn"] not in overridden
            and r["qn"] in skeletons
        ],
        f"MATCH (c:Class {{qualifiedName:row.qn}}) SET c.portState = 'skeleton', "
        f"{stamp}",
        batch_size,
        "portState = skeleton",
    )
    run_unwind(
        "update",
        roadmap,
        [
            {"qn": r["qn"]}
            for r in rows_in
            if r["qn"] not in ported_set and r["qn"] not in overridden
            and r["qn"] not in skeletons
        ],
        f"MATCH (c:Class {{qualifiedName:row.qn}}) SET c.portState = 'unported', "
        f"{stamp}",
        batch_size,
        "portState = unported",
    )
    if overridden:
        print(
            f"  portState preservado por verificacao manual: {len(overridden)}",
            file=sys.stderr,
        )
    phase_added_nested(roadmap, rows_nested, commit, date, batch_size, source_root)


def skeleton_targets(roadmap):
    """The `Class` nodes a Rust skeleton declares itself the skeleton of."""
    return {r["qn"] for r in read(
        roadmap,
        "MATCH ()-[:SKELETON_OF]->(c:Class) RETURN DISTINCT c.qualifiedName AS qn")}


def nested_state(qn, claimed, skeletons):
    """`(portState, portStateEvidence)` of a nested type of an added module,
    from the two facts that need no reading of the port: a type-level `PORTS`
    and a skeleton declaration."""
    if qn in claimed:
        return "ported", "doc-comment"
    if qn in skeletons:
        return "skeleton", "skeleton-doc-comment"
    return "unported", "no-port-claim"


# The Rust item labels a nested type's port can carry.
NESTED_ITEM_LABELS = ("RustStruct", "RustEnum", "RustTrait", "RustAlias")
HOST_FILE_EVIDENCE = "host-file-name"
HOST_MENTION_EVIDENCE = "host-comment-mention"
# Bare names that are also Rust's own prelude or standard types: a backticked
# `Option` in a host file's comment is far more likely Rust's than Java's.
STD_TYPE_NAMES = frozenset({"Option", "Result", "Box", "Vec", "String", "Iterator",
                            "Error", "Entry", "Ordering", "Default", "Drop", "Arc"})
_BACKTICK = re.compile(r"`([^`]+)(?:`|$)")


def enclosing_chain(qn):
    """`pkg.A$B$C` -> `["pkg.A$B", "pkg.A"]`, innermost first."""
    head, _, rest = qn.partition("$")
    if not rest:
        return []
    segs = rest.split("$")
    return ["$".join([head] + segs[:k]) for k in range(len(segs) - 1, -1, -1)]


def comment_mention(text, outer, inner):
    """`(line, comment)` of the first comment of `text` that names the nested
    type in backticks: `Outer.Inner`, `Outer$Inner`, or `Inner` alone (not when
    `Inner` is a Rust standard type name), possibly inside a quoted Java
    expression (`` `new Inner(...)` ``, `` `Inner.run()` ``). Consecutive
    comment lines are read as one block, since a quoted span wraps."""
    qual = re.compile(r"\b" + re.escape(outer) + r"[.$]" + re.escape(inner) + r"\b")
    bare = re.compile(r"(?<![\w.$])" + re.escape(inner) + r"\b")
    blocks, cur, start = [], [], 0
    for i, line in enumerate(text.splitlines(), 1):
        pos = line.find("//")
        if pos >= 0 and not line[:pos].strip():
            if not cur:
                start = i
            cur.append(line[pos:].lstrip("/!").strip())
            continue
        if cur:
            blocks.append((start, " ".join(cur)))
            cur = []
        if pos >= 0:                       # a trailing comment after code
            blocks.append((i, line[pos:].lstrip("/!").strip()))
    if cur:
        blocks.append((start, " ".join(cur)))
    for ln, block in blocks:
        for span in _BACKTICK.findall(block):
            if qual.search(span) or (inner not in STD_TYPE_NAMES and bare.search(span)):
                return ln, block[:160]
    return None


HOST_VARIANT_EVIDENCE = "host-enum-variant"


def enum_variant_fold(qn, siblings, files, text_of):
    """`(file, line, variant, base)` when a subclass hierarchy of nested types
    became one Rust enum in a host file.

    `FilterExpressionParser$AndNode extends InfixNode extends Node`: the port
    declares `enum Kind { Eos, Default, And, Or, ... }` and one `struct Node`.
    The nested type's name is a variant's name followed by the simple name of a
    sibling nested type (the base, `Node`), a host file declares an enum with
    that variant, and a comment of that file names the base in backticks.
    """
    outer, _, inner = qn.rpartition("$")
    for sib in siblings:
        so, _, base = sib.rpartition("$")
        if so != outer or base == inner or not inner.endswith(base):
            continue
        variant = inner[: -len(base)]
        if not variant:
            continue
        line_re = re.compile(r"^\s*" + re.escape(variant) + r"\s*(?:,|\(|\{|=|$)")
        for f in files:
            text = text_of(f)
            if "enum " not in text or f"`{base}`" not in text:
                continue
            for i, line in enumerate(text.splitlines(), 1):
                if line_re.match(line):
                    return f, i, variant, base
    return None


def classify_added_nested(rows, claimed, skeletons, host_files, rust_items, text_of,
                          verdicts):
    """`portState` rows for the nested types of the added modules.

    Strongest evidence first, and the first that applies wins:

    * `doc-comment` -- a type-level `PORTS` from the crate's own claim;
    * `skeleton-doc-comment` -- a skeleton declares itself the type's;
    * `host-file-name` -- **ported**: a Rust type of the nested type's own
      name is declared in a file that holds the port of a type enclosing it
      (`DirectPostingsFormat$DirectFields` is `struct DirectFields` in
      `direct_postings_format.rs`). The edge it rests on is written as a
      `PORTS` with this evidence, so the member triage reaches the item;
    * `host-comment-mention` -- **absorbed**: no such item, but a comment of a
      host file names the type in backticks, which is where the port says its
      work went (`// `new RandomlySplittingBKDWriter(...)``);
    * `verified-absorption` / `verified-absent` -- the hand verdicts of
      `nested_port_verdicts.json`, for a shape no rule reads;
    * `no-port-claim` -- **unported**, with the crate directory its port
      belongs in.

    `host_files` maps a type's qualified name to the files of its type-level
    ports; `rust_items` maps `(name, file)` to the item's label.
    Returns `(state rows, PORTS edge rows)`.
    """
    out, edges = [], []
    siblings = {r["qn"] for r in rows}
    for r in rows:
        qn = r["qn"]
        state, evidence = nested_state(qn, claimed, skeletons)
        note = host = None
        if state == "unported":
            chain = enclosing_chain(qn)
            host = next((h for h in chain if host_files.get(h)), None)
            files = sorted(host_files.get(host, ())) if host else []
            inner = qn.rsplit("$", 1)[-1]
            outer = qn.rsplit("$", 2)[-2].rsplit(".", 1)[-1]
            item = next(((inner, f) for f in files if (inner, f) in rust_items), None)
            if item:
                state, evidence = "ported", HOST_FILE_EVIDENCE
                note = (f"{rust_items[item]} {inner} ({item[1]}) is declared in the file "
                        f"that ports {host}.")
                edges.append({"name": inner, "file": item[1], "label": rust_items[item],
                              "qn": qn})
            else:
                hit = next(((f, comment_mention(text_of(f), outer, inner)) for f in files
                            if comment_mention(text_of(f), outer, inner)), None)
                fold = None if hit else enum_variant_fold(qn, siblings, files, text_of)
                if hit:
                    f, (ln, line) = hit
                    state, evidence = "absorbed", HOST_MENTION_EVIDENCE
                    note = f"named by a comment of the port of {host}, {f}:{ln}: {line}"
                elif fold:
                    f, ln, variant, base = fold
                    state, evidence = "absorbed", HOST_VARIANT_EVIDENCE
                    note = (f"the `{base}` subclass hierarchy is an enum in the port of {host}; "
                            f"this subclass is its variant `{variant}`, {f}:{ln}.")
                elif qn in verdicts.get("absorbed", {}):
                    v = verdicts["absorbed"][qn]
                    state, evidence = "absorbed", "verified-absorption"
                    note = f"absorbed by {v['host']} at {v['where']}: {v['why']}"
                elif qn in verdicts.get("unported", {}):
                    v = verdicts["unported"][qn]
                    evidence = "verified-absent"
                    note = v["reason"]
        out.append({"qn": qn, "state": state, "evidence": evidence, "note": note,
                    "host": host,
                    "dest": pm.crate_destination(r["module"], r["pkg"])
                    if state == "unported" else None})
    return out, edges


def phase_added_nested(roadmap, rows_nested, commit, date, batch_size, source_root=None):
    """`portState` of the nested types of the modules beyond lucene/core.

    `nested_port_kg.py` measures `lucene/core`'s nested types; these are
    classified by `classify_added_nested`, which reads the port's files under
    `source_root` (the commit's `git archive` export) for the comment rule.
    The `host-file-name` `PORTS` edges are owned here: all of them are deleted
    and the current set rewritten, so an edge cannot outlive its evidence.
    """
    import json as _json
    rows = [r for r in rows_nested if r["qn"] and r["module"] in pm.ADDED_MODULES]
    rmp("update", roadmap,
        "MATCH (c:Class)<-[e:PORTS]-(t) WHERE c.portScope = 'nested' "
        f"AND e.evidence = '{HOST_FILE_EVIDENCE}' DELETE e", "retire host-file-name PORTS")
    claimed = {r["qn"] for r in read(
        roadmap,
        "MATCH ()-[e:PORTS]->(c:Class) WHERE c.portScope = 'nested' "
        "AND coalesce(e.claimLevel, 'type') = 'type' RETURN DISTINCT c.qualifiedName AS qn")}
    skeletons = skeleton_targets(roadmap)
    host_files = {}
    for r in read(roadmap, "MATCH (t)-[e:PORTS]->(c:Class) "
                           "WHERE coalesce(e.claimLevel, 'type') = 'type' "
                           "RETURN c.qualifiedName AS qn, t.file AS file"):
        if r["file"]:
            host_files.setdefault(r["qn"], set()).add(r["file"])
    rust_items = {}
    for label in NESTED_ITEM_LABELS:
        for r in read(roadmap, f"MATCH (n:{label}) RETURN n.name AS name, n.file AS file"):
            rust_items.setdefault((r["name"], r["file"]), label)
    verdicts = _json.loads(
        (Path(__file__).resolve().parent / "nested_port_verdicts.json").read_text(encoding="utf-8"))
    cache = {}

    def text_of(f):
        if f not in cache:
            try:
                cache[f] = (Path(source_root or ".") / f).read_text(encoding="utf-8",
                                                                     errors="replace")
            except OSError:
                cache[f] = ""
        return cache[f]

    out, edges = classify_added_nested(rows, claimed, skeletons, host_files, rust_items,
                                       text_of, verdicts)
    for label in NESTED_ITEM_LABELS:
        mine = [dict(e, commit=commit, date=date) for e in edges if e["label"] == label]
        run_unwind(
            "create", roadmap, mine,
            f"MATCH (t:{label} {{name: row.name, file: row.file}}) "
            "MATCH (c:Class {qualifiedName: row.qn}) "
            f"CREATE (t)-[:PORTS {{evidence: '{HOST_FILE_EVIDENCE}', claimLevel: 'type', "
            "gitCommit: row.commit, gitDate: row.date}]->(c)",
            batch_size, f"host-file-name PORTS ({label})")
    for o in out:
        o["commit"], o["date"] = commit, date
    run_unwind(
        "update",
        roadmap,
        out,
        "MATCH (c:Class {qualifiedName:row.qn}) SET c.portState = row.state, "
        "c.portStateEvidence = row.evidence, c.portDestination = row.dest, "
        "c.portStateNote = row.note, c.nestedHost = row.host, "
        "c.gitCommit = row.commit, c.gitDate = row.date",
        batch_size,
        "added-module nested portState",
    )
    import collections as _c
    print("  added-module nested: " + ", ".join(
        f"{k[0]}/{k[1]} {v}" for k, v in sorted(_c.Counter(
            (o["state"], o["evidence"]) for o in out).items())), file=sys.stderr)


def phase_deps(roadmap, deps, commit, date, batch_size):
    print("phase: deps", file=sys.stderr)
    run_unwind(
        "create",
        roadmap,
        [{"a": a, "b": b} for a, b in deps],
        "MATCH (a:Class {qualifiedName:row.a}), (b:Class {qualifiedName:row.b}) "
        "MERGE (a)-[:DEPENDS_ON]->(b)",
        batch_size,
        "Class DEPENDS_ON Class",
    )
    rmp(
        "update",
        roadmap,
        "MATCH (:Class)-[r:DEPENDS_ON]->(:Class) "
        f"SET r.gitCommit = {esc(commit)}, r.gitDate = {esc(date)}",
        "dep provenance",
    )


def phase_candidates(roadmap, survey_path, commit, date, batch_size):
    """Um tipo Lucene sem `PORTS` cujo nome simples existe exactamente uma vez
    no crate e um **candidato** a porte, nao um porte confirmado.

    O `CLAUDE.md` 14.1 manda manter os nomes proximos dos do Lucene, por isso a
    coincidencia de nome e indicio forte - mas continua a ser indicio. Fica
    registada numa aresta propria, `PORTS_CANDIDATE`, para que o `PORTS`
    curado nao seja contaminado por inferencia (`CLAUDE.md` 7).
    """
    print("phase: candidates", file=sys.stderr)
    if not survey_path:
        print("  --survey is required for this phase", file=sys.stderr)
        sys.exit(2)
    survey = json.load(open(survey_path, encoding="utf-8"))
    # A candidate is looked for only in the crate that ports the type's module:
    # a `lucene/core` type in `src/`, any other in its own workspace crate.
    def crate_of(file):
        for m in pm.PORT_MODULES:
            if m["cratePath"] != "." and file.startswith(m["cratePath"] + "/src/"):
                return m["name"]
        return "lucene/core" if file.startswith("src/") else None

    by_name = {}
    for t in survey["types"]:
        module = crate_of(t["file"])
        if t["scope"] != "crate" or module is None:
            continue
        by_name.setdefault((module, t["name"]), []).append(t)

    # Recomeca do zero: um tipo que entretanto passou a ter um `PORTS` curado
    # deixa de ser candidato, e a aresta antiga tem de desaparecer com ele.
    rmp(
        "delete",
        roadmap,
        "MATCH ()-[r:PORTS_CANDIDATE]->() DELETE r",
        "reset candidates",
    )
    # A type with a skeleton is not a candidate: its Rust item is declared, and
    # the name match would only restate that the skeleton exists.
    unported = read(
        roadmap,
        "MATCH (c:Class) WHERE c.portScope = 'in' "
        "AND NOT c.portState IN ['ported', 'skeleton'] "
        "AND c.portStateNote IS NULL "
        "RETURN c.qualifiedName AS qn, c.name AS name, c.module AS module",
    )
    label_of = {
        "struct": "RustStruct",
        "union": "RustStruct",
        "trait": "RustTrait",
        "enum": "RustEnum",
        "alias": "RustAlias",
    }
    rows = []
    for r in unported:
        cand = by_name.get((r["module"], r["name"]), [])
        if len(cand) != 1:
            continue
        t = cand[0]
        rows.append(
            {
                "label": label_of[t["kind"]],
                "n": t["name"],
                "f": t["file"],
                "qn": r["qn"],
            }
        )
    for label in sorted({r["label"] for r in rows}):
        sel = [r for r in rows if r["label"] == label]
        run_unwind(
            "create",
            roadmap,
            [{"n": r["n"], "f": r["f"], "qn": r["qn"]} for r in sel],
            f"MATCH (t:{label} {{name:row.n, file:row.f}}), "
            "(c:Class {qualifiedName:row.qn}) MERGE (t)-[:PORTS_CANDIDATE]->(c)",
            batch_size,
            f"{label} PORTS_CANDIDATE Class",
        )
    rmp(
        "update",
        roadmap,
        "MATCH ()-[r:PORTS_CANDIDATE]->(:Class) SET r.evidence = 'exact-name-match', "
        f"r.gitCommit = {esc(commit)}, r.gitDate = {esc(date)}",
        "candidate provenance",
    )
    run_unwind(
        "update",
        roadmap,
        [{"qn": r["qn"]} for r in rows],
        "MATCH (c:Class {qualifiedName:row.qn}) SET c.portState = 'candidate', "
        f"c.gitCommit = {esc(commit)}, c.gitDate = {esc(date)}",
        batch_size,
        "portState = candidate",
    )
    print(f"  candidates: {len(rows)}", file=sys.stderr)


TASK_STATUSES = ("BACKLOG", "SPRINT", "DOING", "TESTING", "COMPLETED")


def _task_list(roadmap, status, since=None, until=None):
    cmd = ["rmp", "task", "list", "-r", roadmap, "-s", status, "-l", "100"]
    if since:
        cmd += ["--created-since", since]
    if until:
        cmd += ["--created-until", until]
    out = subprocess.run(cmd, capture_output=True, text=True)
    if out.returncode != 0:
        print(out.stderr, file=sys.stderr)
        sys.exit(1)
    return json.loads(out.stdout)


def fetch_tasks(roadmap):
    """Le **todas** as tarefas do `rmp`.

    `rmp task list` devolve no maximo 100 por invocacao e o filtro de data so e
    fiavel com a forma `YYYY-MM-DD` (com um instante RFC3339 completo devolve um
    subconjunto errado, verificado empiricamente). Percorre-se por isso o
    intervalo de datas do projecto, um dia e um estado de cada vez, o que
    mantem cada lote muito abaixo do limite.
    """
    seen = {}
    for status in TASK_STATUSES:
        for t in _task_list(roadmap, status):
            seen[t["id"]] = t
    if not seen:
        return []
    first = min(t["created_at"][:10] for t in seen.values())
    day = datetime.date.fromisoformat(first)
    today = datetime.date.today()
    while day <= today:
        iso = day.isoformat()
        # `--created-until` e exclusivo do proprio dia: a janela do dia D e
        # [D, D+1). Verificado empiricamente contra o `rmp`.
        nxt = (day + datetime.timedelta(days=1)).isoformat()
        for status in TASK_STATUSES:
            for t in _task_list(roadmap, status, since=iso, until=nxt):
                seen[t["id"]] = t
        day += datetime.timedelta(days=1)
    return sorted(seen.values(), key=lambda t: t["id"])


def phase_tasks(roadmap, top_level, commit, date, batch_size):
    print("phase: tasks", file=sys.stderr)
    tasks = fetch_tasks(roadmap)
    open_tasks = [t for t in tasks if t["status"] != "COMPLETED"]
    print(f"  tasks: {len(tasks)} total, {len(open_tasks)} open", file=sys.stderr)

    # Espelha **todas** as tarefas, nao so as fechadas: sem as abertas o grafo
    # nao consegue responder "o que falta fazer a seguir".
    known = {
        r["id"]: r["g"]
        for r in read(roadmap, "MATCH (t:Task) RETURN t.id AS id, t.gitCommit AS g")
    }
    run_unwind(
        "create",
        roadmap,
        [{"id": int(t["id"])} for t in tasks],
        "MERGE (t:Task {id:row.id})",
        batch_size,
        "Task create",
    )
    run_unwind(
        "update",
        roadmap,
        [
            {
                "id": int(t["id"]),
                "name": t["title"],
                "status": t["status"],
                "priority": str(t["priority"]),
            }
            for t in tasks
        ],
        "MATCH (t:Task {id:row.id}) SET t.name = row.name, t.status = row.status, "
        "t.priority = row.priority",
        batch_size,
        "Task update",
    )
    # A proveniencia de uma tarefa ja registada e o commit que a fechou: so se
    # carimbam as que ainda nao tinham nenhuma.
    run_unwind(
        "update",
        roadmap,
        [
            {"id": int(t["id"]), "gitCommit": commit, "gitDate": date}
            for t in tasks
            if not known.get(int(t["id"]))
        ],
        "MATCH (t:Task {id:row.id}) SET t.gitCommit = row.gitCommit, "
        "t.gitDate = row.gitDate",
        batch_size,
        "Task provenance (new nodes only)",
    )

    # Poda (#190): um no `Task` cuja tarefa o `rmp` ja nao tem e uma tarefa
    # inventada, que as consultas de "o que bloqueia o que" contavam como
    # aberta. A leitura vazia nao poda nada -- uma falha do `rmp` nao e prova
    # de que as tarefas desapareceram --, e um no que um commit diz ter fechado
    # (`CLOSES_TASK`) fica e e reportado, porque apaga-lo perderia esse facto.
    live_ids = {int(t["id"]) for t in tasks}
    stale = sorted(set(known) - live_ids) if live_ids else []
    closed_by_commit = {
        r[0] for r in read(
            roadmap,
            f"MATCH (:Commit)-[:CLOSES_TASK]->(t:Task) WHERE t.id IN {stale} RETURN DISTINCT t.id")
    } if stale else set()
    for tid in sorted(closed_by_commit):
        print(f"  KEPT stale Task #{tid}: a commit closes it (CLOSES_TASK)", file=sys.stderr)
    run_unwind(
        "delete",
        roadmap,
        [{"id": tid} for tid in stale if tid not in closed_by_commit],
        "MATCH (t:Task {id:row.id}) DETACH DELETE t",
        batch_size,
        "stale Task nodes pruned",
    )

    # nome simples -> qualifiedName, apenas quando nao e ambiguo
    by_simple = {}
    for qn in top_level:
        simple = qn.rsplit(".", 1)[1]
        by_simple.setdefault(simple, []).append(qn)
    unique = {k: v[0] for k, v in by_simple.items() if len(v) == 1 and len(k) >= 4}

    pairs = []
    for t in open_tasks:
        text = " ".join(
            str(t.get(k) or "")
            for k in (
                "title",
                "functional_requirements",
                "technical_requirements",
                "acceptance_criteria",
            )
        )
        words = set(WORD_RE.findall(text))
        for simple in words & set(unique):
            pairs.append({"id": int(t["id"]), "qn": unique[simple]})
    run_unwind(
        "create",
        roadmap,
        pairs,
        "MATCH (t:Task {id:row.id}), (c:Class {qualifiedName:row.qn}) "
        "MERGE (t)-[:REQUIRES_PORT]->(c)",
        batch_size,
        "Task REQUIRES_PORT Class",
    )
    rmp(
        "update",
        roadmap,
        "MATCH (:Task)-[r:REQUIRES_PORT]->(:Class) "
        f"SET r.gitCommit = {esc(commit)}, r.gitDate = {esc(date)}",
        "requires-port provenance",
    )

    # dependencias entre tarefas, tal como o `rmp` as regista
    all_ids = {int(t["id"]) for t in tasks}
    dep_pairs = [
        {"a": int(t["id"]), "b": int(d)}
        for t in open_tasks
        for d in (t.get("depends_on") or [])
        if int(d) in all_ids
    ]
    run_unwind(
        "create",
        roadmap,
        dep_pairs,
        "MATCH (a:Task {id:row.a}), (b:Task {id:row.b}) MERGE (a)-[:DEPENDS_ON]->(b)",
        batch_size,
        "Task DEPENDS_ON Task",
    )
    rmp(
        "update",
        roadmap,
        "MATCH (:Task)-[r:DEPENDS_ON]->(:Task) "
        f"SET r.gitCommit = {esc(commit)}, r.gitDate = {esc(date)}",
        "task-dep provenance",
    )


def phase_components(roadmap, commit, date, batch_size):
    """`Component.status` diz se a unidade esta confirmada no codigo.

    `ported`   confirmada no levantamento e com um `PORTS` registado;
    `present`  confirmada no levantamento, sem `PORTS` registado;
    `stale`    sem ficheiro confirmado no levantamento.
    """
    print("phase: components", file=sys.stderr)
    rows = read(
        roadmap,
        "MATCH (c:Component) RETURN c.name AS name, c.file AS file",
    )
    ported = {
        r["name"]
        for r in read(
            roadmap, "MATCH (c:Component)-[:PORTS]->() RETURN DISTINCT c.name AS name"
        )
    }
    payload = []
    for r in rows:
        if not r["file"]:
            status = "stale"
        elif r["name"] in ported:
            status = "ported"
        else:
            status = "present"
        payload.append(
            {"name": r["name"], "status": status, "gitCommit": commit, "gitDate": date}
        )
    run_unwind(
        "update",
        roadmap,
        payload,
        "MATCH (c:Component {name:row.name}) SET c.status = row.status, "
        "c.gitCommit = row.gitCommit, c.gitDate = row.gitDate",
        batch_size,
        "Component.status",
    )


def phase_decision(roadmap, top_level, commit, date):
    print("phase: decision", file=sys.stderr)
    name = "Port scope is every top-level type of lucene/core 10.5.0"
    rmp("create", roadmap, f"MERGE (d:Decision {{name:{esc(name)}}})", "decision")
    core = sum(1 for f in top_level.values() if pm.module_of_file(f) == "lucene/core")
    added = "; ".join(
        f"'{d['name']}' ("
        + ", ".join(f"{m} {sum(1 for f in top_level.values() if pm.module_of_file(f) == m)}"
                    for m in d["modules"]) + " types)"
        for d in pm.DECISIONS)
    summary = (
        "Every top-level type declared by a file of the Apache Lucene Core 10.5.0 "
        f"module lucene/core ({core} types, src/java and src/java21) is "
        "marked portScope='in' and forms the denominator of port coverage, with "
        "the modules each later scope decision adds -- see the decisions "
        f"{added}; Class.module tells them apart. Nested "
        "types are marked portScope='nested' and are excluded, because a nested "
        "type is not an independent port unit. That exclusion is measured, not "
        "assumed: tools/kg/nested_port_kg.py gives every one of the 908 a "
        "portState of its own -- see the decision 'Nested types are excluded "
        "from the denominator by measurement, not by assumption'. Anything "
        "else carrying a Java label is portScope='out'. portState splits the "
        "in-scope set four ways: 'ported' (a curated PORTS edge), 'candidate' "
        "(no PORTS edge, but exactly one Rucene type carries the same simple "
        "name, recorded as PORTS_CANDIDATE), 'skeleton' (no PORTS edge, but a "
        "Rust skeleton declares itself the type's, recorded as SKELETON_OF) and "
        "'unported'."
    )
    rationale = (
        "CLAUDE.md 1 states the reference source is Apache Lucene Core 10.5.0 and "
        "demands functional parity plus 100% index compatibility; 16.1 names "
        "lucene/core as the canonical source tree. The whole module is therefore "
        "the target, and any narrower scope would be a scope decision the project "
        "has not taken. The rule is mechanical, so tools/kg/port_coverage_kg.py "
        "reproduces the marking from a clean graph."
    )
    alternatives = (
        "Counting nested types too (rejected: a nested type is not an independent "
        "port unit -- a private inner class that a faithful Rust port folds into "
        "three fields has no item to point a PORTS edge at, so the numerator could "
        "never reach the denominator and the ratio would mislead in the other "
        "direction. What the rejection does not license is silence about them, "
        "which is why each of the 908 now carries its own measured portState). "
        "Excluding src/java21 (rejected: those types back MMapDirectory, which the "
        "port needs; excluding them would be an unapproved scope decision). "
        "Restricting the scope to the packages already touched (rejected: it would "
        "make coverage look complete while the port is not)."
    )
    evidence = (
        f"tools/kg/port_coverage_kg.py scans {core} top-level lucene/core types from "
        "/tmp/lucene1050 at tag releases/lucene/10.5.0; the same number is what the "
        "original Lucene survey recorded."
    )
    rmp(
        "update",
        roadmap,
        f"MATCH (d:Decision {{name:{esc(name)}}}) SET d.kind = 'principle', "
        f"d.summary = {esc(summary)}, d.rationale = {esc(rationale)}, "
        f"d.alternatives = {esc(alternatives)}, d.evidence = {esc(evidence)}, "
        f"d.gitCommit = {esc(commit)}, d.gitDate = {esc(date)}",
        "decision props",
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("roadmap")
    ap.add_argument("--lucene-root", default="/tmp/lucene1050")
    ap.add_argument("--rt-root", help="randomizedtesting 2.8.1 clone "
                    "(else $RANDOMIZEDTESTING_ROOT, else /tmp/randomizedtesting281)")
    ap.add_argument("--commit", required=True)
    ap.add_argument("--date", required=True)
    ap.add_argument("--survey", help="JSON de extract_rucene_kg.py (fase candidates)")
    ap.add_argument("--batch-size", type=int, default=200)
    ap.add_argument("--source-root", help="the commit's `git archive` export (scope phase: "
                    "the port's files the nested-type comment rule reads)")
    ap.add_argument(
        "--phase",
        default="all",
        choices=[
            "all",
            "scope",
            "deps",
            "candidates",
            "tasks",
            "components",
            "decision",
        ],
    )
    args = ap.parse_args()

    top_level, deps = scan_lucene(args.lucene_root, args.rt_root)
    print(
        f"lucene: top-level types={len(top_level)} type-deps={len(deps)}",
        file=sys.stderr,
    )
    if args.phase in ("all", "scope"):
        phase_scope(args.roadmap, top_level, args.commit, args.date, args.batch_size,
                    args.source_root)
    if args.phase in ("all", "deps"):
        phase_deps(args.roadmap, deps, args.commit, args.date, args.batch_size)
    if args.phase in ("all", "candidates"):
        phase_candidates(
            args.roadmap, args.survey, args.commit, args.date, args.batch_size
        )
    if args.phase in ("all", "tasks"):
        phase_tasks(args.roadmap, top_level, args.commit, args.date, args.batch_size)
    if args.phase in ("all", "components"):
        phase_components(args.roadmap, args.commit, args.date, args.batch_size)
    if args.phase in ("all", "decision"):
        phase_decision(args.roadmap, top_level, args.commit, args.date)


if __name__ == "__main__":
    main()
