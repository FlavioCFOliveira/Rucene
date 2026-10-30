#!/usr/bin/env python3
"""Lucene member -> member calls, from the 10.5.0 bytecode, into the KG.

The Lucene half of "who calls the Lucene counterpart of an unreferenced Rust
item" (`knowledge-model.md`, *Who reaches a ported item*). The answer comes
from the released `lucene-core-10.5.0.jar` -- the artifact Maven Central
publishes for the tag the port follows -- disassembled with the JDK's `javap`.
Bytecode is what javac resolved: every overload, every implicit `this.`,
every lambda and method reference is already bound to one member, which no
source regex can do.

What it writes
--------------
``CALLS``
    ``Method``|``Class`` -> ``Method``: the caller's code invokes the callee
    (``kind: 'invoke'``; ``invoke*`` instructions, and the method handles a
    lambda or a method reference binds) or reads/writes it (``kind:
    'field'``; ``get/putfield``, ``get/putstatic``). The callee is the member
    javac named, resolved to the class that declares it. ``sites`` counts the
    instructions; ``resolution`` says how the bytecode member was matched to
    the KG ``Method`` (see `match_member`). The caller is a ``Class`` when its
    code has no ``Method`` node: a static initialiser, or a member the source
    survey does not model.
``OVERRIDES``
    ``Method`` -> ``Method``: the source overrides or implements the target,
    an ancestor's declaration -- transitively, so one hop reaches every
    declaration a virtual call could dispatch from. Needed because a call
    names the member of the static receiver type: ``TermsEnum.next()`` is
    what ``MultiTermsEnum.next()``'s callers invoke.
``Method.overridesExternal``
    The JDK members a Lucene method overrides or implements
    (``java.io.Closeable#close()``), whose callers are not in the jar.

Lambda bodies (``lambda$name$N``) are credited to ``name``, and the code of an
anonymous or local class to the member that encloses it (the class file's
``EnclosingMethod`` attribute). Synthetic bridge methods are never callers;
their target is what makes a generic override an override.
"""
import argparse
import hashlib
import glob
import re
import shutil
import subprocess
import sys
import time
import zipfile
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from kgio import client, counters_of, esc, kg_target, read  # noqa: E402

EVIDENCE = "bytecode"
DEFAULT_JAR = Path.home() / ".m2/repository/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar"

PRIMS = {"B": "byte", "C": "char", "D": "double", "F": "float", "I": "int",
         "J": "long", "S": "short", "Z": "boolean", "V": "void"}


# ---------------------------------------------------------------------------
# Descriptors and signatures
# ---------------------------------------------------------------------------

def descriptor_params(desc: str):
    """Parameter types of a method descriptor, as simple names
    (`(Lorg/a/B$C;[I)V` -> `['C', 'int[]']`)."""
    out = []
    i = desc.index("(") + 1
    while desc[i] != ")":
        dims = 0
        while desc[i] == "[":
            dims += 1
            i += 1
        if desc[i] == "L":
            j = desc.index(";", i)
            name = desc[i + 1:j].rsplit("/", 1)[-1].rsplit("$", 1)[-1]
            i = j + 1
        else:
            name = PRIMS[desc[i]]
            i += 1
        out.append(name + "[]" * dims)
    return out


def _strip_generics(s: str) -> str:
    out, depth = [], 0
    for ch in s:
        if ch == "<":
            depth += 1
        elif ch == ">":
            depth -= 1
        elif depth == 0:
            out.append(ch)
    return "".join(out)


def _split_top(s: str):
    parts, depth, cur = [], 0, []
    for ch in s:
        if ch in "<(":
            depth += 1
        elif ch in ">)":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
    if "".join(cur).strip():
        parts.append("".join(cur))
    return parts


RE_ANNOT = re.compile(r"@[\w.]+(?:\([^)]*\))?")


def signature_params(sig: str):
    """Parameter types of a KG `Method.signature`, as simple names
    (`void f(final Map<K, V> m, int... xs)` -> `['Map', 'int[]']`)."""
    if "(" not in sig:
        return None
    inner = sig[sig.index("(") + 1:sig.rindex(")")] if ")" in sig else ""
    out = []
    for p in _split_top(inner):
        p = RE_ANNOT.sub(" ", p)
        p = _strip_generics(p).replace("...", "[] ")
        toks = [t for t in p.split() if t != "final"]
        if not toks:
            continue
        dims = 0
        if len(toks) >= 2:
            name = toks[-1]
            dims += name.count("[")
            toks = toks[:-1]
        t = "".join(toks)
        dims += t.count("[")
        base = t.replace("[", "").replace("]", "").rsplit(".", 1)[-1]
        out.append(base + "[]" * dims)
    return out


def _is_type_var(t: str) -> bool:
    base = t.replace("[]", "")
    return bool(re.fullmatch(r"[A-Z][A-Z0-9]?", base))


def match_member(cands, name, params, is_ctor=False):
    """The KG `Method`s a bytecode member `name(params)` is, and how matched.

    `cands` are `(qualifiedName, name, params-or-None)` of the class's KG
    members. Returns `(qualifiedNames, resolution)`:

    * `exact` -- same name and parameter types, a type variable on the KG side
      matching any erased type;
    * `synthetic` -- a constructor whose bytecode carries leading synthetic
      parameters (an inner class's outer instance, an enum's name and
      ordinal) matches once they are dropped;
    * `arity` -- the only same-name member with that many parameters;
    * `name` -- the only same-name member;
    * `ambiguous` -- several same-name, same-arity members, all returned.
    """
    same = [c for c in cands if c[1] == name]
    if not same:
        return [], None

    def fits(kg, bc):
        return kg is not None and len(kg) == len(bc) and all(
            k == b or _is_type_var(k) for k, b in zip(kg, bc))

    hit = [c[0] for c in same if fits(c[2], params)]
    if hit:
        return hit[:1] if len(hit) == 1 else hit, "exact"
    if is_ctor:
        for drop in (1, 2):
            hit = [c[0] for c in same if fits(c[2], params[drop:])]
            if hit:
                return hit, "synthetic"
    ar = [c[0] for c in same if c[2] is not None and len(c[2]) == len(params)]
    if len(ar) == 1:
        return ar, "arity"
    if len(same) == 1:
        return [same[0][0]], "name"
    if ar:
        return ar, "ambiguous"
    return [], None


# ---------------------------------------------------------------------------
# javap
# ---------------------------------------------------------------------------

RE_INVOKE = re.compile(
    r"^\s+\d+: (invoke\w+|[gp]et(?:field|static))\s+#\d+(?:,\s+\d+)?\s+// "
    r"(InterfaceMethod|Method|Field|InvokeDynamic) (.+)$")
RE_REF = re.compile(r'^(?:([\w/$]+)\.)?("?[\w$<>]+"?):(\S+)$')
RE_HEAD = re.compile(r"^(?:[a-z]+\s+)*(class|interface|enum|record)\s+([\w.$]+)")
RE_BSM = re.compile(r"^\s+(\d+): #\d+ REF_\w+ ")
RE_BSM_ARG = re.compile(r'^\s+#\d+ REF_(\w+) ([\w/$]+)\.("?[\w$<>]+"?):(\S+)$')


class ClassFile:
    __slots__ = ("name", "access_flags", "super", "interfaces", "methods", "fields",
                 "calls", "indy", "bsm", "enclosing")

    def __init__(self, name):
        self.name = name          # binary name with dots: org.apache.lucene.a.B$C
        self.access_flags = None  # the class's `flags:` line, e.g. "ACC_PUBLIC, ACC_SUPER"
        self.super = None
        self.interfaces = []
        self.methods = {}         # (name, desc) -> flags string
        self.fields = set()
        self.calls = defaultdict(lambda: defaultdict(int))  # (m, d) -> {(kind, owner, n, d): sites}
        self.indy = defaultdict(list)   # (m, d) -> [bootstrap index]
        self.bsm = {}             # index -> [(owner, name, desc)]
        self.enclosing = None     # (class, method-name or None)


def _dots(internal):
    return internal.replace("/", ".")


def parse_javap(text: str):
    """Every class of a `javap -c -p -v` dump."""
    classes = []
    cur = None
    member = None
    section = None
    bsm_idx = None
    lines = text.split("\n")
    i = 0
    while i < len(lines):
        line = lines[i]
        if line.startswith("Classfile "):
            cur = None
            member = None
            section = "header"
            i += 1
            continue
        if section == "header" and cur is None and RE_HEAD.match(line):
            head = _strip_generics(line)
            m = re.search(r"\b(?:class|interface|enum|record)\s+([\w.$]+)", head)
            cur = ClassFile(m.group(1))
            classes.append(cur)
            is_iface = re.search(r"\binterface\s", head) is not None
            ext = re.search(r"\bextends\s+(.+?)(?:\s+implements\s+|$)", head)
            imp = re.search(r"\bimplements\s+(.+)$", head)
            if is_iface and ext:
                cur.interfaces += [x.strip() for x in ext.group(1).split(",")]
            elif ext:
                cur.super = ext.group(1).strip()
            if imp:
                cur.interfaces += [x.strip() for x in imp.group(1).split(",")]
            i += 1
            continue
        if cur is None:
            i += 1
            continue
        if line.startswith("  flags: (") and cur.access_flags is None:
            cur.access_flags = line.split(")", 1)[-1].strip()
        if line.startswith("  super_class:") and cur.super is None:
            m = re.search(r"// (\S+)", line)
            if m:
                cur.super = _dots(m.group(1))
        elif line == "{":
            section = "body"
        elif line == "}" and section == "body":
            section = "attrs"
            member = None
        elif section == "body":
            if line.startswith("  ") and not line.startswith("   ") and line.rstrip().endswith(";"):
                member = ("pending", line.strip())
            elif line.startswith("    descriptor: ") and member and member[0] == "pending":
                desc = line.split(": ", 1)[1].strip()
                head = member[1]
                flags_line = lines[i + 1] if i + 1 < len(lines) else ""
                flags = flags_line.split(")", 1)[-1].strip() if "flags:" in flags_line else ""
                if desc.startswith("("):
                    if head.startswith("static {}"):
                        name = "<clinit>"
                    else:
                        pre = _strip_generics(head.split("(", 1)[0]).split()
                        name = pre[-1]
                        if name == cur.name:
                            name = "<init>"
                    member = (name, desc)
                    cur.methods[member] = flags
                else:
                    pre = _strip_generics(head.rstrip(";")).split()
                    cur.fields.add(pre[-1])
                    member = None
            elif member and member[0] != "pending":
                m = RE_INVOKE.match(line)
                if m:
                    op, kind, ref = m.groups()
                    if kind == "InvokeDynamic":
                        b = re.match(r"#(\d+):", ref)
                        if b:
                            cur.indy[member].append(int(b.group(1)))
                    else:
                        r = RE_REF.match(ref.strip())
                        if r:
                            owner = _dots(r.group(1)) if r.group(1) else cur.name
                            n = r.group(2).strip('"')
                            k = "field" if kind == "Field" else "invoke"
                            cur.calls[member][(k, owner, n, r.group(3))] += 1
        elif section == "attrs":
            if line.startswith("EnclosingMethod:"):
                m = re.match(r"EnclosingMethod: #\d+\.#(\d+)\s+// (\S+)", line)
                if m:
                    target = m.group(2)
                    if m.group(1) == "0":
                        cur.enclosing = (target, None)
                    else:
                        cls, _, meth = target.rpartition(".")
                        cur.enclosing = (cls, meth)
            elif line.startswith("BootstrapMethods:"):
                section = "bsm"
        elif section == "bsm":
            m = RE_BSM.match(line)
            if m:
                bsm_idx = int(m.group(1))
                cur.bsm[bsm_idx] = []
            elif not line.startswith(" "):
                section = "attrs"
                continue
            else:
                a = RE_BSM_ARG.match(line)
                if a and bsm_idx is not None:
                    cur.bsm[bsm_idx].append((_dots(a.group(2)), a.group(3).strip('"'), a.group(4)))
        i += 1
    return classes


def run_javap(javap: str, jar: Path, names, chunk=150):
    out = []
    for k in range(0, len(names), chunk):
        res = subprocess.run([javap, "-c", "-p", "-v", "-cp", str(jar), *names[k:k + chunk]],
                             capture_output=True, text=True, check=False)
        if res.returncode != 0:
            raise SystemExit(f"javap failed: {res.stderr[:500]}")
        out.append(res.stdout)
    return "\n".join(out)


def run_javap_external(javap: str, names):
    """Declared methods of JDK types: `{class: (super, [interfaces], {(n, d)})}`."""
    res = subprocess.run([javap, "-p", "-s", *names], capture_output=True, text=True, check=False)
    info = {}
    cur = None
    pend = None
    for line in res.stdout.split("\n"):
        m = RE_HEAD.match(_strip_generics(line))
        if m:
            head = _strip_generics(line)
            is_iface = " interface " in f" {head} "
            ext = re.search(r"\bextends\s+(.+?)(?:\s+implements\s+|\s*\{|$)", head)
            imp = re.search(r"\bimplements\s+(.+?)(?:\s*\{|$)", head)
            sup = None
            ifs = []
            if is_iface and ext:
                ifs += [x.strip() for x in ext.group(1).split(",")]
            elif ext:
                sup = ext.group(1).strip()
            if imp:
                ifs += [x.strip() for x in imp.group(1).split(",")]
            cur = m.group(2)
            info[cur] = (sup, ifs, set())
            continue
        if cur and line.startswith("  ") and "(" in line and line.rstrip().endswith(";"):
            pend = _strip_generics(line.split("(", 1)[0]).split()[-1]
        elif cur and pend and line.strip().startswith("descriptor:"):
            info[cur][2].add((pend, line.split(":", 1)[1].strip()))
            pend = None
    return info


# ---------------------------------------------------------------------------
# Resolution
# ---------------------------------------------------------------------------

class Hierarchy:
    def __init__(self, classes, external=None):
        self.by_name = {c.name: c for c in classes}
        self.external = external or {}

    def supers(self, name):
        c = self.by_name.get(name)
        if c is not None:
            out = ([c.super] if c.super else []) + list(c.interfaces)
        elif name in self.external:
            sup, ifs, _ = self.external[name]
            out = ([sup] if sup else []) + list(ifs)
        else:
            out = []
        return [s for s in out if s and s != "java.lang.Object"] + (
            ["java.lang.Object"] if name != "java.lang.Object" else [])

    def ancestors(self, name):
        seen, order, todo = set(), [], list(self.supers(name))
        while todo:
            s = todo.pop(0)
            if s in seen:
                continue
            seen.add(s)
            order.append(s)
            todo.extend(self.supers(s))
        return order

    def declarer(self, owner, name, desc, field=False):
        """The class in the jar that declares the member javac named."""
        for cls in [owner] + self.ancestors(owner):
            c = self.by_name.get(cls)
            if c is None:
                continue
            if field and name in c.fields:
                return cls
            if not field and (name, desc) in c.methods:
                return cls
        return None


def caller_of(cf, member, hier):
    """`(class, name, desc)` the code of `member` is credited to: a lambda
    body to the member that wrote it, an anonymous or local class's code to
    its enclosing member. `name` is None when only the class is known."""
    cls, (name, desc) = cf.name, member
    seen = 0
    while seen < 10:
        seen += 1
        m = re.match(r"lambda\$(.+)\$\d+$", name or "")
        if m:
            inner = m.group(1)
            name = {"new": "<init>", "static": "<clinit>"}.get(inner, inner)
            desc = None
            continue
        c = hier.by_name.get(cls)
        if c is not None and re.search(r"\$\d", cls) and c.enclosing:
            cls, name = c.enclosing[0], c.enclosing[1]
            desc = None
            continue
        break
    return cls, name, desc


class KgMembers:
    """The KG `Method`s of each Lucene class: `(qn, name, kind, params)`."""

    def __init__(self, rows, classes):
        self.by_class = defaultdict(list)
        for qn, parent, name, kind, sig in rows:
            params = signature_params(sig or "") if kind in ("method", "constructor") else None
            self.by_class[parent].append((qn, name, kind, params))
        self.classes = set(classes)

    def resolve(self, cls, name, desc, field=False):
        members = self.by_class.get(cls, [])
        if field:
            hit = [m[0] for m in members if m[1] == name and m[2] in ("field", "enumConstant")]
            return (hit[:1], "exact") if hit else ([], None)
        is_ctor = name == "<init>"
        simple = cls.rsplit(".", 1)[-1].rsplit("$", 1)[-1]
        want = simple if is_ctor else name
        kinds = ("constructor",) if is_ctor else ("method",)
        cands = [(m[0], m[1], m[3]) for m in members if m[2] in kinds]
        if desc is None:
            hit = [c[0] for c in cands if c[1] == want]
            if len(hit) == 1:
                return hit, "name"
            return (hit, "ambiguous") if hit else ([], None)
        return match_member(cands, want, descriptor_params(desc), is_ctor)


def build(classes, hier, kg, external_methods):
    """`(calls, overrides, overrides_external, stats)`.

    `calls`: `{(callerLabel, callerKey, calleeQn, kind): [sites, resolution]}`.
    """
    stats = defaultdict(int)
    calls = {}
    for cf in classes:
        bridges = {m for m, f in cf.methods.items() if "ACC_BRIDGE" in f}
        for member in set(cf.calls) | set(cf.indy):
            if member in bridges:
                continue
            ccls, cname, cdesc = caller_of(cf, member, hier)
            if cname is None or cname == "<clinit>":
                caller = ("Class", ccls) if ccls in kg.classes else None
            else:
                qns, _ = kg.resolve(ccls, cname, cdesc)
                caller = ("Method", qns[0]) if qns else (
                    ("Class", ccls) if ccls in kg.classes else None)
            if caller is None:
                stats["caller-unmodelled"] += 1
                continue
            targets = defaultdict(int)
            for (kind, owner, n, d), sites in cf.calls.get(member, {}).items():
                targets[(kind, owner, n, d)] += sites
            for b in cf.indy.get(member, []):
                for owner, n, d in cf.bsm.get(b, []):
                    if n.startswith("lambda$"):
                        continue  # the body is a member of this class, credited above
                    targets[("invoke", owner, n, d)] += 1
            for (kind, owner, n, d), sites in targets.items():
                if not owner.startswith("org.apache.lucene."):
                    continue
                field = kind == "field"
                decl = hier.declarer(owner, n, d, field)
                if decl is None:
                    stats["callee-undeclared"] += sites
                    continue
                qns, how = kg.resolve(decl, n, None if field else d, field)
                if not qns:
                    stats["callee-unmodelled"] += sites
                    continue
                for qn in qns:
                    if caller == ("Method", qn):
                        continue  # recursion
                    key = (caller[0], caller[1], qn, kind)
                    if key in calls:
                        calls[key][0] += sites
                    else:
                        calls[key] = [sites, how]
                stats["sites-resolved"] += sites

    overrides = set()
    ext = defaultdict(set)
    for cf in classes:
        bridge_real = {}
        for (n, d), f in cf.methods.items():
            if "ACC_BRIDGE" in f:
                for (k, o, n2, d2) in cf.calls.get((n, d), {}):
                    if k == "invoke" and o == cf.name and n2 == n and d2 != d:
                        bridge_real[(n, d)] = (n2, d2)
        for (n, d), f in cf.methods.items():
            if n.startswith("<") or "ACC_STATIC" in f or "ACC_PRIVATE" in f or "ACC_SYNTHETIC" in f:
                continue
            sigs = [d] + [bd for (bn, bd), real in bridge_real.items() if real == (n, d)]
            mine, _ = kg.resolve(cf.name, n, d)
            if not mine:
                continue
            for anc in hier.ancestors(cf.name):
                ac = hier.by_name.get(anc)
                for sd in sigs:
                    if ac is not None:
                        af = ac.methods.get((n, sd))
                        if af is None or "ACC_PRIVATE" in af or "ACC_STATIC" in af:
                            continue
                        theirs, _ = kg.resolve(anc, n, sd)
                        for t in theirs:
                            if t not in mine:
                                overrides.add((mine[0], t))
                    elif (n, sd) in external_methods.get(anc, set()):
                        ext[mine[0]].add(f"{anc}#{n}({', '.join(descriptor_params(sd))})")
    return calls, overrides, ext, stats


# ---------------------------------------------------------------------------
# Load
# ---------------------------------------------------------------------------

def _delete_all(roadmap, rel):
    total = 0
    while True:
        c = counters_of(client(roadmap, f"MATCH ()-[e:{rel}]->() WHERE e.evidence = '{EVIDENCE}' "
                                        "WITH e LIMIT 5000 DELETE e", label=f"delete {rel}"))
        n = c.get("relationshipsDeleted", 0)
        total += n
        if not n:
            break
    print(f"  retired {rel}: {total}", file=sys.stderr)


def _unwind_create(roadmap, rows, head, label, batch=300):
    created = 0
    for i in range(0, len(rows), batch):
        chunk = rows[i:i + batch]
        maps = ", ".join("{" + ", ".join(f"{k}:{esc(v)}" for k, v in r.items()) + "}" for r in chunk)
        c = counters_of(client(roadmap, f"UNWIND [{maps}] AS row {head}", label=label))
        if c.get("nodesCreated"):
            raise SystemExit(f"{label} created nodes: {c}")
        created += c.get("relationshipsCreated", 0)
    return created


def top_level_visibility(classes):
    """`{class: 'public'|'package'}` for every top-level class (no `$`)."""
    return {c.name: "public" if "ACC_PUBLIC" in (c.access_flags or "") else "package"
            for c in classes if "$" not in c.name}


def load(roadmap, calls, overrides, ext, commit, date, visibility):
    _delete_all(roadmap, "CALLS")
    _delete_all(roadmap, "OVERRIDES")
    stamp = f"gitCommit: {esc(commit)}, gitDate: {esc(date)}"
    for label in ("Method", "Class"):
        rows = [{"a": k[1], "b": k[2], "k": k[3], "s": v[0], "r": v[1]}
                for k, v in calls.items() if k[0] == label]
        n = _unwind_create(
            roadmap, rows,
            f"MATCH (a:{label} {{qualifiedName: row.a}}) MATCH (b:Method {{qualifiedName: row.b}}) "
            f"CREATE (a)-[:CALLS {{evidence: '{EVIDENCE}', kind: row.k, sites: row.s, "
            f"resolution: row.r, {stamp}}}]->(b)", f"CALLS from {label}")
        print(f"  CALLS from {label}: {n} created of {len(rows)}", file=sys.stderr)
        if n != len(rows):
            raise SystemExit(f"{len(rows) - n} CALLS rows matched no node pair")
    rows = [{"a": a, "b": b} for a, b in sorted(overrides)]
    n = _unwind_create(
        roadmap, rows,
        "MATCH (a:Method {qualifiedName: row.a}) MATCH (b:Method {qualifiedName: row.b}) "
        f"CREATE (a)-[:OVERRIDES {{evidence: '{EVIDENCE}', {stamp}}}]->(b)", "OVERRIDES")
    print(f"  OVERRIDES: {n} created of {len(rows)}", file=sys.stderr)
    if n != len(rows):
        raise SystemExit(f"{len(rows) - n} OVERRIDES rows matched no node pair")
    client(roadmap, "MATCH (m:Method) WHERE m.overridesExternal IS NOT NULL "
                    "REMOVE m.overridesExternal", label="clear overridesExternal")
    rows = [{"q": q, "x": "|".join(sorted(v))} for q, v in sorted(ext.items())]
    for i in range(0, len(rows), 300):
        chunk = rows[i:i + 300]
        maps = ", ".join("{" + f"q:{esc(r['q'])}, x:{esc(r['x'])}" + "}" for r in chunk)
        client(roadmap, f"UNWIND [{maps}] AS row MATCH (m:Method {{qualifiedName: row.q}}) "
                        "SET m.overridesExternal = split(row.x, '|')", label="overridesExternal")
    print(f"  overridesExternal set on {len(rows)} methods", file=sys.stderr)
    # A nested type's javaVisibility comes from the nested-type survey; a
    # top-level type's is the class file's ACC_PUBLIC flag.
    rows = [{"q": q, "v": v} for q, v in sorted(visibility.items())]
    for i in range(0, len(rows), 300):
        chunk = rows[i:i + 300]
        maps = ", ".join("{" + f"q:{esc(r['q'])}, v:{esc(r['v'])}" + "}" for r in chunk)
        client(roadmap, f"UNWIND [{maps}] AS row MATCH (c:Class {{qualifiedName: row.q}}) "
                        "SET c.javaVisibility = row.v", label="javaVisibility")


def verify(roadmap, calls, overrides, ext):
    got = read(roadmap, f"MATCH ()-[e:CALLS]->() WHERE e.evidence = '{EVIDENCE}' "
                        "RETURN count(e), sum(e.sites)")[0]
    want = [len(calls), sum(v[0] for v in calls.values())]
    o = read(roadmap, "MATCH ()-[e:OVERRIDES]->() RETURN count(e)")[0][0]
    x = read(roadmap, "MATCH (m:Method) WHERE m.overridesExternal IS NOT NULL RETURN count(m)")[0][0]
    print(f"  verify: CALLS {got} vs {want}; OVERRIDES {o} vs {len(overrides)}; "
          f"overridesExternal {x} vs {len(ext)}", file=sys.stderr)
    if list(got) != want or o != len(overrides) or x != len(ext):
        raise SystemExit("the graph differs from the extraction")


def find_javap(arg):
    if arg:
        return arg
    found = shutil.which("javap")
    if found:
        return found
    for cand in sorted(glob.glob(str(Path.home() / ".local/jdk-*/bin/javap")), reverse=True):
        return cand
    raise SystemExit("no javap found: install a JDK or pass --javap")


def check_jar(jar: Path):
    sha1 = Path(str(jar) + ".sha1")
    if sha1.is_file():
        want = sha1.read_text().split()[0].strip()
        got = hashlib.sha1(jar.read_bytes()).hexdigest()
        if want != got:
            raise SystemExit(f"{jar}: sha1 {got} does not match the published {want}")
        print(f"  {jar.name}: sha1 {got} matches Maven Central's", file=sys.stderr)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("roadmap")
    ap.add_argument("--jar", default=str(DEFAULT_JAR))
    ap.add_argument("--javap")
    ap.add_argument("--commit", help="provenance commit (default: HEAD of this checkout)")
    ap.add_argument("--work", default=str(kg_target() / "no_caller"))
    ap.add_argument("--dry-run", action="store_true", help="extract and report, write nothing")
    args = ap.parse_args()

    root = Path(__file__).resolve().parents[2]
    commit = subprocess.run(["git", "-C", str(root), "rev-parse", args.commit or "HEAD"],
                            capture_output=True, text=True, check=True).stdout.strip()
    date = subprocess.run(["git", "-C", str(root), "show", "-s", "--format=%cs", commit],
                          capture_output=True, text=True, check=True).stdout.strip()
    jar = Path(args.jar)
    check_jar(jar)
    javap = find_javap(args.javap)
    work = Path(args.work)
    work.mkdir(parents=True, exist_ok=True)

    t = time.time()
    with zipfile.ZipFile(jar) as z:
        names = sorted(n[:-6].replace("/", ".") for n in z.namelist()
                       if n.endswith(".class") and not n.startswith("META-INF")
                       and not n.endswith("module-info.class"))
    dump = work / "lucene-core-10.5.0.javap.txt"
    text = run_javap(javap, jar, names)
    dump.write_text(text)
    classes = parse_javap(text)
    print(f"  javap: {len(classes)} classes of {len(names)} in {time.time() - t:.0f}s", file=sys.stderr)
    if len(classes) != len(names):
        raise SystemExit("javap parse lost classes")

    in_jar = {c.name for c in classes}
    ext_names = set()
    for c in classes:
        for s in ([c.super] if c.super else []) + c.interfaces:
            if s not in in_jar:
                ext_names.add(s)
    external = {}
    todo = sorted(ext_names)
    while todo:
        info = run_javap_external(javap, todo)
        external.update(info)
        todo = sorted({s for v in info.values() for s in ([v[0]] if v[0] else []) + v[1]}
                      - set(external) - in_jar)
    hier = Hierarchy(classes, external)

    rows = read(args.roadmap, "MATCH (m:Method) RETURN m.qualifiedName, m.parentQualifiedName, "
                              "m.name, m.kind, m.signature")
    kg_classes = [r[0] for r in read(args.roadmap, "MATCH (c:Class) RETURN c.qualifiedName")]
    kg = KgMembers(rows, kg_classes)
    calls, overrides, ext, stats = build(classes, hier, kg,
                                         {k: v[2] for k, v in external.items()})
    res = defaultdict(int)
    for v in calls.values():
        res[v[1]] += 1
    print(f"  CALLS {len(calls)} ({dict(res)}), OVERRIDES {len(overrides)}, "
          f"overridesExternal {len(ext)}; {dict(stats)}", file=sys.stderr)
    if args.dry_run:
        return
    load(args.roadmap, calls, overrides, ext, commit, date, top_level_visibility(classes))
    verify(args.roadmap, calls, overrides, ext)


if __name__ == "__main__":
    main()
