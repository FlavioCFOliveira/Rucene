#!/usr/bin/env python3
"""Carrega membros (métodos, construtores, campos) no KG via UNWIND batches.

Processa os ficheiros Cypher gerados por enrich_lucene_kg.py e carrega-os com
muito menos invocações ao rmp, agrupando múltiplos membros por declaração
UNWIND.
"""

import re
import subprocess
import sys
import argparse


def cypher_escape(value: str) -> str:
    """Escapa uma string para usar como literal single-quoted em Cypher."""
    # Primeiro escapa backslashes, depois as aspas simples.
    return value.replace("\\", "\\\\").replace("'", "\\'")


def parse_create_pairs(path: str):
    """Extrai pares (qualifiedName, parentQualifiedName) de um ficheiro create."""
    pairs = {}
    node_re = re.compile(r"MERGE \(([a-z]):Method \{qualifiedName:'((?:\\.|[^'\\])*)'\}\)")
    edge_re = re.compile(
        r"MATCH \(c:Class \{qualifiedName:'((?:\\.|[^'\\])*)'\}\), "
        r"\([a-z]:Method \{qualifiedName:'((?:\\.|[^'\\])*)'\}\) "
        r"MERGE \(c\)-\[:DECLARES\]->\([a-z]\)"
    )
    with open(path, "r", encoding="utf-8") as f:
        lines = [line.strip() for line in f if line.strip()]

    # Cada par de linhas: node merge seguido de edge merge.
    i = 0
    while i < len(lines):
        node_line = lines[i]
        nm = node_re.search(node_line)
        if not nm:
            i += 1
            continue
        qn = nm.group(2)
        if i + 1 < len(lines):
            em = edge_re.search(lines[i + 1])
            if em:
                pairs[qn] = em.group(1)
                i += 2
                continue
        pairs[qn] = None
        i += 1
    return pairs


def parse_update_rows(path: str):
    """Extrai propriedades de cada membro num ficheiro update."""
    rows = {}
    qn_re = re.compile(r"MATCH \([a-z]:Method \{qualifiedName:'((?:\\.|[^'\\])*)'\}\)")
    # Cada atribuição é da forma prop='valor'
    prop_re = re.compile(r"([a-zA-Z_][a-zA-Z0-9_]*)='((?:\\.|[^'\\])*)'")
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            qm = qn_re.search(line)
            if not qm:
                continue
            qn = qm.group(1)
            # A parte após SET
            set_part = line.split(" SET ", 1)[1] if " SET " in line else ""
            props = {}
            for pm in prop_re.finditer(set_part):
                key = pm.group(1)
                value = pm.group(2)
                # Desfazer escaping Cypher original (\\ -> \, \' -> ')
                value = value.replace("\\'", "'").replace("\\\\", "\\")
                props[key] = value
            rows[qn] = props
    return rows


def build_member_rows(create_path: str, update_path: str):
    pairs = parse_create_pairs(create_path)
    updates = parse_update_rows(update_path)
    rows = []
    for qn, cqn in pairs.items():
        props = updates.get(qn, {})
        # Incluir cqn numa propriedade auxiliar
        props["_cqn"] = cqn or props.get("parentQualifiedName", "")
        rows.append((qn, props))
    return rows


def emit_create_batches(rows, batch_size: int, roadmap: str):
    total = len(rows)
    print(f"Creating {total} member nodes/edges in batches of {batch_size}", file=sys.stderr)
    for start in range(0, total, batch_size):
        batch = rows[start : start + batch_size]
        maps = []
        for qn, props in batch:
            cqn = props.pop("_cqn")
            maps.append(f"{{qualifiedName:'{cypher_escape(qn)}', classQualifiedName:'{cypher_escape(cqn)}'}}")
        query = (
            "UNWIND ["
            + ", ".join(maps)
            + "] AS row "
            "MERGE (m:Method {qualifiedName:row.qualifiedName}) "
            "MERGE (c:Class {qualifiedName:row.classQualifiedName}) "
            "MERGE (c)-[:DECLARES]->(m)"
        )
        run_rmp("create", roadmap, query, start // batch_size + 1, (total + batch_size - 1) // batch_size)


def emit_update_batches(rows, batch_size: int, roadmap: str):
    total = len(rows)
    print(f"Updating {total} member nodes in batches of {batch_size}", file=sys.stderr)
    # Determinar todas as propriedades presentes para manter ordem estável
    all_props = set()
    for _, props in rows:
        all_props.update(props.keys())
    all_props.discard("_cqn")
    all_props = sorted(all_props)

    for start in range(0, total, batch_size):
        batch = rows[start : start + batch_size]
        assignments = []
        for qn, props in batch:
            escaped_qn = cypher_escape(qn)
            row_assigns = [f"qualifiedName:'{escaped_qn}'"]
            for key in all_props:
                if key in props:
                    row_assigns.append(f"{key}:'{cypher_escape(props[key])}'")
                else:
                    row_assigns.append(f"{key}:null")
            assignments.append("{" + ", ".join(row_assigns) + "}")
        set_clauses = ", ".join(f"m.{p}=row.{p}" for p in all_props)
        query = (
            "UNWIND ["
            + ", ".join(assignments)
            + "] AS row "
            f"MATCH (m:Method {{qualifiedName:row.qualifiedName}}) "
            f"SET {set_clauses}"
        )
        run_rmp("update", roadmap, query, start // batch_size + 1, (total + batch_size - 1) // batch_size)


def run_rmp(mode: str, roadmap: str, query: str, idx: int, total: int):
    cmd = ["rmp", "graph", mode, "-r", roadmap]
    result = subprocess.run(cmd, input=query, text=True, capture_output=True)
    if result.returncode != 0:
        print(f"\nBATCH {idx}/{total} FAILED (exit {result.returncode})", file=sys.stderr)
        print(result.stderr, file=sys.stderr)
        print("QUERY:\n" + query[:500], file=sys.stderr)
        sys.exit(1)
    out = result.stdout.strip()
    if idx % 10 == 0 or idx == total:
        print(f"  {idx}/{total} done", file=sys.stderr)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("roadmap")
    parser.add_argument("--batch-size", type=int, default=500)
    parser.add_argument(
        "--members",
        nargs="*",
        default=["methods", "constructors", "fields"],
        help="Tipos de membros a carregar",
    )
    args = parser.parse_args()

    base = "/tmp/lucene_kg_enrich"
    for kind in args.members:
        create_path = f"{base}/{kind}_create.cypher"
        update_path = f"{base}/{kind}_update.cypher"
        rows = build_member_rows(create_path, update_path)
        if not rows:
            print(f"No rows for {kind}; skipping.", file=sys.stderr)
            continue
        print(f"\nLoading {kind}: {len(rows)} members", file=sys.stderr)
        emit_create_batches(rows.copy(), args.batch_size, args.roadmap)
        emit_update_batches(rows, args.batch_size, args.roadmap)
    print("All member loads completed.", file=sys.stderr)


if __name__ == "__main__":
    main()
