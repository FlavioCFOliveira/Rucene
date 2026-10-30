#!/usr/bin/env python3
"""Rust item -> item references, from rust-analyzer's SCIP index, into the KG.

The question this answers is "which items of the crate does production code
actually reach?" -- the Rust half of "which ported `pub` items have no
production reference" (`knowledge-model.md`, *Who reaches a ported item*).
The compiler front end answers it, not a regex: `rust-analyzer scip` resolves
every name in the crate -- paths, method calls, trait bounds, field types --
to the item it denotes, and this tool only attributes each resolved occurrence
to the item that contains it.

Phases
------
``extract``
    Runs ``rust-analyzer scip`` on the commit (the checkout itself when it is
    clean at that commit, else a ``git archive`` of it under ``--work``;
    ``--scip`` reuses an index) and writes one JSON line per (referrer,
    target) pair to ``--refs``. Each line counts the occurrences at
    **production** sites and at **test** sites separately.
``load``
    Replaces every ``REFERENCES`` edge this tool owns (``evidence``
    ``'rust-analyzer-scip'`` or ``'macro-body-lexical'``) with the extracted
    ones, stamped with the commit.

What is a reference
-------------------
* **Target**: an item the KG models -- a ``RustStruct``/``RustEnum``/
  ``RustTrait``/``RustAlias`` (identity ``name`` + ``file``) or a ``RustFn``
  (identity ``qualifiedName``) -- defined in a ``src/`` file. Fields, enum
  variants, associated consts, statics and locals are not KG items and are not
  targets.
* **Referrer**: the innermost KG item whose definition span (SCIP
  ``enclosing_range``) contains the occurrence. Spans that are not KG items are
  climbed through: a closure or a nested fn credits its enclosing fn; a method
  of an ``impl Trait for Type`` block (which the KG does not model) credits the
  block's self type. An occurrence inside no item -- a ``use`` declaration --
  is not a reference and is dropped. One inside a module-level ``const``/
  ``static`` or a module-level macro invocation credits the ``File``.
* **Self references are dropped**: an item's mention of itself (recursion, a
  type's own ``impl`` header) says nothing about who reaches it.
* **Production vs test**: a site is a *test* site when its file is not under
  ``src/`` (``tests/``, ``benches/``, ``examples/``), or when any span that
  contains it is gated by ``#[cfg(test)]`` or carries ``#[test]``/
  ``#[tokio::test]`` -- item attributes are inside SCIP's ``enclosing_range`` --
  or belongs to a ``mod tests``/``mod test`` (the convention
  ``extract_rucene_kg.py`` applies), or to an ``impl`` block gated by
  ``#[cfg(test)]``. Every other site is production.

Limitations (stated, not hidden)
--------------------------------
* rust-analyzer indexes with ``cfg(test)`` on and the default features, so code
  under ``#[cfg(not(test))]`` or a non-default feature is not indexed.
* A call that dispatches through a trait (``x.next_doc()`` on ``dyn Trait`` or
  a generic) resolves to the trait's method declaration, and one on a concrete
  type to that type's ``impl Trait for`` method, which is not a KG item: the
  call is attributed to whichever the compiler names, never to both.
* rust-analyzer emits no occurrence for code a ``macro_rules!`` body
  generates. What such a body names is read off its text (`macro_mentions`)
  and loaded as ``evidence: 'macro-body-lexical'`` edges from the file of each
  invocation; the recorded queries report them beside the compiler-resolved
  edges and never count them as a production reference.
* rust-analyzer 1.99 reports duplicate SCIP symbols for items of the
  integration-test crates (every crate of this package shares one symbol
  prefix). A reference is resolved to the definition in the same document
  first, then to the one under ``src/``; production sites only ever see
  ``src/`` definitions, so the production counts are unaffected.
"""
import argparse
import bisect
import json
import re
import subprocess
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import scipio  # noqa: E402
from extract_rucene_kg import base_type_name, blank_macro_bodies, mask  # noqa: E402
from kgio import client, counters_of, esc, format_counters, kg_target, read  # noqa: E402

EVIDENCE = "rust-analyzer-scip"
# rust-analyzer emits no occurrence for code a `macro_rules!` body generates,
# so what that code names is read off the body's text instead -- see
# `macro_mentions`. Its edges say so, and the recorded queries keep them
# apart from the compiler-resolved ones.
LEXICAL = "macro-body-lexical"
OWNED = (EVIDENCE, LEXICAL)
TYPE_LABELS = ("RustStruct", "RustEnum", "RustTrait", "RustAlias")
CRATE_PREFIX = "rust-analyzer cargo rucene "

RE_CFG_TEST = re.compile(r"#\[\s*cfg\s*\(\s*test\s*\)\s*\]")
RE_TEST_ATTR = re.compile(r"#\[\s*(?:tokio::)?test\b")
RE_IMPL = re.compile(r"^[ \t]*(?:unsafe\s+)?impl\b", re.M)
RE_USE = re.compile(r"^[ \t]*(?:pub(?:\s*\([^)]*\))?\s+)?use\s", re.M)
RE_MACRO_RULES = re.compile(r"\bmacro_rules!\s*[A-Za-z_]\w*\s*\{")


# ---------------------------------------------------------------------------
# SCIP symbols
# ---------------------------------------------------------------------------

def parse_descriptors(desc: str):
    """Split a SCIP descriptor string into `(name, suffix)` pairs.

    `suffix` is `/` namespace, `#` type, `.` term, `()` method (any
    disambiguator), `[]` type parameter, `!` macro, `:` meta, `(p)` parameter.
    Names may be backtick-escaped (a doubled backtick is a literal one).
    """
    out = []
    i, n = 0, len(desc)
    while i < n:
        c = desc[i]
        if c == "[":
            name, i = _name(desc, i + 1)
            out.append((name, "[]"))
            i += 1  # ']'
            continue
        if c == "(":
            j = desc.index(")", i)
            out.append((desc[i + 1:j], "(p)"))
            i = j + 1
            continue
        name, i = _name(desc, i)
        if i >= n:
            out.append((name, ""))
            break
        s = desc[i]
        if s == "(":
            j = desc.index(")", i)
            out.append((name, "()"))
            i = j + 2  # ')' and '.'
        else:
            out.append((name, s))
            i += 1
    return out


def _name(desc, i):
    if desc[i] == "`":
        j = i + 1
        buf = []
        while True:
            if desc[j] == "`":
                if j + 1 < len(desc) and desc[j + 1] == "`":
                    buf.append("`")
                    j += 2
                    continue
                return "".join(buf), j + 1
            buf.append(desc[j])
            j += 1
    j = i
    while j < len(desc) and (desc[j].isalnum() or desc[j] in "_$-+"):
        j += 1
    return desc[i:j], j


def crate_descriptors(symbol: str):
    """The descriptor string of a symbol of this package, or None."""
    if not symbol.startswith(CRATE_PREFIX):
        return None
    rest = symbol[len(CRATE_PREFIX):]
    _version, _, desc = rest.partition(" ")
    return desc


def item_of(symbol: str):
    """What a crate symbol denotes, as `(kind, owner, name, mods)`.

    `kind` is `type`, `fn`, or None for anything the KG does not model
    (namespace, field, variant, const, static, local, macro). For a fn,
    `owner` is the inherent impl's self type or the declaring trait, `None`
    for a free fn, and `impl-trait` marks a method of `impl Trait for Type`
    (not a KG item; `owner` is then the self type). `mods` is the module path.
    """
    desc = crate_descriptors(symbol)
    if desc is None:
        return None
    try:
        parts = parse_descriptors(desc)
    except (ValueError, IndexError):
        return None
    mods = []
    i = 0
    while i < len(parts) and parts[i][1] == "/":
        mods.append(parts[i][0])
        i += 1
    rest = parts[i:]
    if not rest:
        return None
    if len(rest) == 1:
        name, suf = rest[0]
        if suf == "#":
            return ("type", None, name, tuple(mods))
        if suf == "()":
            return ("fn", None, name, tuple(mods))
        return None
    head, hsuf = rest[0]
    if head == "impl" and hsuf == "#":
        params = [n for n, s in rest[1:] if s == "[]"]
        last, lsuf = rest[-1]
        if lsuf != "()" or not params:
            return None
        owner = base_type_name(params[0])
        if len(params) >= 2:
            return ("impl-trait", owner, last, tuple(mods))
        return ("fn", owner, last, tuple(mods))
    if hsuf == "#" and len(rest) == 2 and rest[1][1] == "()":
        return ("fn", head, rest[1][0], tuple(mods))
    return None


# ---------------------------------------------------------------------------
# Source regions the SCIP index does not delimit
# ---------------------------------------------------------------------------

def _match_brace(masked: str, i: int) -> int:
    depth = 0
    n = len(masked)
    while i < n:
        c = masked[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return n - 1


def _line_of(starts, offset):
    return bisect.bisect_right(starts, offset) - 1


def _attrs_before(lines, line_no):
    """The attribute lines just above `line_no` (masked text).

    Comments are masked to blanks, so a doc comment between attributes reads
    as an empty line and is stepped over; the walk stops at the first line
    that is neither blank nor an attribute.
    """
    out = []
    k = line_no - 1
    while k >= 0:
        st = lines[k].strip()
        if st.startswith("#") or st == "":
            out.append(st)
            k -= 1
            continue
        break
    return "\n".join(reversed(out))


def scan_regions(text: str):
    """Line regions of one file: `impl` blocks, `use` declarations and
    `macro_rules!` bodies.

    Returns a dict with `impls` -- `(start, header_end, end, self_type,
    test)`: the block's first line, the line of its `{`, its last line, the
    self type's base name, and whether `#[cfg(test)]` gates it -- `uses` and
    `macros` -- `(start, end)` line pairs -- and the masked `lines`. Lines
    are 0-based, as SCIP's.
    """
    masked = mask(text)
    starts = [0]
    for m in re.finditer("\n", masked):
        starts.append(m.end())
    lines = masked.split("\n")
    macros = []
    for m in RE_MACRO_RULES.finditer(masked):
        end = _match_brace(masked, masked.index("{", m.start()))
        macros.append((_line_of(starts, m.start()), _line_of(starts, end)))
    nomacro = blank_macro_bodies(masked)
    impls = []
    for m in RE_IMPL.finditer(nomacro):
        brace = nomacro.find("{", m.end())
        semi = nomacro.find(";", m.end())
        if brace == -1 or (semi != -1 and semi < brace):
            continue
        end = _match_brace(nomacro, brace)
        head = nomacro[m.end():brace]
        head = re.split(r"\bwhere\b", head)[0]
        head = re.sub(r"^\s*<", "<", head)
        if head.lstrip().startswith("<"):
            # drop the impl's own generic parameter list
            depth = 0
            h = head.lstrip()
            for k, ch in enumerate(h):
                depth += ch == "<"
                depth -= ch == ">"
                if depth == 0:
                    head = h[k + 1:]
                    break
        fors = re.split(r"\bfor\b", head, maxsplit=1)
        self_type = base_type_name(fors[-1])
        sl = _line_of(starts, m.start())
        test = bool(RE_CFG_TEST.search(_attrs_before(lines, sl)))
        impls.append((sl, _line_of(starts, brace), _line_of(starts, end), self_type, test))
    uses = []
    for m in RE_USE.finditer(nomacro):
        end = nomacro.find(";", m.end())
        uses.append((_line_of(starts, m.start()), _line_of(starts, end if end != -1 else m.end())))
    return {"impls": impls, "uses": uses, "macros": macros, "lines": lines}


# ---------------------------------------------------------------------------
# The KG's view of the crate
# ---------------------------------------------------------------------------

class KgIndex:
    """Lookup of KG Rust nodes by what a SCIP definition tells us.

    `types`: rows `(label, name, file, qualifiedName)`; `fns`: rows
    `(qualifiedName, file, owner, name)`; `files`: paths.

    A definition is matched by where it is written first -- `(name, file)` for
    a type, `(file, owner, name)` for a fn -- and then by qualified name, built
    from the SCIP module path. The second key is what finds a macro-generated
    item: the KG records it at the macro's invocation site, while SCIP places
    its definition on the name token inside the `macro_rules!` body. A last
    fallback takes the only fn of that name in the file, which is what an
    `impl Alias { .. }` needs: SCIP names the aliased type as the owner, the KG
    the alias.
    """

    def __init__(self, types, fns, files):
        self.types = {}
        self.type_qn = defaultdict(list)
        for label, name, file, qn in types:
            self.types[(name, file)] = label
            if qn:
                self.type_qn[qn].append((label, name, file))
        self.fns = defaultdict(list)
        self.fn_qn = set()
        self.fn_file_name = defaultdict(list)
        for qn, file, owner, name in fns:
            self.fns[(file, owner, name)].append(qn)
            self.fn_qn.add(qn)
            self.fn_file_name[(file, name)].append(qn)
        self.files = set(files)
        self.type_by_name = defaultdict(list)
        for (name, file), label in self.types.items():
            self.type_by_name[name].append((label, name, file))
        self.fn_by_owner_name = defaultdict(list)
        for (file, owner, name), qns in self.fns.items():
            self.fn_by_owner_name[(owner, name)] += [(file, q) for q in qns]

    def node(self, item, file):
        """The KG node key for a crate item defined in `file`, or None."""
        if item is None:
            return None
        kind, owner, name, mods = item
        qn = "::".join(["rucene", *mods, *([owner] if owner else []), name])
        if kind == "type":
            label = self.types.get((name, file))
            if label:
                return (label, name, file)
            hits = self.type_qn.get(qn, [])
            return hits[0] if len(hits) == 1 else None
        if kind == "fn":
            qns = self.fns.get((file, owner, name), [])
            if len(qns) > 1:
                qns = [q for q in qns if q == qn] or qns[:1]
            if qns:
                return ("RustFn", qns[0])
            if qn in self.fn_qn:
                return ("RustFn", qn)
            same = self.fn_file_name.get((file, name), [])
            return ("RustFn", same[0]) if len(same) == 1 else None
        return None


def kg_rust_index(roadmap):
    types = []
    for label in TYPE_LABELS:
        for name, file, qn in read(roadmap, f"MATCH (t:{label}) RETURN t.name, t.file, "
                                            "t.qualifiedName"):
            types.append((label, name, file, qn))
    fns = read(roadmap, "MATCH (f:RustFn) RETURN f.qualifiedName, f.file, f.owner, f.name")
    files = [r[0] for r in read(roadmap, "MATCH (f:File) WHERE f.language = 'Rust' RETURN f.path")]
    return KgIndex(types, fns, files)


# ---------------------------------------------------------------------------
# Attribution
# ---------------------------------------------------------------------------

def _contains(span, line, char):
    sl, sc, el, ec = span
    return (sl, sc) <= (line, char) < (el, ec) or (line == el and char == ec and sl != el)


def _span_is_test(span, def_pos, lines, name=None):
    """Whether a definition span is test-only: a `mod tests`/`mod test`, or
    its attributes (inside the span, before the name) gate it on
    `#[cfg(test)]` or mark it `#[test]`/`#[tokio::test]`."""
    if name in ("tests", "test"):
        return True
    dl, dc = def_pos
    attr = "\n".join(lines[span[0]:dl] + [lines[dl][:dc] if dl < len(lines) else ""])
    return bool(RE_CFG_TEST.search(attr) or RE_TEST_ATTR.search(attr))


def attribute_document(doc, text, kg, def_index):
    """Yield `(referrer, target, is_test)` for every reference in one file.

    `referrer` and `target` are KG node keys; `referrer` may be
    `("File", path)`. `def_index` maps a symbol to its definitions
    `[(path, item)]` over the whole index.
    """
    path = doc.path
    regions = scan_regions(text)
    lines = regions["lines"]
    file_is_test = not path.startswith("src/")

    defs = []  # (span, def_line, symbol, node, is_test)
    for o in doc.occurrences:
        if not o.is_definition or o.symbol.startswith("local ") or not o.enclosing:
            continue
        item = item_of(o.symbol)
        desc = crate_descriptors(o.symbol) or ""
        name = None
        if desc.endswith("/"):
            name = parse_descriptors(desc)[-1][0] if desc else None
        node = kg.node(item, path) if item and item[0] in ("type", "fn") else None
        is_test = _span_is_test(o.enclosing, (o.range[0], o.range[1]), lines, name)
        if desc.endswith("/") and not is_test:
            continue  # a non-test module span says nothing
        defs.append((o.enclosing, o.range[0], o.symbol, node, is_test, item))
    # Spans nest, so a sweep in position order with a stack of the open spans
    # gives each occurrence its chain of containing items, innermost last.
    defs.sort(key=lambda d: ((d[0][0], d[0][1]), (-d[0][2], -d[0][3])))
    refs = sorted((o for o in doc.occurrences
                   if not o.is_definition and not o.symbol.startswith("local ")),
                  key=lambda o: (o.range[0], o.range[1]))
    stack = []
    di = 0

    impls = regions["impls"]
    impl_self = {}

    for o in refs:
        pos = (o.range[0], o.range[1])
        while di < len(defs) and (defs[di][0][0], defs[di][0][1]) <= pos:
            d = defs[di]
            while stack and (stack[-1][0][2], stack[-1][0][3]) <= (d[0][0], d[0][1]):
                stack.pop()
            stack.append(d)
            di += 1
        while stack and (stack[-1][0][2], stack[-1][0][3]) <= pos:
            stack.pop()
        line, char = o.range[0], o.range[1]
        tnode = _target(o.symbol, path, kg, def_index)
        if tnode is None:
            continue
        chain = [d for d in stack if _contains(d[0], line, char)]
        is_test = file_is_test or any(d[4] for d in chain)
        imp = None
        for cand in impls:
            if cand[0] <= line <= cand[2]:
                if imp is None or cand[0] >= imp[0]:
                    imp = cand
        if imp is not None and imp[4]:
            is_test = True
        referrer = None
        for d in reversed(chain):
            if d[3] is not None:
                referrer = d[3]
                break
        if referrer is None and imp is not None:
            key = impl_self.get(imp)
            if key is None:
                key = _impl_self_node(doc, imp, kg, def_index)
                impl_self[imp] = key or False
            referrer = key or None
        if referrer is None:
            if any(u[0] <= line <= u[1] for u in regions["uses"]) and not chain:
                continue  # a `use` declaration is not a reference
            referrer = ("File", path)
        if referrer == tnode:
            continue
        yield referrer, tnode, is_test


def _target(symbol, path, kg, def_index):
    """The KG node a referenced symbol denotes, or None.

    The definition in the same document wins, then the one under `src/`
    (see the duplicate-symbol note in the module docstring). A symbol with no
    definition in the index is an item a macro generated: rust-analyzer
    reports references to it but no definition, so it is matched by its
    qualified name alone.
    """
    tgt_defs = def_index.get(symbol)
    if not tgt_defs:
        if not symbol.startswith(CRATE_PREFIX):
            return None
        item = item_of(symbol)
        return kg.node(item, None) if item and item[0] in ("type", "fn") else None
    tgt = next(((dp, di) for dp, di in tgt_defs if dp == path), None)
    if tgt is None:
        src_defs = [d for d in tgt_defs if d[0].startswith("src/")]
        tgt = src_defs[0] if src_defs else tgt_defs[0]
    if not tgt[0].startswith("src/"):
        return None
    return kg.node(tgt[1], tgt[0])


def _impl_self_node(doc, imp, kg, def_index):
    """The KG type an `impl` block's header names as its self type."""
    sl, hl, _el, self_type, _t = imp
    for o in doc.occurrences:
        if o.is_definition or not (sl <= o.range[0] <= hl):
            continue
        for dpath, ditem in def_index.get(o.symbol, []):
            if ditem and ditem[0] == "type" and ditem[2] == self_type:
                node = kg.node(ditem, dpath)
                if node:
                    return node
    return None


def build_def_index(docs):
    idx = defaultdict(list)
    for doc in docs:
        for o in doc.occurrences:
            if o.is_definition and o.symbol.startswith(CRATE_PREFIX):
                idx[o.symbol].append((doc.path, item_of(o.symbol)))
    return idx


def extract(docs, read_text, kg):
    """Aggregate `{(referrer, target): [prod, test]}` over every document."""
    def_index = build_def_index(docs)
    agg = defaultdict(lambda: [0, 0])
    for doc in docs:
        text = read_text(doc.path)
        if text is None:
            continue
        for ref, tgt, is_test in attribute_document(doc, text, kg, def_index):
            agg[(ref, tgt)][1 if is_test else 0] += 1
    return agg


RE_MACRO_NAME = re.compile(r"\bmacro_rules!\s*([A-Za-z_]\w*)\s*\{")
RE_PATH_CALL = re.compile(r"(?<![\w$])([A-Z]\w*)::([a-z_]\w*)\b")
RE_TYPE_NAME = re.compile(r"(?<![\w$:])([A-Z][A-Za-z0-9]*)\b")


def macro_mentions(docs, read_text, kg):
    """`{(("File", invocationFile), target): [prod, test]}` read off the text
    of every `macro_rules!` body of `src/`.

    A body is credited to the file of each of its invocations, because that
    is the module the generated code lives in. A name resolves first among
    the items of the invocation file -- a type by name, a fn by
    `Owner::name` -- then to the one item of that name in the crate; any other
    name is ambiguous and dropped. An invocation outside `src/` is a test
    site. This is lexical, not resolved: it says a generated body *names* the
    item, which is weaker than the compiler's word.
    """
    bodies = defaultdict(list)  # macro name -> [masked body]
    for doc in docs:
        if not doc.path.startswith("src/"):
            continue
        text = read_text(doc.path)
        if text is None or "macro_rules!" not in text:
            continue
        masked = mask(text)
        for m in RE_MACRO_NAME.finditer(masked):
            b = masked.index("{", m.start())
            bodies[m.group(1)].append(masked[b:_match_brace(masked, b) + 1])
    invocations = defaultdict(list)  # macro name -> [path]
    for doc in docs:
        for o in doc.occurrences:
            if o.is_definition or not o.symbol.startswith(CRATE_PREFIX) or not o.symbol.endswith("!"):
                continue
            name = re.split(r"[/ ]", o.symbol[:-1])[-1].strip("`")
            if name in bodies:
                invocations[name].append(doc.path)
    agg = defaultdict(lambda: [0, 0])
    for name, texts in bodies.items():
        body = "\n".join(texts)
        fns = Counter(RE_PATH_CALL.findall(body))
        types = Counter(RE_TYPE_NAME.findall(body))
        for path in invocations.get(name, []):
            hits = []
            for (owner, fname), n in fns.items():
                local = [q for f, q in kg.fn_by_owner_name.get((owner, fname), []) if f == path]
                pool = local or [q for _f, q in kg.fn_by_owner_name.get((owner, fname), [])]
                if len(pool) == 1:
                    hits.append((("RustFn", pool[0]), n))
            for tname, n in types.items():
                label = kg.types.get((tname, path))
                if label:
                    hits.append(((label, tname, path), n))
                elif len(kg.type_by_name.get(tname, [])) == 1:
                    hits.append((kg.type_by_name[tname][0], n))
            for tgt, n in hits:
                agg[(("File", path), tgt)][0 if path.startswith("src/") else 1] += n
    return agg


# ---------------------------------------------------------------------------
# rust-analyzer
# ---------------------------------------------------------------------------

def head_commit(root: Path):
    sha = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"],
                         capture_output=True, text=True, check=True).stdout.strip()
    date = subprocess.run(["git", "-C", str(root), "show", "-s", "--format=%cs", sha],
                          capture_output=True, text=True, check=True).stdout.strip()
    return sha, date


def prepare_source(root: Path, commit: str, work: Path) -> Path:
    """The directory to index: the checkout itself when it is clean at
    `commit`, else a `git archive` of `commit` under `work`."""
    head, _ = head_commit(root)
    dirty = subprocess.run(["git", "-C", str(root), "status", "--porcelain", "--", "src", "tests",
                            "Cargo.toml", "Cargo.lock"],
                           capture_output=True, text=True, check=True).stdout.strip()
    if commit == head and not dirty:
        return root
    dest = work / f"src-{commit[:12]}"
    if not dest.exists():
        dest.mkdir(parents=True)
        archive = subprocess.run(["git", "-C", str(root), "archive", commit],
                                 capture_output=True, check=True).stdout
        subprocess.run(["tar", "-x", "-C", str(dest)], input=archive, check=True)
    return dest


def run_scip(source: Path, out: Path, work: Path):
    target = work / "ra-target"
    cfg = work / "ra-config.json"
    cfg.write_text(json.dumps({"cargo": {"targetDir": str(target)}}))
    env = dict(__import__("os").environ, CARGO_TARGET_DIR=str(target))
    t = time.time()
    subprocess.run(["rust-analyzer", "scip", str(source), "--output", str(out),
                    "--config-path", str(cfg), "--num-threads", "4"],
                   env=env, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    print(f"  rust-analyzer scip: {time.time() - t:.0f}s -> {out}", file=sys.stderr)


# ---------------------------------------------------------------------------
# Load
# ---------------------------------------------------------------------------

def _match(var, key):
    if key[0] == "File":
        return f"MATCH ({var}:File {{path: row.{var}k}})", {f"{var}k": key[1]}
    if key[0] == "RustFn":
        return f"MATCH ({var}:RustFn {{qualifiedName: row.{var}k}})", {f"{var}k": key[1]}
    label, name, file = key
    return (f"MATCH ({var}:{label} {{name: row.{var}n, file: row.{var}f}})",
            {f"{var}n": name, f"{var}f": file})


def delete_owned(roadmap):
    total = {}
    while True:
        out = client(roadmap,
                     f"MATCH ()-[e:REFERENCES]->() WHERE e.evidence IN {list(OWNED)} "
                     "WITH e LIMIT 5000 DELETE e", label="delete REFERENCES")
        c = counters_of(out)
        if not c.get("relationshipsDeleted"):
            break
        for k, v in c.items():
            total[k] = total.get(k, 0) + v
    print(f"  retired REFERENCES {list(OWNED)}: {format_counters(total)}", file=sys.stderr)


def load(roadmap, rows, commit, date, batch=250):
    """`CREATE` one `REFERENCES` edge per row, after deleting the old ones.

    The edge type is owned whole by this tool, so a full replace keeps the
    graph equal to the extraction; `CREATE` (never a matched `MERGE`) is what
    keeps a pair already joined by `DECLARES`/`IMPLEMENTS` from taking the
    write on the wrong relationship.
    """
    delete_owned(roadmap)
    groups = defaultdict(list)
    for r in rows:
        src, dst = tuple(r["src"]), tuple(r["dst"])
        groups[(src[0], dst[0])].append((src, dst, r["prod"], r["test"], r.get("evidence", EVIDENCE)))
    created = 0
    for (sl, dl), items in sorted(groups.items()):
        for i in range(0, len(items), batch):
            chunk = items[i:i + batch]
            maps = []
            for src, dst, prod, test, ev in chunk:
                ms, ps = _match("a", src)
                md, pd = _match("b", dst)
                props = {**ps, **pd, "p": prod, "t": test, "ev": ev}
                maps.append("{" + ", ".join(f"{k}:{esc(v)}" for k, v in props.items()) + "}")
            q = (f"UNWIND [{', '.join(maps)}] AS row {ms} {md} "
                 f"CREATE (a)-[:REFERENCES {{evidence: row.ev, prodRefs: row.p, "
                 f"testRefs: row.t, gitCommit: {esc(commit)}, gitDate: {esc(date)}}}]->(b)")
            c = counters_of(client(roadmap, q, label=f"REFERENCES {sl}->{dl}"))
            created += c.get("relationshipsCreated", 0)
            if c.get("nodesCreated"):
                raise SystemExit(f"REFERENCES load created nodes: {c}")
    print(f"  REFERENCES created: {created} of {len(rows)} rows", file=sys.stderr)
    if created != len(rows):
        raise SystemExit(f"{len(rows) - created} REFERENCES rows matched no node pair")
    return created


def verify(roadmap, rows):
    n = read(roadmap, f"MATCH ()-[e:REFERENCES]->() WHERE e.evidence IN {list(OWNED)} "
                      "RETURN count(e), sum(e.prodRefs), sum(e.testRefs)")[0]
    want = (len(rows), sum(r["prod"] for r in rows), sum(r["test"] for r in rows))
    print(f"  verify: graph {tuple(n)} vs extraction {want}", file=sys.stderr)
    if tuple(n) != want:
        raise SystemExit("REFERENCES in the graph differ from the extraction")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("roadmap")
    ap.add_argument("--source-root", default=str(Path(__file__).resolve().parents[2]))
    ap.add_argument("--commit", help="commit to index (default: HEAD of --source-root)")
    ap.add_argument("--phase", choices=["extract", "load", "all"], default="all")
    ap.add_argument("--work", default=str(kg_target() / "no_caller"))
    ap.add_argument("--scip", help="an existing SCIP index to read instead of running rust-analyzer")
    ap.add_argument("--refs", help="the extracted references (JSONL)")
    args = ap.parse_args()

    root = Path(args.source_root)
    work = Path(args.work)
    work.mkdir(parents=True, exist_ok=True)
    head, _ = head_commit(root)
    commit = subprocess.run(["git", "-C", str(root), "rev-parse", args.commit or head],
                            capture_output=True, text=True, check=True).stdout.strip()
    date = subprocess.run(["git", "-C", str(root), "show", "-s", "--format=%cs", commit],
                          capture_output=True, text=True, check=True).stdout.strip()
    refs = Path(args.refs) if args.refs else work / f"rust_refs_{commit[:7]}.jsonl"

    if args.phase in ("extract", "all"):
        source = prepare_source(root, commit, work)
        scip = Path(args.scip) if args.scip else work / f"index_{commit[:7]}.scip"
        if not args.scip:
            run_scip(source, scip, work)
        t = time.time()
        docs = scipio.load(scip)
        kg = kg_rust_index(args.roadmap)

        def read_text(p):
            f = source / p
            return f.read_text(encoding="utf-8") if f.is_file() else None

        agg = extract(docs, read_text, kg)
        lex = macro_mentions(docs, read_text, kg)
        with refs.open("w") as fh:
            for ev, rows in ((EVIDENCE, agg), (LEXICAL, lex)):
                for (src, dst), (prod, test) in sorted(rows.items()):
                    if src == dst:
                        continue
                    fh.write(json.dumps({"src": list(src), "dst": list(dst),
                                         "prod": prod, "test": test, "evidence": ev,
                                         "commit": commit}) + "\n")
        print(f"  macro bodies: {len(lex)} lexical File->item pairs", file=sys.stderr)
        print(f"  extracted {len(agg)} referrer->target pairs "
              f"({sum(v[0] for v in agg.values())} production, "
              f"{sum(v[1] for v in agg.values())} test occurrences) "
              f"in {time.time() - t:.0f}s -> {refs}", file=sys.stderr)

    if args.phase in ("load", "all"):
        rows = [json.loads(line) for line in refs.open()]
        if rows and rows[0]["commit"] != commit:
            raise SystemExit(f"{refs} was extracted at {rows[0]['commit']}, not {commit}")
        load(args.roadmap, rows, commit, date)
        verify(args.roadmap, rows)


if __name__ == "__main__":
    main()
