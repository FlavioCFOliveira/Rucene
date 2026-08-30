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

Uso:
    python3 tools/kg/port_coverage_kg.py rucene \\
        --lucene-root /tmp/lucene1050 \\
        --commit <sha> --date <YYYY-MM-DD>
"""

import argparse
import datetime
import json
import os
import re
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from extract_lucene_kg import clean_for_brace_count  # noqa: E402
from load_rucene_kg import esc, read, rmp, run_unwind  # noqa: E402

CORE_PREFIX = "org.apache.lucene"
SCOPE_RULE_IN = "lucene-core-top-level"
SCOPE_RULE_NESTED = "nested-in-enclosing-type"
SCOPE_RULE_OUT = "not-a-lucene-core-type"

DECL_RE = re.compile(
    r"^\s*(?:public\s+|protected\s+|private\s+|abstract\s+|final\s+|static\s+"
    r"|strictfp\s+|sealed\s+|non-sealed\s+)*"
    r"(class|interface|enum|record|@interface)\s+([A-Za-z_$][A-Za-z0-9_$]*)",
    re.M,
)
PKG_RE = re.compile(r"\n\s*package\s+([a-zA-Z0-9_.]+)\s*;")
IMPORT_RE = re.compile(r"\n\s*import\s+(?:static\s+)?([a-zA-Z0-9_.$]+)\s*;")
WORD_RE = re.compile(r"\b[A-Z][A-Za-z0-9_$]*\b")


# ---------------------------------------------------------------------------
# Leitura do codigo-fonte de referencia
# ---------------------------------------------------------------------------


def scan_lucene(lucene_root: str):
    """Devolve (top_level, deps).

    top_level: qualifiedName -> ficheiro relativo
    deps:      lista de (from, to) entre tipos de topo do `lucene/core`
    """
    roots = [
        os.path.join(lucene_root, "lucene/core/src/java"),
        os.path.join(lucene_root, "lucene/core/src/java21"),
    ]
    files = []
    for r in roots:
        for dp, _dirs, fns in os.walk(r):
            for fn in fns:
                if fn.endswith(".java") and fn != "module-info.java":
                    files.append(os.path.join(dp, fn))
    files.sort()

    info = {}
    pkg_types = {}
    for f in files:
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
        rel = os.path.relpath(f, lucene_root)
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
            if not imp.startswith(CORE_PREFIX):
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


def phase_scope(roadmap, top_level, commit, date, batch_size):
    print("phase: scope", file=sys.stderr)
    nodes = read(
        roadmap,
        "MATCH (c:Class) RETURN c.qualifiedName AS qn, c.file AS file, "
        "c.package AS pkg",
    )
    rows_in, rows_nested, rows_out = [], [], []
    for n in nodes:
        qn = n["qn"]
        if qn and qn in top_level:
            rows_in.append({"qn": qn, "file": top_level[qn]})
        elif qn and qn.startswith(CORE_PREFIX):
            rows_nested.append({"qn": qn})
        elif (n["file"] or "").startswith("lucene/core/"):
            rows_nested.append({"qn": qn})
        else:
            rows_out.append({"qn": qn})
    stamp = f"c.gitCommit = {esc(commit)}, c.gitDate = {esc(date)}"
    run_unwind(
        "update",
        roadmap,
        rows_in,
        "MATCH (c:Class {qualifiedName:row.qn}) "
        f"SET c.portScope = 'in', c.portScopeRule = '{SCOPE_RULE_IN}', "
        f"c.file = coalesce(c.file, row.file), {stamp}",
        batch_size,
        "portScope = in",
    )
    run_unwind(
        "update",
        roadmap,
        [r for r in rows_nested if r["qn"]],
        "MATCH (c:Class {qualifiedName:row.qn}) "
        f"SET c.portScope = 'nested', c.portScopeRule = '{SCOPE_RULE_NESTED}', "
        f"{stamp}",
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

    # portState: tem, ou nao, um porte registado
    ported = read(
        roadmap,
        "MATCH (x)-[:PORTS]->(c:Class) WHERE c.portScope = 'in' "
        "RETURN DISTINCT c.qualifiedName AS qn",
    )
    ported_set = {r["qn"] for r in ported}
    run_unwind(
        "update",
        roadmap,
        [{"qn": qn} for qn in sorted(ported_set)],
        f"MATCH (c:Class {{qualifiedName:row.qn}}) SET c.portState = 'ported', {stamp}",
        batch_size,
        "portState = ported",
    )
    run_unwind(
        "update",
        roadmap,
        [{"qn": r["qn"]} for r in rows_in if r["qn"] not in ported_set],
        f"MATCH (c:Class {{qualifiedName:row.qn}}) SET c.portState = 'unported', "
        f"{stamp}",
        batch_size,
        "portState = unported",
    )


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
    by_name = {}
    for t in survey["types"]:
        if t["scope"] != "crate" or not t["file"].startswith("src/"):
            continue
        by_name.setdefault(t["name"], []).append(t)

    # Recomeca do zero: um tipo que entretanto passou a ter um `PORTS` curado
    # deixa de ser candidato, e a aresta antiga tem de desaparecer com ele.
    rmp(
        "delete",
        roadmap,
        "MATCH ()-[r:PORTS_CANDIDATE]->() DELETE r",
        "reset candidates",
    )
    unported = read(
        roadmap,
        "MATCH (c:Class) WHERE c.portScope = 'in' AND c.portState <> 'ported' "
        "RETURN c.qualifiedName AS qn, c.name AS name",
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
        cand = by_name.get(r["name"], [])
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
    summary = (
        "Every top-level type declared by a file of the Apache Lucene Core 10.5.0 "
        f"module lucene/core ({len(top_level)} types, src/java and src/java21) is "
        "marked portScope='in' and forms the denominator of port coverage. Nested "
        "types are marked portScope='nested' and are excluded, because they are "
        "ported with the type that encloses them, not independently. Anything "
        "else carrying a Java label is portScope='out'. portState splits the "
        "in-scope set three ways: 'ported' (a curated PORTS edge), 'candidate' "
        "(no PORTS edge, but exactly one Rucene type carries the same simple "
        "name, recorded as PORTS_CANDIDATE) and 'unported'."
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
        "port unit, so it would inflate both numerator and denominator with noise). "
        "Excluding src/java21 (rejected: those types back MMapDirectory, which the "
        "port needs; excluding them would be an unapproved scope decision). "
        "Restricting the scope to the packages already touched (rejected: it would "
        "make coverage look complete while the port is not)."
    )
    evidence = (
        f"tools/kg/port_coverage_kg.py scans {len(top_level)} top-level types from "
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
    ap.add_argument("--commit", required=True)
    ap.add_argument("--date", required=True)
    ap.add_argument("--survey", help="JSON de extract_rucene_kg.py (fase candidates)")
    ap.add_argument("--batch-size", type=int, default=200)
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

    top_level, deps = scan_lucene(args.lucene_root)
    print(
        f"lucene: top-level types={len(top_level)} type-deps={len(deps)}",
        file=sys.stderr,
    )
    if args.phase in ("all", "scope"):
        phase_scope(args.roadmap, top_level, args.commit, args.date, args.batch_size)
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
