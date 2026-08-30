#!/usr/bin/env python3
"""
Extrai a estrutura do crate Rucene (ficheiros, structs, traits, enums, funcoes
publicas, testes, impl-blocks e dependencias derivadas dos `use`) e emite um
ficheiro JSON com o levantamento completo.

E o equivalente Rucene do `extract_lucene_kg.py`: mesma abordagem (varrimento
por regex sobre texto mascarado, sem dependencias externas) e mesmas convencoes
de proveniencia (`--commit` / `--date`).

O carregamento para o grafo e feito por `load_rucene_kg.py`.

Uso:
    python3 tools/kg/extract_rucene_kg.py \
        --source-root /path/to/rucene \
        --commit <sha> --date <YYYY-MM-DD> \
        --output /tmp/rucene_kg/survey.json
"""

import argparse
import json
import os
import re
import sys
from pathlib import Path

CRATE = "rucene"

# --------------------------------------------------------------------------
# Mascaramento: substitui comentarios, strings e chars por espacos, preservando
# posicoes e mudancas de linha, para que a contagem de chavetas e as regexes
# nao sejam confundidas pelo conteudo deles.
# --------------------------------------------------------------------------


def mask(text: str) -> str:
    out = list(text)
    i = 0
    n = len(text)
    while i < n:
        c = text[i]
        # comentario de linha
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            j = i
            while j < n and text[j] != "\n":
                out[j] = " "
                j += 1
            i = j
            continue
        # comentario de bloco (aninhado, como em Rust)
        if c == "/" and i + 1 < n and text[i + 1] == "*":
            depth = 0
            j = i
            while j < n:
                if text[j] == "/" and j + 1 < n and text[j + 1] == "*":
                    depth += 1
                    out[j] = out[j + 1] = " "
                    j += 2
                    continue
                if text[j] == "*" and j + 1 < n and text[j + 1] == "/":
                    depth -= 1
                    out[j] = out[j + 1] = " "
                    j += 2
                    if depth == 0:
                        break
                    continue
                if text[j] != "\n":
                    out[j] = " "
                j += 1
            i = j
            continue
        # raw string: r"..." / r#"..."# / br##"..."##
        m = re.match(r'(?:b?r)(#*)"', text[i : i + 16])
        if m and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            hashes = m.group(1)
            close = '"' + hashes
            start = i + m.end()
            end = text.find(close, start)
            if end == -1:
                end = n
            for k in range(i, min(end + len(close), n)):
                if text[k] != "\n":
                    out[k] = " "
            i = min(end + len(close), n)
            continue
        # string normal (inclui b"...")
        if c == '"':
            out[i] = " "
            j = i + 1
            while j < n:
                if text[j] == "\\":
                    out[j] = " "
                    if j + 1 < n:
                        out[j + 1] = " "
                    j += 2
                    continue
                if text[j] == '"':
                    out[j] = " "
                    j += 1
                    break
                if text[j] != "\n":
                    out[j] = " "
                j += 1
            i = j
            continue
        # char literal vs lifetime: 'a' e char, 'a e lifetime
        if c == "'":
            m2 = re.match(r"'(?:\\.|[^\\'])'", text[i : i + 8])
            if m2:
                for k in range(i, i + m2.end()):
                    out[k] = " "
                i += m2.end()
                continue
            i += 1  # lifetime: deixa passar
            continue
        i += 1
    return "".join(out)


# --------------------------------------------------------------------------
# Regexes de itens. Todas ancoradas ao inicio da linha: o crate e formatado com
# `cargo fmt`, logo cada item comeca na sua propria linha.
# --------------------------------------------------------------------------

VIS = r"(?:pub(?:\s*\([^)]*\))?\s+)?"

RE_MOD = re.compile(r"^[ \t]*" + VIS + r"mod\s+([A-Za-z_][A-Za-z0-9_]*)", re.M)
RE_TYPE = re.compile(
    r"^[ \t]*"
    + VIS
    + r"(?:unsafe\s+)?(struct|enum|union|trait)\s+([A-Za-z_][A-Za-z0-9_]*)",
    re.M,
)
RE_IMPL = re.compile(r"^[ \t]*(?:unsafe\s+)?impl\b", re.M)
RE_ALIAS = re.compile(
    r"^[ \t]*" + VIS + r"type\s+([A-Za-z_][A-Za-z0-9_]*)", re.M
)
RE_FN = re.compile(
    r"^[ \t]*(?P<vis>pub(?:\s*\([^)]*\))?\s+)?"
    r"(?:default\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?"
    r'(?:extern\s+"[^"]*"\s+)?fn\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)',
    re.M,
)
RE_USE = re.compile(r"^[ \t]*(?:pub(?:\s*\([^)]*\))?\s+)?use\s+", re.M)
RE_ATTR_TEST = re.compile(r"#\[\s*(?:test|tokio::test|bench)\b")
RE_CFG_TEST = re.compile(r"#\[\s*cfg\s*\(\s*test\s*\)\s*\]")


def strip_generics(s: str) -> str:
    """Remove parametros genericos equilibrando <>."""
    out = []
    depth = 0
    for ch in s:
        if ch == "<":
            depth += 1
        elif ch == ">":
            if depth:
                depth -= 1
            continue
        if depth == 0:
            out.append(ch)
    return "".join(out)


def base_type_name(s: str) -> str:
    """Reduz uma expressao de tipo ao nome simples do tipo."""
    s = strip_generics(s).strip()
    s = s.replace("&", " ").replace("'", " ").strip()
    s = re.sub(r"\b(?:dyn|mut|impl)\b", " ", s).strip()
    s = s.split("(")[0].split("[")[0].strip()
    if not s:
        return ""
    tok = s.split()[0]
    return tok.split("::")[-1].strip()


def module_path_for(rel: str):
    """Caminho de modulo do crate para um ficheiro `src/...`, ou None."""
    if not rel.startswith("src/") or not rel.endswith(".rs"):
        return None
    parts = rel[len("src/") : -len(".rs")].split("/")
    if parts == ["lib"]:
        return CRATE
    if parts[-1] == "mod":
        parts = parts[:-1]
    return "::".join([CRATE] + parts)


def parse_file(rel: str, text: str):
    """Devolve (types, fns, impls, uses) de um ficheiro Rust."""
    masked = mask(text)
    n = len(masked)

    # 1. Recolher todos os itens com a sua posicao no texto mascarado.
    items = []
    for m in RE_MOD.finditer(masked):
        items.append((m.start(), "mod", m.group(1), m))
    for m in RE_TYPE.finditer(masked):
        items.append((m.start(), m.group(1), m.group(2), m))
    for m in RE_IMPL.finditer(masked):
        items.append((m.start(), "impl", None, m))
    for m in RE_ALIAS.finditer(masked):
        items.append((m.start(), "alias", m.group(1), m))
    for m in RE_FN.finditer(masked):
        items.append((m.start(), "fn", m.group("name"), m))
    items.sort(key=lambda t: t[0])
    by_pos = {it[0]: it for it in items}
    positions = sorted(by_pos)

    # 2. Varrer o ficheiro mantendo a profundidade de chavetas e a pilha de
    #    escopos, registando cada item no momento em que abre corpo (`{`) ou
    #    termina sem corpo (`;`).
    types, fns, impls, mods = [], [], [], []
    scope = []
    pending = None
    depth = 0
    pi = 0
    i = 0

    def resolve_impl_head(entry, end_pos):
        head = masked[entry["start"]:end_pos]
        head = re.split(r"\bwhere\b", head)[0]
        head = head.split("impl", 1)[1] if "impl" in head else head
        head = strip_generics(head).strip()
        if re.search(r"\bfor\b", head):
            tr, ty = re.split(r"\bfor\b", head, maxsplit=1)
            entry["implTrait"] = base_type_name(tr)
            entry["name"] = base_type_name(ty)
        else:
            entry["implTrait"] = None
            entry["name"] = base_type_name(head)

    while i < n:
        if pi < len(positions) and positions[pi] == i:
            _, kind, name, m = by_pos[i]
            pending = {"kind": kind, "name": name, "m": m, "start": i}
            pi += 1
            i = m.end()
            continue
        ch = masked[i]
        if ch == "{":
            if pending is not None:
                entry = dict(pending)
                entry["depth"] = depth
                if entry["kind"] == "impl":
                    resolve_impl_head(entry, i)
                elif entry["kind"] == "mod":
                    pre = text[max(0, entry["start"] - 200):entry["start"]]
                    entry["test"] = bool(RE_CFG_TEST.search(pre)) or entry["name"] in ("tests", "test")
                ms = [s["name"] for s in scope if s["kind"] == "mod"]
                tst = any(s.get("test") for s in scope)
                in_fn = any(x["kind"] == "fn" for x in scope)
                if entry["kind"] in ("struct", "enum", "union", "trait") and not in_fn:
                    # tipos declarados dentro do corpo de uma funcao sao locais
                    # a essa funcao e nao fazem parte da estrutura do modulo
                    _record_type(types, entry, ms, tst)
                elif entry["kind"] == "fn":
                    _record_fn(fns, entry, scope, ms, tst, text)
                elif entry["kind"] == "mod" and not entry["test"] and not tst:
                    mods.append({
                        "name": entry["name"],
                        "modSuffix": list(ms),
                    })
                if entry["kind"] == "impl":
                    impls.append({
                        "type": entry["name"],
                        "trait": entry["implTrait"],
                        "modSuffix": list(ms),
                    })
                scope.append(entry)
                pending = None
            depth += 1
            i += 1
            continue
        if ch == "}":
            depth -= 1
            while scope and scope[-1]["depth"] >= depth:
                scope.pop()
            i += 1
            continue
        if ch == ";":
            if pending is not None:
                ms = [s["name"] for s in scope if s["kind"] == "mod"]
                tst = any(s.get("test") for s in scope)
                in_fn = any(x["kind"] == "fn" for x in scope)
                if pending["kind"] in ("struct", "union") and not in_fn:
                    _record_type(types, pending, ms, tst)
                elif pending["kind"] == "alias" and not in_fn:
                    # so os aliases de nivel de modulo: os que estao dentro de
                    # um `impl`/`trait` sao tipos associados, nao itens.
                    if not any(x["kind"] in ("impl", "trait") for x in scope):
                        _record_type(types, pending, ms, tst)
                elif pending["kind"] == "fn":
                    _record_fn(fns, pending, scope, ms, tst, text)
                pending = None
            i += 1
            continue
        if ch == "(" and pending is not None and pending["kind"] in ("struct", "union"):
            pd = 0
            j = i
            while j < n:
                if masked[j] == "(":
                    pd += 1
                elif masked[j] == ")":
                    pd -= 1
                    if pd == 0:
                        break
                j += 1
            i = j + 1
            continue
        i += 1

    return types, fns, impls, mods, extract_uses(masked)


def _record_type(acc, entry, mod_suffix, in_test):
    acc.append(
        {
            "name": entry["name"],
            "kind": entry["kind"],
            "modSuffix": list(mod_suffix),
            "visibility": "pub" if _is_pub(entry["m"].group(0)) else "private",
            "scope": "test" if in_test else "crate",
        }
    )


def _is_pub(decl: str) -> bool:
    return bool(re.match(r"^[ \t]*pub\b", decl))


def _attr_lines_before(text: str, pos: int) -> str:
    """Devolve as linhas de atributo/doc contiguas imediatamente antes de `pos`."""
    head = text[:pos]
    lines = head.split("\n")
    if lines and lines[-1].strip() == "":
        lines = lines[:-1]
    collected = []
    for line in reversed(lines):
        st = line.strip()
        if st.startswith("#[") or st.startswith("#!") or st.startswith("//"):
            collected.append(st)
            continue
        break
    return "\n".join(collected)


def _record_fn(acc, entry, scope, mod_suffix, in_test, text):
    own = own_kind = own_trait = None
    for s in reversed(scope):
        if s["kind"] == "fn":
            return  # funcao aninhada dentro de outra funcao
        if s["kind"] in ("impl", "trait"):
            own = s["name"]
            own_kind = s["kind"]
            own_trait = s.get("implTrait")
            break
    decl = entry["m"].group(0)
    is_pub = bool(entry["m"].group("vis"))
    has_test_attr = bool(RE_ATTR_TEST.search(_attr_lines_before(text, entry["start"])))

    if has_test_attr:
        kind = "test"
    elif own_kind == "trait":
        kind = "trait-method"
    elif own_kind == "impl" and own_trait:
        kind = "trait-impl-method"
    elif own_kind == "impl":
        kind = "method"
    else:
        kind = "function"

    # Entram no grafo: as funcoes livres, os metodos de blocos `impl` inerentes,
    # os metodos declarados num `trait` (API por definicao) e os testes -
    # publicos ou privados, porque um metodo privado pode ser um porte
    # carregado por si so. Ficam de fora os metodos de `impl Trait for Type`,
    # ja cobertos pela aresta IMPLEMENTS e pela declaracao no trait.
    if kind == "trait-impl-method":
        return

    acc.append(
        {
            "name": entry["name"],
            "kind": kind,
            "owner": own,
            "ownerKind": own_kind,
            "ownerTrait": own_trait,
            "modSuffix": list(mod_suffix),
            "visibility": "pub" if is_pub else "private",
            "scope": "test" if in_test else "crate",
            "signature": " ".join(decl.split()),
        }
    )


def extract_uses(masked: str):
    """Extrai os caminhos de cada declaracao `use`, expandindo grupos `{}`."""
    paths = []
    for m in RE_USE.finditer(masked):
        start = m.end()
        j = start
        depth = 0
        while j < len(masked):
            c = masked[j]
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
            elif c == ";" and depth == 0:
                break
            j += 1
        stmt = masked[start:j]
        paths.extend(expand_use(stmt.strip()))
    return paths


def expand_use(stmt: str):
    stmt = " ".join(stmt.split())
    stmt = re.sub(r"\s+as\s+[A-Za-z_][A-Za-z0-9_]*", "", stmt)
    out = []

    def walk(prefix: str, s: str):
        s = s.strip()
        if not s:
            return
        if s.startswith("{") and s.endswith("}"):
            for part in split_top(s[1:-1]):
                walk(prefix, part)
            return
        idx = s.find("{")
        if idx == -1:
            out.append((prefix + s).strip())
            return
        head = s[:idx]
        rest = s[idx:]
        # apanhar o grupo equilibrado
        depth = 0
        for k, c in enumerate(rest):
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    break
        group = rest[: k + 1]
        for part in split_top(group[1:-1]):
            walk(prefix + head, part)

    walk("", stmt)
    return [p for p in out if p and p != "*"]


def split_top(s: str):
    parts, depth, cur = [], 0, []
    for c in s:
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
        if c == "," and depth == 0:
            parts.append("".join(cur))
            cur = []
            continue
        cur.append(c)
    parts.append("".join(cur))
    return [p.strip() for p in parts if p.strip()]


# --------------------------------------------------------------------------


def test_crate_names(root: Path):
    """Le os nomes de `[[test]]` do Cargo.toml: path -> nome do crate de teste."""
    txt = (root / "Cargo.toml").read_text(encoding="utf-8")
    mapping = {}
    for m in re.finditer(
        r"\[\[test\]\]\s*\n\s*name\s*=\s*\"([^\"]+)\"\s*\n\s*path\s*=\s*\"([^\"]+)\"",
        txt,
    ):
        mapping[m.group(2)] = m.group(1)
    return mapping


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--source-root", default=".")
    ap.add_argument("--commit", required=True)
    ap.add_argument("--date", required=True)
    ap.add_argument("--output", required=True)
    args = ap.parse_args()

    root = Path(args.source_root).resolve()
    tmap = test_crate_names(root)

    rel_files = []
    for base in ("src", "tests"):
        for dirpath, _dirs, names in os.walk(root / base):
            for fn in sorted(names):
                if fn.endswith(".rs"):
                    rel_files.append(
                        str(Path(dirpath).joinpath(fn).relative_to(root)).replace(
                            os.sep, "/"
                        )
                    )
    rel_files.sort()

    # modulo -> ficheiro (apenas src/)
    mod_to_file = {}
    for rel in rel_files:
        mp = module_path_for(rel)
        if mp:
            mod_to_file[mp] = rel

    files, types, fns, impls, mods, deps = [], [], [], [], [], []

    for rel in rel_files:
        text = (root / rel).read_text(encoding="utf-8", errors="ignore")
        mp = module_path_for(rel)
        is_test = rel.startswith("tests/")
        crate_name = tmap.get(rel, Path(rel).stem) if is_test else CRATE
        base_path = mp if mp else crate_name
        ftypes, ffns, fimpls, fmods, fuses = parse_file(rel, text)

        files.append(
            {
                "path": rel,
                "name": Path(rel).name,
                "kind": "test" if is_test else "source",
                "modulePath": base_path,
                "crate": crate_name,
                "loc": text.count("\n") + 1,
                "types": len(ftypes),
                "fns": len(ffns),
            }
        )

        for t in ftypes:
            qn = "::".join([base_path] + t["modSuffix"] + [t["name"]])
            types.append(dict(t, file=rel, qualifiedName=qn))
        for f in ffns:
            owner = f["owner"]
            segs = [base_path] + f["modSuffix"]
            if owner:
                segs.append(owner)
            segs.append(f["name"])
            fns.append(dict(f, file=rel, qualifiedName="::".join(segs)))
        for im in fimpls:
            impls.append(dict(im, file=rel))
        for md in fmods:
            mods.append(
                {
                    "name": md["name"],
                    "file": rel,
                    "modulePath": "::".join([base_path] + md["modSuffix"] + [md["name"]]),
                }
            )

        # dependencias derivadas dos `use`
        seen = set()
        for p in fuses:
            target = resolve_use(p, base_path, mod_to_file, is_test)
            if target and target != rel and target not in seen:
                seen.add(target)
                deps.append({"from": rel, "to": target})

    out = {
        "commit": args.commit,
        "date": args.date,
        "crate": CRATE,
        "files": files,
        "types": types,
        "fns": fns,
        "impls": impls,
        "mods": mods,
        "deps": deps,
    }
    Path(args.output).parent.mkdir(parents=True, exist_ok=True)
    Path(args.output).write_text(json.dumps(out, indent=1), encoding="utf-8")
    print(
        f"files={len(files)} types={len(types)} fns={len(fns)} "
        f"impls={len(impls)} mods={len(mods)} deps={len(deps)}",
        file=sys.stderr,
    )


def resolve_use(path: str, base_path: str, mod_to_file: dict, is_test: bool):
    """Resolve um caminho `use` para o ficheiro que declara o modulo alvo."""
    segs = path.split("::")
    if not segs:
        return None
    head = segs[0]
    if head == "crate":
        segs = [CRATE] + segs[1:]
    elif head == "self":
        segs = base_path.split("::") + segs[1:]
    elif head == "super":
        parent = base_path.split("::")[:-1]
        rest = segs[1:]
        while rest and rest[0] == "super":
            parent = parent[:-1]
            rest = rest[1:]
        segs = parent + rest
    elif head == CRATE:
        pass
    else:
        return None  # crate externo
    for k in range(len(segs), 0, -1):
        cand = "::".join(segs[:k])
        if cand in mod_to_file:
            return mod_to_file[cand]
    return None


if __name__ == "__main__":
    main()
