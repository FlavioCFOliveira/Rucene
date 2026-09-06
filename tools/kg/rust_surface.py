#!/usr/bin/env python3
"""Parse the crate's Rust surface into the shape the port triage needs.

`extract_rucene_kg.py` already surveys the crate, but it deliberately blanks
`macro_rules!` bodies and records only the items the graph models. Triaging a
Java member against the port needs the opposite bias: every place a name could
legitimately live, including macro templates, trait declarations in other files,
private helpers, and the doc comment that states which Lucene member an item
discharges.

It is a scanner, not a compiler. Brace depth over masked text is enough to say
which `impl` or `trait` block a `fn` sits in, which is the only structural fact
the triage rules need.
"""
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from extract_rucene_kg import mask, strip_generics, base_type_name

RE_FN = re.compile(
    r"^[ \t]*(?P<vis>pub(?:\s*\([^)]*\))?\s+)?"
    r"(?:default\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?"
    r'(?:extern\s+"[^"]*"\s+)?fn\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)',
    re.M,
)
RE_IMPL_HEAD = re.compile(r"^[ \t]*(?:unsafe\s+)?impl(?![A-Za-z0-9_])", re.M)
RE_TRAIT_HEAD = re.compile(
    r"^[ \t]*(?:pub(?:\s*\([^)]*\))?\s+)?(?:unsafe\s+)?trait\s+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)", re.M)
RE_TYPE_DECL = re.compile(
    r"^[ \t]*(?:pub(?:\s*\([^)]*\))?\s+)?(?:unsafe\s+)?"
    r"(?P<kind>struct|enum|union)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)", re.M)
RE_ALIAS = re.compile(
    r"^[ \t]*(?:pub(?:\s*\([^)]*\))?\s+)?type\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)", re.M)
RE_CONST = re.compile(
    r"^[ \t]*(?:pub(?:\s*\([^)]*\))?\s+)?(?:const|static)\s+(?:mut\s+)?"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)", re.M)
RE_MACRO_RULES = re.compile(r"\bmacro_rules!\s*(?P<name>[A-Za-z_]\w*)\s*\{")
RE_DERIVE = re.compile(r"#\[\s*derive\s*\(([^)]*)\)\s*\]")
# A variant sits alone on its line inside the enum body, named like a type and
# followed by `,`, its payload, a discriminant, or nothing. Every other line in
# that body is an attribute, a doc comment or a lower-case field of a struct
# variant, none of which can match.
RE_VARIANT = re.compile(
    r"^[ \t]*(?P<name>[A-Z][A-Za-z0-9_]*)[ \t]*(?=[,({=]|$)", re.M)


def block_end(masked, open_pos):
    """Index just past the `}` matching the `{` at `open_pos`."""
    depth, j, n = 0, open_pos, len(masked)
    while j < n:
        if masked[j] == "{":
            depth += 1
        elif masked[j] == "}":
            depth -= 1
            if depth == 0:
                return j + 1
        j += 1
    return n


def next_brace(masked, start):
    """Index of the `{` that opens the item beginning at `start`, or None.

    Stops at `;` so a trait or impl with no body (`impl X for Y;` never occurs,
    but `type X = Y;` and a bodyless `fn f();` do) is not credited with the
    next unrelated block.
    """
    j, n = start, len(masked)
    while j < n:
        if masked[j] == "{":
            return j
        if masked[j] == ";":
            return None
        j += 1
    return None


def doc_before(lines, lineno):
    """The `///` / `#[doc]` block immediately above 1-based line `lineno`."""
    out, i = [], lineno - 2
    while i >= 0:
        s = lines[i].strip()
        if s.startswith("///"):
            out.append(s[3:].strip())
        elif s.startswith("#[doc") or s.startswith("#[") or s.startswith("]") \
                or s.startswith("concat!") or s.startswith('"') or s.startswith("//!"):
            out.append(s)
        elif not s:
            break
        else:
            break
        i -= 1
    return " ".join(reversed(out))


def impl_head(text_slice):
    """(self_type, trait_name_or_None) from the header text before the `{`."""
    head = strip_generics(text_slice)
    head = re.sub(r"\bwhere\b.*$", "", head, flags=re.S)
    head = head.replace("\n", " ").strip()
    head = re.sub(r"^\s*(?:unsafe\s+)?impl\s*", "", head)
    m = re.search(r"\bfor\b", head)
    if m:
        tr = base_type_name(head[:m.start()])
        ty = base_type_name(head[m.end():])
        return ty, tr
    return base_type_name(head), None


def parse(rel, text):
    masked = mask(text)
    lines = text.splitlines()
    lineno = lambda pos: text.count("\n", 0, pos) + 1

    impls, traits, macros = [], [], []
    covered = []          # (start, end) of impl / trait / macro bodies

    for m in RE_IMPL_HEAD.finditer(masked):
        ob = next_brace(masked, m.end())
        if ob is None:
            continue
        end = block_end(masked, ob)
        ty, tr = impl_head(text[m.start():ob])
        impls.append({"self": ty, "trait": tr, "start": ob, "end": end,
                      "line": lineno(m.start()), "fns": []})
        covered.append((m.start(), end))

    for m in RE_TRAIT_HEAD.finditer(masked):
        ob = next_brace(masked, m.end())
        if ob is None:
            continue
        end = block_end(masked, ob)
        supers = []
        colon = text[m.end():ob]
        if ":" in colon:
            supers = [base_type_name(p) for p in
                      strip_generics(colon.split(":", 1)[1]).split("+")]
        traits.append({"name": m.group("name"), "supers": [s for s in supers if s],
                       "start": ob, "end": end, "line": lineno(m.start()),
                       "doc": doc_before(lines, lineno(m.start())), "fns": []})
        covered.append((m.start(), end))

    for m in RE_MACRO_RULES.finditer(masked):
        ob = masked.index("{", m.start())
        end = block_end(masked, ob)
        macros.append({"name": m.group("name"), "start": ob, "end": end,
                       "fns": []})
        covered.append((m.start(), end))

    def owner_of(pos):
        best = None
        for c in impls + traits + macros:
            if c["start"] <= pos < c["end"]:
                if best is None or c["start"] > best["start"]:
                    best = c
        return best

    free = []
    for m in RE_FN.finditer(masked):
        rec = {"name": m.group("name"), "line": lineno(m.start()),
               "doc": doc_before(lines, lineno(m.start())),
               "pub": bool(m.group("vis"))}
        own = owner_of(m.start())
        (own["fns"] if own else free).append(rec)

    types = {}
    for m in list(RE_TYPE_DECL.finditer(masked)) + list(RE_ALIAS.finditer(masked)):
        ln = lineno(m.start())
        # derives sit in the attribute block directly above the declaration
        i, ders = ln - 2, []
        while i >= 0 and (lines[i].strip().startswith("#[")
                          or lines[i].strip().startswith("///")
                          or lines[i].strip().startswith("//")):
            for g in RE_DERIVE.findall(lines[i]):
                ders += [d.strip() for d in g.split(",") if d.strip()]
            i -= 1
        # An enum variant is a declaration in its own right and carries its own
        # doc comment, which is where the port states what that one variant is
        # equivalent to. `LateInteractionRescorer.combine` is written on
        # `LateInteractionFallback::Zero`, because Java distinguishes the two
        # behaviours by subclassing and the port distinguishes them by variant.
        variants = []
        if m.re is RE_TYPE_DECL and m.group("kind") == "enum":
            ob = next_brace(masked, m.end())
            if ob is not None:
                end = block_end(masked, ob)
                for v in RE_VARIANT.finditer(masked, ob + 1, end - 1):
                    vln = lineno(v.start())
                    variants.append({"name": v.group("name"), "line": vln,
                                     "doc": doc_before(lines, vln)})
        types[m.group("name")] = {"line": ln, "derives": ders,
                                  "doc": doc_before(lines, ln),
                                  "kind": (m.group("kind")
                                           if m.re is RE_TYPE_DECL else "type"),
                                  "variants": variants}

    consts = {m.group("name") for m in RE_CONST.finditer(masked)}
    return {"file": rel, "impls": impls, "traits": traits, "macros": macros,
            "trait_docs": {t["name"]: t["doc"] for t in traits},
            "free": free, "types": types, "consts": consts}


def parse_tree(root):
    out = {}
    for p in sorted(Path(root, "src").rglob("*.rs")):
        rel = str(p.relative_to(root))
        out[rel] = parse(rel, p.read_text(encoding="utf-8", errors="replace"))
    return out


if __name__ == "__main__":
    import json
    tree = parse_tree(sys.argv[1] if len(sys.argv) > 1 else ".")
    nf = sum(len(f["free"]) for f in tree.values())
    ni = sum(len(i["fns"]) for f in tree.values() for i in f["impls"])
    nt = sum(len(t["fns"]) for f in tree.values() for t in f["traits"])
    nm = sum(len(m["fns"]) for f in tree.values() for m in f["macros"])
    nv = sum(len(t["variants"]) for f in tree.values()
             for t in f["types"].values())
    print(f"files {len(tree)}  free fns {nf}  impl fns {ni}  trait fns {nt}  "
          f"macro fns {nm}  impls {sum(len(f['impls']) for f in tree.values())}  "
          f"traits {sum(len(f['traits']) for f in tree.values())}  "
          f"enum variants {nv}")
