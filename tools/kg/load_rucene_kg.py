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
    python3 tools/kg/load_rucene_kg.py rucene --survey /tmp/rucene_kg/survey.json
"""

import argparse
import json
import subprocess
import sys
from collections import defaultdict

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


def rmp(mode: str, roadmap: str, query: str, label: str = ""):
    result = subprocess.run(
        ["rmp", "graph", mode, "-r", roadmap],
        input=query,
        text=True,
        capture_output=True,
    )
    if result.returncode != 0:
        print(f"\n{label} FAILED (exit {result.returncode})", file=sys.stderr)
        print(result.stderr, file=sys.stderr)
        print("QUERY:\n" + query[:800], file=sys.stderr)
        sys.exit(1)
    return result.stdout


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
            batch_size,
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
            batch_size,
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
            batch_size,
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
        batch_size,
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
                    **stamp,
                }
                for t in sel
            ],
            f"MATCH (t:{label} {{name:row.name, file:row.file}}) "
            "SET t.qualifiedName = row.qualifiedName, t.kind = row.kind, "
            "t.visibility = row.visibility, t.scope = row.scope, "
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
        batch_size,
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
                **stamp,
            }
            for f in s.fns
        ],
        "MATCH (f:RustFn {qualifiedName:row.qualifiedName}) SET f.name = row.name, "
        "f.file = row.file, f.kind = row.kind, f.owner = row.owner, "
        "f.visibility = row.visibility, f.scope = row.scope, "
        "f.signature = row.signature, f.language = 'Rust', "
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


def _transfer_edges(roadmap, stale_id, canon_id, batch_size):
    """Move todas as arestas de `stale_id` para `canon_id`."""
    out = read(
        roadmap,
        f"MATCH (a)-[r]->(b) WHERE id(a) = {stale_id} "
        "RETURN type(r) AS t, id(b) AS other",
    )
    inn = read(
        roadmap,
        f"MATCH (b)-[r]->(a) WHERE id(a) = {stale_id} "
        "RETURN type(r) AS t, id(b) AS other",
    )
    moved = 0
    # Uma aresta por consulta, ancorando cada extremo com `WITH`: um
    # `MATCH (x), (y)` produziria o produto cartesiano de todos os nos.
    for e in out:
        rmp(
            "create",
            roadmap,
            f"MATCH (x) WHERE id(x) = {canon_id} WITH x "
            f"MATCH (y) WHERE id(y) = {e['other']} "
            f"MERGE (x)-[:{e['t']}]->(y)",
            "transfer-edge-out",
        )
        moved += 1
    for e in inn:
        rmp(
            "create",
            roadmap,
            f"MATCH (x) WHERE id(x) = {canon_id} WITH x "
            f"MATCH (y) WHERE id(y) = {e['other']} "
            f"MERGE (y)-[:{e['t']}]->(x)",
            "transfer-edge-in",
        )
        moved += 1
    return moved


def phase_reconcile(roadmap, s: Survey, batch_size):
    """Funde os nos Rust pre-existentes que o levantamento mostra serem o mesmo
    conceito de um no canonico (mesmo tipo, `file` em falta ou errado, ou
    etiqueta errada), preservando as arestas."""
    print("phase: reconcile", file=sys.stderr)
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
        merged += _transfer_edges(roadmap, r["id"], cid, batch_size)
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
        merged += _transfer_edges(roadmap, r["id"], cid, batch_size)
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


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("roadmap")
    ap.add_argument("--survey", required=True)
    ap.add_argument("--batch-size", type=int, default=200)
    ap.add_argument(
        "--phase",
        default="all",
        choices=[
            "all",
            "repair",
            "nodes",
            "edges",
            "reconcile",
            "stamp",
            "audit",
        ],
    )
    args = ap.parse_args()
    s = Survey(args.survey)
    if args.phase in ("all", "repair"):
        phase_repair(args.roadmap, s, args.batch_size)
    if args.phase in ("all", "nodes"):
        phase_nodes(args.roadmap, s, args.batch_size)
    if args.phase in ("all", "edges"):
        phase_edges(args.roadmap, s, args.batch_size)
    if args.phase in ("all", "reconcile"):
        phase_reconcile(args.roadmap, s, args.batch_size)
    if args.phase in ("all", "stamp"):
        phase_stamp(args.roadmap, s)
    if args.phase in ("all", "audit"):
        phase_audit(args.roadmap, s)


if __name__ == "__main__":
    main()
