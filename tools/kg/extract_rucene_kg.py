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
import subprocess
import sys
import tempfile
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


RE_MACRO_RULES = re.compile(r"\bmacro_rules!\s*[A-Za-z_]\w*\s*\{")


def blank_macro_bodies(masked: str) -> str:
    """Blank out every `macro_rules!` body, preserving length and line breaks.

    A `struct $name { ... }` inside a macro body is a template, not a
    declaration: no type of that name exists until the macro is invoked, and the
    invocation may produce several under different names. Reading them literally
    invented 12 types in `src/internal/hppc/macros.rs` alone, a file that
    declares nothing at all outside its macros. The real instances come from the
    macro-expansion pass below.
    """
    out = list(masked)
    for m in RE_MACRO_RULES.finditer(masked):
        i = masked.index("{", m.start())
        depth, j, n = 0, i, len(masked)
        while j < n:
            if masked[j] == "{":
                depth += 1
            elif masked[j] == "}":
                depth -= 1
                if depth == 0:
                    j += 1
                    break
            j += 1
        for k in range(m.start(), j):
            if out[k] != "\n":
                out[k] = " "
    return "".join(out)


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
    masked = blank_macro_bodies(mask(text))
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
                    _record_type(types, entry, ms, tst, masked)
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
                    _record_type(types, pending, ms, tst, masked)
                elif pending["kind"] == "alias" and not in_fn:
                    # so os aliases de nivel de modulo: os que estao dentro de
                    # um `impl`/`trait` sao tipos associados, nao itens.
                    if not any(x["kind"] in ("impl", "trait") for x in scope):
                        _record_type(types, pending, ms, tst, masked)
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


RE_CFG_TEST = re.compile(r"#\[cfg\(test\)\]")


def _record_type(acc, entry, mod_suffix, in_test, text=None):
    # `#[cfg(test)]` on the item itself makes it test-only just as surely as a
    # `#[cfg(test)] mod` around it. Reading only the enclosing module recorded
    # `DummyScorer` (src/util/hnsw/neighbor.rs) as production code.
    if not in_test and text is not None:
        in_test = bool(RE_CFG_TEST.search(_attr_lines_before(text, entry["start"])))
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
    ap.add_argument(
        "--expand",
        action="store_true",
        help="also record macro-generated types, by asking the compiler to "
        "expand the crate (needs the nightly toolchain)",
    )
    ap.add_argument(
        "--expanded-file",
        help="reuse a previously captured `-Zunpretty=expanded` dump instead "
        "of running the compiler again",
    )
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

    macro_types, macro_fns = [], []
    if args.expand or args.expanded_file:
        expanded = (
            Path(args.expanded_file).read_text(encoding="utf-8")
            if args.expanded_file
            else run_macro_expansion(root)
        )
        known = {(t["name"], t["file"]) for t in types}
        macro_types = macro_generated_types(root, mod_to_file, known, expanded)
        types.extend(macro_types)
        known_fns = {(f["name"], f["file"], f.get("owner")) for f in fns}
        macro_fns = macro_generated_fns(mod_to_file, known_fns, expanded)
        fns.extend(macro_fns)
        # A file's type count must stay consistent with the types recorded.
        per_file = {}
        for t in macro_types:
            per_file[t["file"]] = per_file.get(t["file"], 0) + 1
        for f in files:
            if f["path"] in per_file:
                f["types"] += per_file[f["path"]]

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
        f"impls={len(impls)} mods={len(mods)} deps={len(deps)} "
        f"macro-generated={len(macro_types)} types, {len(macro_fns)} fns",
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



# ---------------------------------------------------------------------------
# Macro-expansion pass
# ---------------------------------------------------------------------------
#
# The regex parser above reads literal source lines, so it cannot see a type
# that no source line declares. Rucene generates such types with `macro_rules!`
# in five places -- `internal::hppc` (the primitive containers Lucene's own code
# generator emits), `util::packed` (`BulkOperationPackedN`), `document`'s range
# fields and range doc-values fields, and `search::doc_values_iteration` -- so
# 67 real, `PORTS`-carrying types were invisible to the survey and the loader's
# audit reported them as stale nodes to delete.
#
# Rather than re-implement macro expansion (a guess, `CLAUDE.md` 7), this pass
# asks the compiler: `cargo +nightly rustc --lib -- -Zunpretty=expanded` prints
# the crate with every macro expanded. The types found only there are added with
# `origin: "macro"`, which keeps the graph honest about why a node exists that
# no literal declaration backs.
#
# The pass never overrides the literal survey: it only adds names the literal
# parser did not already record for the same file. Expansion is compiled with
# `cfg(test)` off, so it contributes production types only.

RE_EXP_MOD = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_]\w*)\s*\{")
RE_EXP_TYPE = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(struct|enum|trait|union)\s+([A-Za-z_]\w*)"
)
RE_EXP_IMPL = re.compile(r"^\s*(?:unsafe\s+)?impl\b")
RE_EXP_TRAIT = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?trait\s")
RE_EXP_FN = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?"
    r"(?:const\s+|async\s+|unsafe\s+|extern\s+\"[^\"]*\"\s+)*fn\s"
)


def run_macro_expansion(root: Path) -> str:
    """Expanded crate source, straight from the compiler."""
    target = Path(tempfile.gettempdir()) / "rucene-kg-expand-target"
    proc = subprocess.run(
        ["cargo", "+nightly", "rustc", "--lib", "--", "-Zunpretty=expanded"],
        cwd=str(root),
        capture_output=True,
        text=True,
        env=dict(os.environ, CARGO_TARGET_DIR=str(target)),
    )
    if proc.returncode != 0:
        raise SystemExit(
            "macro expansion failed (needs the nightly toolchain):\n"
            + proc.stderr[-2000:]
        )
    return proc.stdout


def parse_expanded(text: str):
    """Module-level types and functions of the expanded crate.

    Scans the masked text brace by brace and classifies each block from the
    *header* that precedes its `{` -- the text since the last `;`, `{` or `}`.
    Reading the header rather than the opening line is what makes this correct
    for the multi-line `impl<V> Clone for CharArrayMap<V> where ... {` headers
    the expander emits: matching only the opening line mistook 348 derive- and
    trait-impl methods for free functions.

    Returns `(types, fns)` where `types` is `(module_path, name, kind)` and
    `fns` is `(module_path, name, owner, signature)`. Anything inside a `fn`, a
    `trait` or an `impl Trait for Type` body is skipped, matching what the
    literal parser models; inherent `impl Type` blocks yield their methods with
    `owner` set.
    """
    # `macro_rules!` definitions survive `-Zunpretty=expanded` verbatim, so the
    # templates inside them have to be blanked here too, exactly as for a source
    # file. Otherwise the scan reads `$name` as a type and as a method owner.
    masked = blank_macro_bodies(mask(text))
    n = len(masked)
    stack = []  # (kind, name)
    types, fns = [], []
    header_start = 0
    i = 0

    def mods():
        return "::".join([CRATE] + [nm for k, nm in stack if k == "mod"])

    def opaque():
        return any(k in ("fn", "trait", "impl") for k, _ in stack)

    def owner():
        return next((nm for k, nm in reversed(stack) if k == "inherent"), None)

    def classify(header):
        h = " ".join(header.split())
        m = re.search(r"\bmod\s+([A-Za-z_]\w*)\s*$", h)
        if m:
            return ("mod", m.group(1))
        m = re.search(r"\b(struct|enum|union)\s+([A-Za-z_]\w*)", h)
        if m and " fn " not in h:
            return ("type", m.group(2))
        if re.search(r"\btrait\s+[A-Za-z_]", h) and " fn " not in h:
            return ("trait", "")
        if re.search(r"\bfn\s+[A-Za-z_]", h):
            return ("fn", "")
        m = re.match(r"^(?:.*\s)?impl(?:<.*?>)?\s+(.*)$", h)
        if m and re.search(r"\bimpl\b", h):
            rest = m.group(1)
            if re.search(r"\bfor\b", rest):
                return ("impl", "")
            base = re.split(r"[<\s{]", rest.strip(), 1)[0]
            return ("inherent", base.split("::")[-1].strip())
        return ("block", "")

    while i < n:
        c = masked[i]
        if c in ";}":
            if c == ";" and not opaque() and not owner():
                # `pub struct BulkOperationPacked1;` -- a unit struct opens no
                # brace, so it has to be caught here, where the module context
                # the surrounding scan maintains is still available.
                h = " ".join(masked[header_start:i].split())
                # The header still carries the item's attributes -- masking
                # blanks a `#[doc = "..."]` string but not its brackets -- so
                # anchor on the end of the header, not on its start.
                mu = re.search(
                    r"\b(struct)\s+([A-Za-z_]\w*)\s*(?:<[^>]*>)?"
                    r"\s*(?:\([^;]*\))?\s*$",
                    h,
                )
                if mu:
                    types.append((mods(), mu.group(2), "struct"))
            if c == "}" and stack:
                stack.pop()
            header_start = i + 1
        elif c == "{":
            header = masked[header_start:i]
            kind, name = classify(header)
            if kind == "type" and not opaque() and not owner():
                m = re.search(r"\b(struct|enum|union)\s+([A-Za-z_]\w*)", " ".join(header.split()))
                types.append((mods(), name, m.group(1)))
            elif kind == "trait" and not opaque() and not owner():
                m = re.search(r"\btrait\s+([A-Za-z_]\w*)", " ".join(header.split()))
                if m:
                    types.append((mods(), m.group(1), "trait"))
            elif kind == "fn" and not opaque():
                m = re.search(r"\bfn\s+([A-Za-z_]\w*)", " ".join(header.split()))
                if m:
                    fns.append((mods(), m.group(1), owner(), " ".join(header.split())))
            stack.append((kind, name))
            header_start = i + 1
        i += 1

    return types, fns


def macro_generated_types(root: Path, mod_to_file: dict, known: set, expanded: str):
    """Types the compiler sees that the literal parser did not record.

    `known` holds the (name, file) pairs the literal survey already produced.
    A module path is resolved to the file that declares it, walking up so that a
    type generated into an inline `mod` lands on the right file with the leftover
    segments kept as `modSuffix`.
    """
    added = []
    types_seen, _fns = parse_expanded(expanded)
    for mod_path, name, kind in types_seen:
        segs = mod_path.split("::")
        suffix = []
        rel = None
        while segs:
            cand = "::".join(segs)
            if cand in mod_to_file:
                rel = mod_to_file[cand]
                break
            suffix.insert(0, segs.pop())
        if rel is None or (name, rel) in known:
            continue
        known.add((name, rel))
        added.append(
            {
                "name": name,
                "kind": kind,
                "modSuffix": suffix,
                "visibility": "pub",
                "scope": "crate",
                "origin": "macro",
                "file": rel,
                "qualifiedName": "::".join([mod_path, name]),
            }
        )
    return added


def macro_generated_fns(mod_to_file: dict, known: set, expanded: str):
    """Free functions and inherent methods the compiler sees but no source line
    declares. Same rule and same `origin: "macro"` marker as the types above;
    `impl Trait for Type` bodies stay out, as everywhere else in this model."""
    added = []
    _types, fns = parse_expanded(expanded)
    for mod_path, name, owner, signature in fns:
        segs = mod_path.split("::")
        suffix = []
        rel = None
        while segs:
            cand = "::".join(segs)
            if cand in mod_to_file:
                rel = mod_to_file[cand]
                break
            suffix.insert(0, segs.pop())
        if rel is None:
            continue
        key = (name, rel, owner)
        if key in known:
            continue
        known.add(key)
        segments = [mod_path] + ([owner] if owner else []) + [name]
        added.append(
            {
                "name": name,
                "kind": "method" if owner else "function",
                "owner": owner,
                "modSuffix": suffix,
                "visibility": "pub" if signature.lstrip().startswith("pub") else "private",
                "scope": "crate",
                "origin": "macro",
                "signature": signature,
                "file": rel,
                "qualifiedName": "::".join(segments),
            }
        )
    return added

if __name__ == "__main__":
    main()
