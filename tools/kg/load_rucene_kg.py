#!/usr/bin/env python3
"""
Carrega o levantamento produzido por `extract_rucene_kg.py` no grafo de
conhecimento (`rmp graph`), em lotes `UNWIND`, seguindo as convencoes de
`load_members_unwind.py`.

Fases (executadas por omissao pela ordem indicada):

  repair  higiene dos nos legados, usando o levantamento como verdade terreno:
          colapsa `Struct`/`Trait`/`Enum`/`Interface`/`Class`(rucene)/`Test`
          nas etiquetas canonicas `RustStruct`/`RustTrait`/`RustEnum`/
          `RustAlias`/`RustFn`, resolve os `file` em falta, elimina os nos
          duplicados do lado Lucene (preservando as arestas `PORTS`) e o no
          sem etiqueta.
  nodes   cria/actualiza `File`, `RustStruct`, `RustTrait`, `RustEnum`,
          `RustAlias` e `RustFn`, e o no `File` de cada ficheiro `.rs`.
  edges   cria `CONTAINS`, `DECLARES`, `IMPLEMENTS` e `DEPENDS_ON`.
  audit   compara o grafo com o levantamento e reporta divergencias.

Cada no e cada aresta escritos levam `gitCommit` e `gitDate` do levantamento.

Uso:
    python3 tools/kg/load_rucene_kg.py rucene --survey <kg-target>/rucene_kg/survey.json
"""

import argparse
import json
import re
import sys
from pathlib import Path
from collections import defaultdict

from kgio import PER_ROW_SCAN_BATCH, client as kg_client, kg_target

TYPE_LABEL = {
    "struct": "RustStruct",
    "union": "RustStruct",
    "trait": "RustTrait",
    "enum": "RustEnum",
    "alias": "RustAlias",
}
ALL_TYPE_LABELS = ["RustStruct", "RustTrait", "RustEnum", "RustAlias"]
LEGACY_TYPE_LABELS = ["Struct", "Trait", "Enum", "Interface"]


def esc(v) -> str:
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    return "'" + str(v).replace("\\", "\\\\").replace("'", "\\'") + "'"


def rmp(mode: str, roadmap: str, query: str, label: str = "", attempts: int = 6):
    # One statement through the roadmap's graph server (`kgio.client`). `mode`
    # named a subcommand of the old CLI and now only labels a failure. A failure
    # exits non-zero rather than leaving the graph half-applied, a state no audit
    # can distinguish from real drift.
    return kg_client(roadmap, query, label=label or mode, attempts=attempts)


def read(roadmap: str, query: str):
    out = rmp("query", roadmap, query, "read")
    doc = json.loads(out)
    return [dict(zip(doc["columns"], row)) for row in doc["rows"]]


def batched(rows, size):
    for i in range(0, len(rows), size):
        yield rows[i : i + size]


def run_unwind(mode, roadmap, rows, body, batch_size, label):
    """Executa `UNWIND [<rows>] AS row <body>` em lotes."""
    total = len(rows)
    if not total:
        return
    n = 0
    for chunk in batched(rows, batch_size):
        maps = ", ".join(
            "{" + ", ".join(f"{k}:{esc(v)}" for k, v in r.items()) + "}" for r in chunk
        )
        rmp(mode, roadmap, f"UNWIND [{maps}] AS row {body}", label)
        n += len(chunk)
    print(f"  {label}: {n}", file=sys.stderr)


# ---------------------------------------------------------------------------
# Indices do levantamento
# ---------------------------------------------------------------------------


class Survey:
    def __init__(self, path):
        self.d = json.load(open(path, encoding="utf-8"))
        self.commit = self.d["commit"]
        self.date = self.d["date"]
        self.files = self.d["files"]
        self.types = self.d["types"]
        self.fns = self.d["fns"]
        self.impls = self.d["impls"]
        self.mods = self.d.get("mods", [])
        self.deps = self.d["deps"]
        self.by_key = {(t["name"], t["file"]): t for t in self.types}
        self.by_qn = {t["qualifiedName"]: t for t in self.types}
        self.by_name = defaultdict(list)
        for t in self.types:
            self.by_name[t["name"]].append(t)
        self.fn_by_qn = {f["qualifiedName"]: f for f in self.fns}
        self.fn_by_name = defaultdict(list)
        for f in self.fns:
            self.fn_by_name[f["name"]].append(f)
        # traits internos, para restringir as arestas IMPLEMENTS
        self.traits = defaultdict(list)
        for t in self.types:
            if t["kind"] == "trait":
                self.traits[t["name"]].append(t)

    def resolve_type(self, name, file, qn):
        """Localiza o tipo do levantamento correspondente a um no do grafo."""
        if file and (name, file) in self.by_key:
            return self.by_key[(name, file)]
        if qn:
            norm = qn.replace("crate::", "rucene::", 1)
            if norm in self.by_qn:
                return self.by_qn[norm]
        cand = self.by_name.get(name, [])
        if len(cand) == 1:
            return cand[0]
        return None

    def resolve_fn(self, name, file, qn):
        if qn:
            norm = qn.replace("crate::", "rucene::", 1)
            if norm in self.fn_by_qn:
                return self.fn_by_qn[norm]
        cand = [f for f in self.fn_by_name.get(name, []) if not file or f["file"] == file]
        if len(cand) == 1:
            return cand[0]
        cand = self.fn_by_name.get(name, [])
        if len(cand) == 1:
            return cand[0]
        return None


# ---------------------------------------------------------------------------
# Fase 1 - higiene dos nos legados
# ---------------------------------------------------------------------------


def phase_repair(roadmap, s: Survey, batch_size):
    print("phase: repair", file=sys.stderr)
    stamp = {"gitCommit": s.commit, "gitDate": s.date}

    # --- 1.1 no sem etiqueta -------------------------------------------------
    orphans = read(roadmap, "MATCH (n) WHERE size(labels(n)) = 0 RETURN count(n) AS c")
    if orphans and orphans[0]["c"]:
        rmp(
            "delete",
            roadmap,
            "MATCH (n) WHERE size(labels(n)) = 0 DETACH DELETE n",
            "drop-unlabelled",
        )
        print(f"  unlabelled nodes deleted: {orphans[0]['c']}", file=sys.stderr)

    # --- 1.2 duplicados do lado Lucene --------------------------------------
    # Um no `Class` de um pacote `org.apache.lucene` sem `kind` e um esboco
    # criado como alvo de um `PORTS`; o no canonico do mesmo tipo ja existe.
    stubs = read(
        roadmap,
        "MATCH (n:Class) WHERE n.kind IS NULL AND n.package STARTS WITH "
        "'org.apache.lucene' RETURN n.qualifiedName AS qn, n.name AS name, "
        "n.package AS pkg",
    )
    if stubs:
        # O id da origem, e nao o nome: um `MATCH (x {name:...})` apanharia
        # tambem os `Method` e as `Class` do lado Java com o mesmo nome.
        ports = read(
            roadmap,
            "MATCH (x)-[:PORTS]->(n:Class) WHERE n.kind IS NULL AND n.package "
            "STARTS WITH 'org.apache.lucene' RETURN coalesce(n.qualifiedName, "
            "n.package + '.' + n.name) AS target, id(x) AS srcId",
        )
        rmp(
            "delete",
            roadmap,
            "MATCH (n:Class) WHERE n.kind IS NULL AND n.package STARTS WITH "
            "'org.apache.lucene' DETACH DELETE n",
            "drop-lucene-stubs",
        )
        print(f"  lucene stub duplicates deleted: {len(stubs)}", file=sys.stderr)
        for p in ports:
            rmp(
                "create",
                roadmap,
                f"MATCH (x) WHERE id(x) = {p['srcId']} WITH x "
                f"MATCH (c:Class {{qualifiedName:{esc(p['target'])}}}) "
                "MERGE (x)-[:PORTS]->(c)",
                "re-link PORTS",
            )
        print(f"  PORTS re-linked to the canonical type: {len(ports)}",
              file=sys.stderr)

    # --- 1.2b `PORTS` apontado ao ficheiro Java em vez do tipo ---------------
    misdirected = read(
        roadmap,
        "MATCH (x)-[:PORTS]->(f:File) WHERE f.path STARTS WITH 'lucene/core/' "
        "RETURN id(x) AS srcId, f.package AS pkg, f.name AS fname",
    )
    for m in misdirected:
        qn = f"{m['pkg']}.{m['fname'][:-5]}"
        rmp(
            "create",
            roadmap,
            f"MATCH (x) WHERE id(x) = {m['srcId']} WITH x "
            f"MATCH (c:Class {{qualifiedName:{esc(qn)}}}) MERGE (x)-[:PORTS]->(c)",
            "re-point PORTS",
        )
    if misdirected:
        rmp(
            "delete",
            roadmap,
            "MATCH (x)-[r:PORTS]->(f:File) WHERE f.path STARTS WITH 'lucene/core/' "
            "DELETE r",
            "drop-file-PORTS",
        )
        print(
            f"  PORTS re-pointed from File to Class: {len(misdirected)}",
            file=sys.stderr,
        )

    # --- 1.4 nos `Module` que sao na verdade ficheiros ----------------------
    rmp(
        "update",
        roadmap,
        "MATCH (n:Module) WHERE n.name STARTS WITH 'src/' "
        "SET n:File, n.path = n.name, n.language = 'Rust' REMOVE n:Module",
        "module->rustfile",
    )

    # --- 1.3 modulos de teste (TestSuite) -----------------------------------
    suites = read(
        roadmap,
        "MATCH (n:TestSuite)-[:TESTS]->(x) RETURN coalesce(n.path, n.file) AS "
        "origin, x.name AS target",
    )
    if suites:
        # As arestas TESTS sobem para o ficheiro que contem o modulo de teste,
        # que e a origem que o modelo ja preve para um TESTS.
        run_unwind(
            "create",
            roadmap,
            [
                {"o": r["origin"], "t": r["target"]}
                for r in suites
                if r["origin"] and r["target"] and r["target"] != r["origin"]
            ],
            "MATCH (f:File {path:row.o}) MATCH (x {name:row.t}) "
            "MERGE (f)-[:TESTS]->(x)",
            min(batch_size, PER_ROW_SCAN_BATCH),
            "TestSuite TESTS lifted to file",
        )
    rmp("delete", roadmap, "MATCH (n:TestSuite) DETACH DELETE n", "drop-testsuite")

    # --- 1.5 colapso das etiquetas duplicadas -------------------------------
    # Os nos sao identificados por `id(n)`: as etiquetas legadas coexistem umas
    # com as outras e com as canonicas, e so o id e inequivoco.
    legacy = read(
        roadmap,
        "MATCH (n) WHERE n:Struct OR n:Trait OR n:Enum OR n:Interface OR n:Test "
        "OR n:Component OR (n:Class AND (n.file STARTS WITH 'src/' "
        "OR n.qualifiedName STARTS WITH 'rucene::' "
        "OR n.qualifiedName STARTS WITH 'crate::')) "
        "RETURN id(n) AS id, labels(n) AS labels, n.name AS name, "
        "n.file AS file, n.qualifiedName AS qn",
    )
    to_type, to_fn, keep_component, unresolved = [], [], [], []
    for row in legacy:
        labels = row["labels"]
        name, file, qn = row["name"], row["file"], row["qn"]
        t = s.resolve_type(name, file, qn)
        if t is not None:
            to_type.append(dict(row, target=t, label=TYPE_LABEL[t["kind"]]))
            continue
        f = s.resolve_fn(name, file, qn)
        if f is not None:
            to_fn.append(dict(row, target=f))
            continue
        if "Component" in labels:
            keep_component.append(row)
        else:
            unresolved.append(row)

    drop = set(LEGACY_TYPE_LABELS) | {"Test", "Component", "Class"}

    # 1.5a nos que sao tipos: etiqueta canonica + file/qualifiedName corrigidos
    groups = defaultdict(list)
    for r in to_type:
        remove = tuple(sorted(set(r["labels"]) & drop - {r["label"]}))
        groups[(r["label"], remove)].append(r)
    for (label, remove), rows in groups.items():
        rem = "".join(f", n:{x}" for x in remove)
        run_unwind(
            "update",
            roadmap,
            [
                {
                    "id": r["id"],
                    "file": r["target"]["file"],
                    "qualifiedName": r["target"]["qualifiedName"],
                    "kind": r["target"]["kind"],
                    "visibility": r["target"]["visibility"],
                    "scope": r["target"]["scope"],
                    **stamp,
                }
                for r in rows
            ],
            "MATCH (n) WHERE id(n) = row.id "
            f"SET n:{label}, n.file = row.file, "
            "n.qualifiedName = row.qualifiedName, n.kind = row.kind, "
            "n.visibility = row.visibility, n.scope = row.scope, "
            "n.language = 'Rust', n.gitCommit = row.gitCommit, "
            f"n.gitDate = row.gitDate REMOVE n.package{rem}",
            min(batch_size, PER_ROW_SCAN_BATCH),
            f"relabel {'+'.join(remove) or '-'} -> {label}",
        )

    # 1.5b nos que sao funcoes
    groups = defaultdict(list)
    for r in to_fn:
        remove = tuple(sorted(set(r["labels"]) & drop))
        groups[remove].append(r)
    for remove, rows in groups.items():
        rem = "".join(f", n:{x}" for x in remove)
        run_unwind(
            "update",
            roadmap,
            [
                {
                    "id": r["id"],
                    "file": r["target"]["file"],
                    "qualifiedName": r["target"]["qualifiedName"],
                    "kind": r["target"]["kind"],
                    "visibility": r["target"]["visibility"],
                    "scope": r["target"]["scope"],
                    **stamp,
                }
                for r in rows
            ],
            "MATCH (n) WHERE id(n) = row.id "
            "SET n:RustFn, n.file = row.file, "
            "n.qualifiedName = row.qualifiedName, n.kind = row.kind, "
            "n.visibility = row.visibility, n.scope = row.scope, "
            "n.language = 'Rust', n.gitCommit = row.gitCommit, "
            f"n.gitDate = row.gitDate REMOVE n.package{rem}",
            min(batch_size, PER_ROW_SCAN_BATCH),
            f"relabel {'+'.join(remove) or '-'} -> RustFn",
        )

    # 1.5c `Component` que continua a ser um Component: um modulo de funcoes
    #      livres que nao declara nenhum tipo.
    # Um `Component` que sobra e um modulo de funcoes livres: pode ser um
    # ficheiro-modulo (`src/util/vector_util.rs`) ou um `mod` inline.
    module_files = {f["name"].replace(".rs", ""): f["path"] for f in s.files}
    module_files.update({m["name"]: m["file"] for m in s.mods})
    comp_rows = []
    for row in keep_component:
        file = row["file"]
        if not file:
            snake = "".join(
                ("_" + c.lower()) if c.isupper() else c for c in row["name"]
            ).lstrip("_")
            file = module_files.get(snake) or module_files.get(row["name"])
        comp_rows.append({"id": row["id"], "file": file, **stamp})
    run_unwind(
        "update",
        roadmap,
        comp_rows,
        "MATCH (n) WHERE id(n) = row.id SET n.file = row.file, "
        "n.kind = 'module', n.language = 'Rust', "
        "n.gitCommit = row.gitCommit, n.gitDate = row.gitDate",
        min(batch_size, PER_ROW_SCAN_BATCH),
        "Component kept (module of free functions)",
    )
    for row in unresolved:
        print(f"  UNRESOLVED legacy node: {row}", file=sys.stderr)
    print(
        f"  legacy nodes: {len(to_type)} -> type labels, {len(to_fn)} -> RustFn, "
        f"{len(keep_component)} kept as Component, {len(unresolved)} unresolved",
        file=sys.stderr,
    )


# ---------------------------------------------------------------------------
# Fase 2 - nos do levantamento
# ---------------------------------------------------------------------------


def phase_nodes(roadmap, s: Survey, batch_size):
    print("phase: nodes", file=sys.stderr)
    stamp = {"gitCommit": s.commit, "gitDate": s.date}

    # Raiz do crate, para que o carregamento funcione a partir de um grafo
    # vazio e nao apenas sobre um ja povoado.
    rmp(
        "create",
        roadmap,
        "MERGE (p:Project {name:'Rucene'}) MERGE (m:Module {name:'rucene'}) "
        "MERGE (p)-[:CONTAINS]->(m)",
        "crate root",
    )
    rmp(
        "update",
        roadmap,
        "MATCH (p:Project {name:'Rucene'}) SET p.language = 'Rust', p.path = '.', "
        f"p.gitCommit = {esc(s.commit)}, p.gitDate = {esc(s.date)}",
        "crate root props",
    )
    rmp(
        "update",
        roadmap,
        "MATCH (m:Module {name:'rucene'}) SET m.kind = 'crate', m.path = '.', "
        f"m.gitCommit = {esc(s.commit)}, m.gitDate = {esc(s.date)}",
        "crate module props",
    )
    rmp(
        "update",
        roadmap,
        "MATCH (:Project {name:'Rucene'})-[r:CONTAINS]->(:Module {name:'rucene'}) "
        f"SET r.gitCommit = {esc(s.commit)}, r.gitDate = {esc(s.date)}",
        "crate root edge",
    )

    run_unwind(
        "create",
        roadmap,
        [{"path": f["path"]} for f in s.files],
        "MERGE (f:File {path:row.path})",
        batch_size,
        "File create",
    )
    run_unwind(
        "update",
        roadmap,
        [
            {
                "path": f["path"],
                "name": f["name"],
                "kind": f["kind"],
                "modulePath": f["modulePath"],
                "crate": f["crate"],
                "loc": str(f["loc"]),
                **stamp,
            }
            for f in s.files
        ],
        "MATCH (f:File {path:row.path}) SET f.name = row.name, "
        "f.kind = row.kind, f.modulePath = row.modulePath, f.crate = row.crate, "
        "f.loc = row.loc, f.language = 'Rust', f.gitCommit = row.gitCommit, "
        "f.gitDate = row.gitDate",
        batch_size,
        "File update",
    )

    for kind, label in (
        ("struct", "RustStruct"),
        ("union", "RustStruct"),
        ("trait", "RustTrait"),
        ("enum", "RustEnum"),
        ("alias", "RustAlias"),
    ):
        sel = [t for t in s.types if t["kind"] == kind]
        if not sel:
            continue
        run_unwind(
            "create",
            roadmap,
            [{"name": t["name"], "file": t["file"]} for t in sel],
            f"MERGE (t:{label} {{name:row.name, file:row.file}})",
            batch_size,
            f"{label} create ({kind})",
        )
        run_unwind(
            "update",
            roadmap,
            [
                {
                    "name": t["name"],
                    "file": t["file"],
                    "qualifiedName": t["qualifiedName"],
                    "kind": t["kind"],
                    "visibility": t["visibility"],
                    "scope": t["scope"],
                    "origin": t.get("origin", "source"),
                    **stamp,
                }
                for t in sel
            ],
            f"MATCH (t:{label} {{name:row.name, file:row.file}}) "
            "SET t.qualifiedName = row.qualifiedName, t.kind = row.kind, "
            "t.visibility = row.visibility, t.scope = row.scope, "
            "t.origin = row.origin, "
            "t.language = 'Rust', t.gitCommit = row.gitCommit, "
            "t.gitDate = row.gitDate",
            batch_size,
            f"{label} update ({kind})",
        )

    run_unwind(
        "create",
        roadmap,
        [{"qualifiedName": f["qualifiedName"]} for f in s.fns],
        "MERGE (f:RustFn {qualifiedName:row.qualifiedName})",
        min(batch_size, PER_ROW_SCAN_BATCH),
        "RustFn create",
    )
    run_unwind(
        "update",
        roadmap,
        [
            {
                "qualifiedName": f["qualifiedName"],
                "name": f["name"],
                "file": f["file"],
                "kind": f["kind"],
                "owner": f["owner"],
                "visibility": f["visibility"],
                "scope": f["scope"],
                "signature": f["signature"][:200],
                "origin": f.get("origin", "source"),
                **stamp,
            }
            for f in s.fns
        ],
        "MATCH (f:RustFn {qualifiedName:row.qualifiedName}) SET f.name = row.name, "
        "f.file = row.file, f.kind = row.kind, f.owner = row.owner, "
        "f.visibility = row.visibility, f.scope = row.scope, "
        "f.signature = row.signature, f.origin = row.origin, f.language = 'Rust', "
        "f.gitCommit = row.gitCommit, f.gitDate = row.gitDate",
        batch_size,
        "RustFn update",
    )


# ---------------------------------------------------------------------------
# Fase 3 - arestas
# ---------------------------------------------------------------------------


def phase_edges(roadmap, s: Survey, batch_size):
    print("phase: edges", file=sys.stderr)

    run_unwind(
        "create",
        roadmap,
        [{"path": f["path"]} for f in s.files],
        "MATCH (m:Module {name:'rucene'}), (f:File {path:row.path}) "
        "MERGE (m)-[:CONTAINS]->(f)",
        batch_size,
        "Module CONTAINS File",
    )

    for label in ALL_TYPE_LABELS:
        sel = [t for t in s.types if TYPE_LABEL[t["kind"]] == label]
        run_unwind(
            "create",
            roadmap,
            [{"name": t["name"], "file": t["file"]} for t in sel],
            f"MATCH (f:File {{path:row.file}}), "
            f"(t:{label} {{name:row.name, file:row.file}}) "
            "MERGE (f)-[:DECLARES]->(t)",
            batch_size,
            f"File DECLARES {label}",
        )

    run_unwind(
        "create",
        roadmap,
        [{"qn": f["qualifiedName"], "file": f["file"]} for f in s.fns],
        "MATCH (f:File {path:row.file}), (n:RustFn {qualifiedName:row.qn}) "
        "MERGE (f)-[:DECLARES]->(n)",
        batch_size,
        "File DECLARES RustFn",
    )

    # tipo -> metodo, quando o dono e um tipo declarado no mesmo ficheiro
    owned = []
    for f in s.fns:
        if not f["owner"]:
            continue
        t = s.by_key.get((f["owner"], f["file"]))
        if t is None:
            continue
        owned.append(
            {"name": t["name"], "file": t["file"], "qn": f["qualifiedName"],
             "label": TYPE_LABEL[t["kind"]]}
        )
    for label in ALL_TYPE_LABELS:
        sel = [o for o in owned if o["label"] == label]
        run_unwind(
            "create",
            roadmap,
            [{"name": o["name"], "file": o["file"], "qn": o["qn"]} for o in sel],
            f"MATCH (t:{label} {{name:row.name, file:row.file}}), "
            "(n:RustFn {qualifiedName:row.qn}) MERGE (t)-[:DECLARES]->(n)",
            batch_size,
            f"{label} DECLARES RustFn",
        )

    # impl Trait for Type -> IMPLEMENTS (apenas traits internos do crate)
    impl_rows = []
    for im in s.impls:
        if not im["trait"]:
            continue
        t = s.by_key.get((im["type"], im["file"]))
        if t is None:
            cand = s.by_name.get(im["type"], [])
            t = cand[0] if len(cand) == 1 else None
        if t is None:
            continue
        traits = s.traits.get(im["trait"], [])
        tr = next((x for x in traits if x["file"] == im["file"]), None) or (
            traits[0] if len(traits) == 1 else None
        )
        if tr is None:
            continue
        impl_rows.append(
            {
                "srcLabel": TYPE_LABEL[t["kind"]],
                "sn": t["name"],
                "sf": t["file"],
                "tn": tr["name"],
                "tf": tr["file"],
            }
        )
    for label in ALL_TYPE_LABELS:
        sel = [r for r in impl_rows if r["srcLabel"] == label]
        run_unwind(
            "create",
            roadmap,
            [{"sn": r["sn"], "sf": r["sf"], "tn": r["tn"], "tf": r["tf"]} for r in sel],
            f"MATCH (a:{label} {{name:row.sn, file:row.sf}}), "
            "(b:RustTrait {name:row.tn, file:row.tf}) MERGE (a)-[:IMPLEMENTS]->(b)",
            batch_size,
            f"{label} IMPLEMENTS RustTrait",
        )

    run_unwind(
        "create",
        roadmap,
        [{"a": d["from"], "b": d["to"]} for d in s.deps],
        "MATCH (a:File {path:row.a}), (b:File {path:row.b}) "
        "MERGE (a)-[:DEPENDS_ON]->(b)",
        batch_size,
        "File DEPENDS_ON File",
    )


# ---------------------------------------------------------------------------
# Fase 3c - proveniencia das arestas
# ---------------------------------------------------------------------------


def phase_stamp(roadmap, s: Survey):
    """Carimba `gitCommit`/`gitDate` em todas as arestas que este carregador
    escreve. Corre depois da reconciliacao, porque a transferencia de arestas
    cria arestas novas."""
    print("phase: stamp", file=sys.stderr)
    stamp = f"SET r.gitCommit = {esc(s.commit)}, r.gitDate = {esc(s.date)}"
    patterns = [
        "MATCH (:Module {name:'rucene'})-[r:CONTAINS]->(:File)",
        "MATCH (f:File)-[r:DECLARES]->() WHERE f.language = 'Rust'",
        "MATCH (f:File)-[r:DEPENDS_ON]->(:File) WHERE f.language = 'Rust'",
        "MATCH (f:File)-[r:TESTS]->() WHERE f.language = 'Rust'",
    ]
    for label in ALL_TYPE_LABELS:
        patterns.append(f"MATCH (:{label})-[r:DECLARES]->(:RustFn)")
        patterns.append(f"MATCH (:{label})-[r:IMPLEMENTS]->(:RustTrait)")
        patterns.append(f"MATCH (:{label})-[r:PORTS]->(:Class)")
        patterns.append(f"MATCH (:{label})-[r:DEPENDS_ON]->()")
        patterns.append(f"MATCH ()-[r:DEPENDS_ON]->(:{label})")
    patterns.append("MATCH (:Component)-[r]->()")
    patterns.append("MATCH (:RustFn)-[r]->()")
    for pat in patterns:
        sep = "AND" if " WHERE " in pat else "WHERE"
        rmp("update", roadmap, f"{pat} {sep} r.gitCommit IS NULL {stamp}", "stamp")
    print(f"  stamped {len(patterns)} edge shapes", file=sys.stderr)


# ---------------------------------------------------------------------------
# Fase 3b - reconciliacao dos nos pre-existentes
# ---------------------------------------------------------------------------


_PROP_KEY = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


def _lit(v) -> str:
    """A Cypher literal for a property value read back from the graph."""
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return repr(v)
    if isinstance(v, str):
        return esc(v)
    if isinstance(v, list):
        return "[" + ", ".join(_lit(x) for x in v) + "]"
    raise ValueError(f"unsupported property value: {v!r}")


def _prop_key(k: str) -> str:
    return k if _PROP_KEY.match(k) else "`" + k.replace("`", "``") + "`"


def report_dropped_edge(dropped, phase, node, edge, direction, other, props, reason):
    """Record one edge that is about to be lost, on stderr and in `dropped`.

    `node` is the Rust node's identity (`label`, `name`, `file`, and
    `qualifiedName` for a `RustFn`); `other` names the node at the far end
    (the Lucene type for a `PORTS` edge); `direction` is `"out"` or `"in"`
    relative to `node`; `props` are the edge's properties.
    """
    props = props or {}
    rec = {
        "phase": phase,
        "node": node,
        "edgeType": edge,
        "direction": direction,
        "other": other,
        "lucene": other if edge == "PORTS" else None,
        "note": props.get("note"),
        "evidence": props.get("evidence"),
        "declaredAt": props.get("declaredAt"),
        "properties": props,
        "reason": reason,
    }
    who = node.get("qualifiedName") or f"{node.get('name')} @ {node.get('file')}"
    ends = f"{who} -> {other}" if direction == "out" else f"{other} -> {who}"
    print(
        f"  DROPPED {edge}: {node.get('label')} {ends} "
        f"[{reason}] note: {rec['note'] if rec['note'] is not None else '(none)'}",
        file=sys.stderr,
    )
    dropped.append(rec)


def report_dropped_port(dropped, phase, node, lucene, props, reason):
    """`report_dropped_edge` for a `PORTS` edge leaving `node` for `lucene`."""
    report_dropped_edge(dropped, phase, node, "PORTS", "out", lucene, props, reason)


def _move_edge(roadmap, src, dst, edge, props) -> bool:
    """Create `(src)-[:edge]->(dst)` and give it the properties in `props` it lacks.

    When the surviving node already has that edge, its values are canonical and
    are kept: each property is written as `coalesce(e.k, <stale value>)`, so a
    stale value only fills a key the edge does not have.

    Returns whether the edge now exists and holds every key of `props`. The engine
    lets a `MERGE` match an edge of another type between the same ordered pair
    and then create nothing (`knowledge-model.md`, provenance convention), so
    the move is read back rather than assumed.
    """
    props = {k: v for k, v in (props or {}).items() if v is not None}
    typ = _prop_key(edge)
    rmp(
        "create",
        roadmap,
        f"MATCH (x) WHERE id(x) = {src} WITH x "
        f"MATCH (y) WHERE id(y) = {dst} MERGE (x)-[:{typ}]->(y)",
        f"move-{edge}",
    )
    if props:
        sets = ", ".join(f"e.{_prop_key(k)} = coalesce(e.{_prop_key(k)}, {_lit(v)})"
                         for k, v in sorted(props.items()))
        rmp(
            "update",
            roadmap,
            f"MATCH (x)-[e:{typ}]->(y) WHERE id(x) = {src} AND id(y) = {dst} "
            f"SET {sets}",
            f"move-{edge}-properties",
        )
    back = read(
        roadmap,
        f"MATCH (x)-[e:{typ}]->(y) WHERE id(x) = {src} AND id(y) = {dst} "
        "RETURN properties(e) AS props",
    )
    if not back:
        return False
    got = back[0].get("props") or {}
    return all(got.get(k) is not None for k in props)


def _move_port(roadmap, canon_id, port) -> bool:
    """`_move_edge` for one `PORTS` edge (`port` carries `cid` and `props`)."""
    return _move_edge(roadmap, canon_id, port["cid"], "PORTS", port.get("props"))


def _far_end(e):
    return e.get("qn") or e.get("path") or e.get("name") or f"id {e['other']}"


def _transfer_edges(roadmap, stale_id, canon_id, batch_size, stale_node=None,
                    dropped=None):
    """Move todas as arestas de `stale_id` para `canon_id`.

    Every edge, of every type and in both directions, is re-created with **all
    its properties** (`evidence`, `note`, `declaredAt`, `gitCommit`, ...) and
    read back. It used to be re-created by type alone, which dropped every
    property -- the `note` of a re-kinded type's `PORTS` among them. An edge that
    cannot be re-created is reported through `report_dropped_edge` instead of
    vanishing with the stale node. A self-loop becomes a self-loop on
    `canon_id`.
    """
    if dropped is None:
        dropped = []
    node = stale_node or {"id": stale_id}
    cols = ("RETURN type(r) AS t, id(b) AS other, properties(r) AS props, "
            "b.qualifiedName AS qn, b.path AS path, b.name AS name")
    out = read(roadmap, f"MATCH (a)-[r]->(b) WHERE id(a) = {stale_id} {cols}")
    inn = read(roadmap, f"MATCH (b)-[r]->(a) WHERE id(a) = {stale_id} {cols}")
    moved = 0
    # Uma aresta por consulta, ancorando cada extremo com `WITH`: um
    # `MATCH (x), (y)` produziria o produto cartesiano de todos os nos.
    for direction, edges in (("out", out), ("in", inn)):
        for e in edges:
            other = canon_id if e["other"] == stale_id else e["other"]
            if direction == "in" and e["other"] == stale_id:
                continue  # the self-loop, already moved as an outgoing edge
            src, dst = (canon_id, other) if direction == "out" else (other, canon_id)
            if _move_edge(roadmap, src, dst, e["t"], e.get("props")):
                moved += 1
            else:
                report_dropped_edge(
                    dropped, "reconcile", node, e["t"], direction, _far_end(e),
                    e.get("props"), "could not be re-created on the surviving node")
    return moved


def _type_identity(r):
    labels = [lb for lb in r.get("labels") or [] if lb in ALL_TYPE_LABELS]
    return {"label": labels[0] if labels else (r.get("labels") or [None])[0],
            "name": r.get("name"), "file": r.get("file")}


def phase_reconcile(roadmap, s: Survey, batch_size, dropped=None):
    """Funde os nos Rust pre-existentes que o levantamento mostra serem o mesmo
    conceito de um no canonico (mesmo tipo, `file` em falta ou errado, ou
    etiqueta errada), preservando as arestas.

    This is where a re-kinded item lands: an `enum` that became a `trait` keeps
    its `{name, file}` identity, `phase_nodes` creates the `RustTrait`, and the
    old `RustEnum` is merged into it here. Its edges -- `PORTS` and every other
    type -- move with their properties (`_transfer_edges`); one that cannot be
    moved is appended to `dropped` and printed, never lost silently.
    """
    print("phase: reconcile", file=sys.stderr)
    if dropped is None:
        dropped = []
    rows = read(
        roadmap,
        "MATCH (n) WHERE n:RustStruct OR n:RustTrait OR n:RustEnum OR n:RustAlias "
        "RETURN id(n) AS id, labels(n) AS labels, n.name AS name, n.file AS file, "
        "n.qualifiedName AS qn",
    )
    canon = {}
    dupes = []
    for r in sorted(rows, key=lambda x: x["id"]):
        key = (r["name"], r["file"])
        t = s.by_key.get(key)
        if t is not None and TYPE_LABEL[t["kind"]] in r["labels"]:
            if t["qualifiedName"] in canon:
                # duplicado exacto: o grafo tinha ja dois nos para o mesmo tipo
                dupes.append(r)
            else:
                canon[t["qualifiedName"]] = r["id"]

    merged = deleted = unresolved = 0
    for r in dupes:
        cid = canon[s.by_key[(r["name"], r["file"])]["qualifiedName"]]
        merged += _transfer_edges(roadmap, r["id"], cid, batch_size,
                                  _type_identity(r), dropped)
        rmp(
            "delete",
            roadmap,
            f"MATCH (n) WHERE id(n) = {r['id']} DETACH DELETE n",
            "drop-duplicate",
        )
        deleted += 1
    for r in rows:
        if r in dupes:
            continue
        key = (r["name"], r["file"])
        t = s.by_key.get(key)
        if t is not None and TYPE_LABEL[t["kind"]] in r["labels"]:
            continue  # ja e o no canonico
        target = s.resolve_type(r["name"], r["file"], r["qn"])
        if target is None or target["qualifiedName"] not in canon:
            print(
                f"  UNRESOLVED stale node: {r['labels']} {r['name']} {r['file']}",
                file=sys.stderr,
            )
            unresolved += 1
            continue
        cid = canon[target["qualifiedName"]]
        if cid == r["id"]:
            continue
        merged += _transfer_edges(roadmap, r["id"], cid, batch_size,
                                  _type_identity(r), dropped)
        rmp(
            "delete",
            roadmap,
            f"MATCH (n) WHERE id(n) = {r['id']} DETACH DELETE n",
            "drop-stale",
        )
        deleted += 1
    print(
        f"  stale nodes merged: {deleted} (edges moved: {merged}), "
        f"unresolved: {unresolved}",
        file=sys.stderr,
    )


# ---------------------------------------------------------------------------
# Fase 4 - auditoria
# ---------------------------------------------------------------------------


def phase_audit(roadmap, s: Survey):
    print("phase: audit", file=sys.stderr)
    problems = 0

    got = {r["path"] for r in read(roadmap, "MATCH (f:File) WHERE f.path ENDS WITH '.rs' RETURN f.path AS path")}
    want = {f["path"] for f in s.files}
    for p in sorted(want - got):
        print(f"  MISSING File: {p}", file=sys.stderr)
        problems += 1
    for p in sorted(got - want):
        print(f"  STALE File (not in the tree at {s.commit[:7]}): {p}",
              file=sys.stderr)
        problems += 1

    rows = read(
        roadmap,
        "MATCH (n) WHERE n:RustStruct OR n:RustTrait OR n:RustEnum OR n:RustAlias "
        "RETURN n.name AS name, n.file AS file, labels(n) AS labels",
    )
    want_types = {(t["name"], t["file"]): TYPE_LABEL[t["kind"]] for t in s.types}
    seen = set()
    for r in rows:
        key = (r["name"], r["file"])
        if key in seen:
            print(f"  DUPLICATE type node: {key}", file=sys.stderr)
            problems += 1
        seen.add(key)
        if key not in want_types:
            print(f"  STALE type node: {key} {r['labels']}", file=sys.stderr)
            problems += 1
        elif want_types[key] not in r["labels"]:
            print(
                f"  WRONG label: {key} has {r['labels']}, survey says "
                f"{want_types[key]}",
                file=sys.stderr,
            )
            problems += 1
    for key in sorted(set(want_types) - seen):
        print(f"  MISSING type node: {key}", file=sys.stderr)
        problems += 1

    dup_labels = read(
        roadmap,
        "MATCH (n) WHERE n:Struct OR n:Trait OR n:Enum OR n:Interface OR n:Test "
        "OR n:TestSuite OR n:File AND FALSE RETURN count(n) AS c",
    )
    if dup_labels and dup_labels[0]["c"]:
        print(f"  LEGACY labels still present: {dup_labels[0]['c']}", file=sys.stderr)
        problems += 1

    print(f"  audit problems: {problems}", file=sys.stderr)
    return problems


def phase_prune(roadmap, s: Survey, batch_size, dropped=None):
    """Delete the files and nodes the survey no longer confirms.

    `phase_audit` reports them; without this phase they accumulate, and a node
    for a type the crate deleted is a false statement about the code.

    **Which rule yields.** The original safeguard refused to prune *any* node
    carrying a `PORTS` edge, and `port_evidence_kg.py` never retired an edge
    whose declaration site had vanished. Between them a deleted file kept its
    port for ever: `TwoPhaseCommitTool` still read `ported` at 9cf23e5 on a
    doc comment in `src/index/two_phase_commit.rs`, a file the same commit
    deleted. That inflates coverage silently for every file the crate will ever
    delete, which is exactly what `CLAUDE.md` 6.1 forbids -- coverage has to be
    defensible.

    So the safeguard yields, but only where it was wrong. It conflated two
    situations it could not tell apart:

    1. the crate really deleted the item -- the edge must go with it;
    2. the survey failed to *see* an item that exists -- nothing may be deleted.

    The discriminator is not "does it carry `PORTS`" but "can this survey see
    this kind of item at all". A survey run without `--expand` cannot see the
    macro-generated types, and a curated edge has no source site for any survey
    to confirm. Those two are still protected, by name:

    * `evidence: 'curated'` -- asserted by hand, not by a doc comment, so the
      crate survey is not its authority;
    * `declarationSource: 'macro-expansion'` -- declared by a type no source
      line contains, invisible to a survey that did not expand macros.

    Everything else is a `doc-comment` edge over a source-declared item, and
    for those the survey *is* the authority: if it does not list the item, the
    doc comment that declared the edge does not exist either. `port_evidence_kg.py`
    closes the same gap from the other end, retiring a `doc-comment` edge whose
    declaration site no longer states the claim, so an edge cannot outlive its
    evidence even when the node survives for another reason.

    Stale `File` nodes are pruned here too. `phase_audit` has always reported
    them and nothing ever deleted them, so a deleted `.rs` file left a `File`
    node behind permanently -- the third limb of the same defect.

    The remaining safeguard is unchanged and unconditional: an empty survey is
    refused, so a failed extraction can never be read as "the crate has no
    types".

    **No `PORTS` edge is lost silently.** Before a node carrying `PORTS` is
    deleted, a surviving node with the same identity takes the edge with all
    its properties: for a `RustFn` (identity `qualifiedName`), the one surviving
    `RustFn` with the same `file` and `name`, when exactly one exists -- the fn
    whose kind or owner changed. A type cannot have such a survivor here: a
    stale type's `{name, file}` is by definition absent from the survey, and a
    re-kinded type keeps its `{name, file}` and is merged by `phase_reconcile`.
    Every edge that is not moved is printed with the node's identity, the
    Lucene target and its `note`, and appended to `dropped`, which `main`
    writes to the dropped-ports report.
    """
    print("phase: prune", file=sys.stderr)
    if dropped is None:
        dropped = []
    if not s.types or not s.fns or not s.files:
        print("  survey is empty; refusing to prune", file=sys.stderr)
        return

    survey_types = {(t["name"], t["file"]) for t in s.types}
    survey_fns = {f["qualifiedName"] for f in s.fns}

    graph_types = read(
        roadmap,
        "MATCH (t) WHERE t:RustStruct OR t:RustTrait OR t:RustEnum OR t:RustAlias "
        "RETURN t.name AS name, t.file AS file",
    )
    graph_fns = read(roadmap, "MATCH (f:RustFn) RETURN f.qualifiedName AS qn, "
                              "id(f) AS id, f.name AS name, f.file AS file")

    stale_types = [
        {"name": r["name"], "file": r["file"]}
        for r in graph_types
        if (r["name"], r["file"]) not in survey_types
    ]
    stale_fns = [
        {"qn": r["qn"]} for r in graph_fns if r["qn"] not in survey_fns
    ]

    # Only the two edge kinds this survey cannot judge are protected; see the
    # docstring for why the blanket `PORTS` protection had to go.
    unjudgeable = ("MATCH (t)-[e:PORTS]->(:Class) "
                   "WHERE e.evidence = 'curated' "
                   "OR e.declarationSource = 'macro-expansion' ")
    protected = {
        (r["name"], r["file"])
        for r in read(roadmap, unjudgeable + "RETURN t.name AS name, t.file AS file")
    }
    kept = [t for t in stale_types if (t["name"], t["file"]) in protected]
    stale_types = [t for t in stale_types if (t["name"], t["file"]) not in protected]
    protected_fns = {
        r["qn"]
        for r in read(
            roadmap,
            "MATCH (f:RustFn)-[e:PORTS]->(:Class) "
            "WHERE e.evidence = 'curated' OR e.declarationSource = 'macro-expansion' "
            "RETURN f.qualifiedName AS qn",
        )
    }
    kept += [f for f in stale_fns if f["qn"] in protected_fns]
    stale_fns = [f for f in stale_fns if f["qn"] not in protected_fns]

    for t in kept:
        print(f"  KEPT (carries a curated or macro-expansion PORTS): {t}",
              file=sys.stderr)
    if stale_types:
        doomed = {(t["name"], t["file"]) for t in stale_types}
        for r in read(
            roadmap,
            "MATCH (t)-[e:PORTS]->(c:Class) "
            "WHERE t:RustStruct OR t:RustTrait OR t:RustEnum OR t:RustAlias "
            "RETURN t.name AS name, t.file AS file, labels(t) AS labels, "
            "c.qualifiedName AS lucene, properties(e) AS props",
        ):
            if (r["name"], r["file"]) in doomed:
                report_dropped_port(
                    dropped, "prune", _type_identity(r), r["lucene"], r.get("props"),
                    "node deleted: the survey no longer declares it")
    if stale_fns:
        doomed_qn = {f["qn"] for f in stale_fns}
        survivors = defaultdict(list)
        for r in graph_fns:
            if r["qn"] not in doomed_qn and r.get("name") is not None:
                survivors[(r.get("file"), r["name"])].append(r)
        for r in read(
            roadmap,
            "MATCH (f:RustFn)-[e:PORTS]->(c:Class) "
            "RETURN f.qualifiedName AS qn, f.name AS name, f.file AS file, "
            "id(c) AS cid, c.qualifiedName AS lucene, properties(e) AS props",
        ):
            if r["qn"] not in doomed_qn:
                continue
            node = {"label": "RustFn", "qualifiedName": r["qn"],
                    "name": r["name"], "file": r["file"]}
            heirs = survivors.get((r["file"], r["name"]), [])
            if len(heirs) == 1:
                if _move_port(roadmap, heirs[0]["id"], r):
                    print(f"  MOVED PORTS {r['qn']} -> {heirs[0]['qn']} "
                          f"({r['lucene']})", file=sys.stderr)
                    continue
                reason = "could not be re-created on the surviving node"
            elif heirs:
                reason = (f"node deleted: {len(heirs)} surviving RustFn share its "
                          "file and name, none is its unique successor")
            else:
                reason = "node deleted: the survey no longer declares it"
            report_dropped_port(dropped, "prune", node, r["lucene"],
                                r.get("props"), reason)

    survey_files = {f["path"] for f in s.files}
    stale_files = [
        {"path": r["path"]}
        for r in read(
            roadmap,
            "MATCH (f:File) WHERE f.path ENDS WITH '.rs' RETURN f.path AS path",
        )
        if r["path"] not in survey_files
    ]

    if stale_fns:
        run_unwind(
            "delete", roadmap, stale_fns,
            "MATCH (f:RustFn {qualifiedName:row.qn}) DETACH DELETE f",
            batch_size, "stale RustFn deleted",
        )
    if stale_types:
        run_unwind(
            "delete", roadmap, stale_types,
            "MATCH (t {name:row.name, file:row.file}) "
            "WHERE t:RustStruct OR t:RustTrait OR t:RustEnum OR t:RustAlias "
            "DETACH DELETE t",
            min(batch_size, PER_ROW_SCAN_BATCH), "stale type nodes deleted",
        )
    if stale_files:
        run_unwind(
            "delete", roadmap, stale_files,
            "MATCH (f:File {path:row.path}) DETACH DELETE f",
            batch_size, "stale File nodes deleted",
        )
    if not stale_types and not stale_fns and not stale_files:
        print("  nothing to prune", file=sys.stderr)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("roadmap")
    ap.add_argument("--survey", required=True)
    ap.add_argument("--batch-size", type=int, default=200)
    ap.add_argument(
        "--dropped-ports-report",
        default=str(kg_target() / "rucene_kg" / "dropped_ports.jsonl"),
        help="JSON-lines file each edge the reconcile phase could not move, "
        "and each PORTS edge the prune phase could not keep, is appended to "
        "(default: %(default)s)",
    )
    ap.add_argument(
        "--phase",
        default="all",
        choices=[
            "all",
            "repair",
            "nodes",
            "edges",
            "reconcile",
            "prune",
            "stamp",
            "audit",
        ],
    )
    args = ap.parse_args()
    s = Survey(args.survey)
    dropped = []
    if args.phase in ("all", "repair"):
        phase_repair(args.roadmap, s, args.batch_size)
    if args.phase in ("all", "nodes"):
        phase_nodes(args.roadmap, s, args.batch_size)
    if args.phase in ("all", "edges"):
        phase_edges(args.roadmap, s, args.batch_size)
    if args.phase in ("all", "reconcile"):
        phase_reconcile(args.roadmap, s, args.batch_size, dropped)
    if args.phase in ("all", "prune"):
        phase_prune(args.roadmap, s, args.batch_size, dropped)
    if args.phase in ("all", "stamp"):
        phase_stamp(args.roadmap, s)
    if args.phase in ("all", "audit"):
        phase_audit(args.roadmap, s)
    if dropped:
        write_dropped_report(args.dropped_ports_report, s.commit, dropped)


def write_dropped_report(path, commit, dropped):
    """Append the dropped `PORTS` edges to `path`, one JSON object per line."""
    out = Path(path)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("a", encoding="utf-8") as fh:
        for rec in dropped:
            fh.write(json.dumps({"surveyCommit": commit, **rec}, sort_keys=True) + "\n")
    print(f"  dropped PORTS edges: {len(dropped)}, reported in {out}", file=sys.stderr)


if __name__ == "__main__":
    main()
