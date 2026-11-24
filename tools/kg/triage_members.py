#!/usr/bin/env python3
"""Classify every unmatched (Lucene type, Java method) pair with evidence.

`measure_depth` reports 1,816 Java method names it could not find in the port.
That figure is the complement of a deliberately conservative floor, not a work
list: it counts as absent every method the port renamed idiomatically, every
method discharged by a `#[derive]`, and every method whose Rust counterpart
lives in an item the floor's node set does not reach.

This turns the figure into three classes, each carrying the rule that produced
it so the classification can be audited:

  absent      no Rust item discharges the contract -- real work still to do
  renamed     an item does discharge it, under a name the floor cannot reach
  n/a         the method exists only because of a Java construct the port does
              not have, and the port answers the same contract another way

Rules are tried strongest first and the first one that fires wins. Anything no
rule settles is `absent`: erring toward "still to port" costs a check, while the
opposite would cost a falsehood (`CLAUDE.md` 7).

Evidence is tiered by how tightly the Rust item is bound to the type:

  own     the item is on the Rust type that declares it ports this Lucene type,
          on a trait that type implements, or on one of that trait's supertraits
  super   the same, for a Rust type that ports a Java supertype -- Java
          re-declares inherited methods as overrides, the port declares once
  file    anywhere else in the porting file, its sibling `macros.rs`, or a
          free function of the same module

The loosest name rule -- a Rust name that extends the Java one, which is how
`BulkOperationPacked.decode` became `decode_long_blocks_to_longs` -- is allowed
only at `own` and `super`, where the item is bound to the type. At `file` scope
it would match any name against any of the dozens of types a large module ports.

The `file` tier also refuses an item whose `impl` block ports a different Lucene
type, for the same reason. That refusal is a proximity argument and it does not
apply to an explicit claim: a doc comment naming this member of this type is the
port stating the correspondence, not a name that happens to collide, and it is
read whatever block encloses it. Only the claim is exempt; the name rules of the
tier keep their binding.

A claim is not always written on a function. Where Java returns an anonymous
class from a factory method, the port declares a *type* and writes the claim
there -- `KnnVectorValues.fromDISI` is `struct FromDisiDocIndexIterator` -- and
where Java distinguishes two behaviours by subclassing, the port distinguishes
them by enum variant and writes it on the variant, as
`LateInteractionRescorer.combine` is written on `LateInteractionFallback::Zero`.
Those docs are read too, through a stricter recogniser: a function's doc is
about that one function, so a member it names is almost always the
correspondence, while a *type's* doc describes the whole type and routinely
discusses members the port decided **not** to have -- `BytesRefBuilder` names
`equals` and `hashCode` precisely to declare that it implements neither. A claim
read from a declaration must therefore be introduced by an equivalence lead-in,
where one read from a function need only name the member.
"""
import argparse
import json
import os
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from rust_surface import parse_tree
from port_modules_kg import JAVA_PREFIXES, claim_crate_of

# --------------------------------------------------------------------------
# Java-to-Rust name shapes
# --------------------------------------------------------------------------

def camel_to_snake(name):
    s = re.sub(r"(.)([A-Z][a-z]+)", r"\1_\2", name)
    s = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s)
    return s.lower()


flat = lambda s: s.replace("_", "")

ACCESSOR_PREFIXES = ("get_", "is_", "has_", "set_")

# Java's Object/Comparable/Iterable contracts are discharged by a Rust trait,
# not by a like-named function. Each entry names the trait that answers the
# method and the function that trait uses, so the evidence can say which.
CONTRACT_TRAITS = {
    "toString":   [("Display", "fmt")],
    "equals":     [("PartialEq", "eq"), ("Eq", "eq")],
    "equalsTo":   [("PartialEq", "eq"), ("Eq", "eq")],
    "hashCode":   [("Hash", "hash")],
    "clone":      [("Clone", "clone"), ("Copy", "clone")],
    "compareTo":  [("Ord", "cmp"), ("PartialOrd", "partial_cmp")],
    "iterator":   [("IntoIterator", "into_iter"), ("Iterator", "next")],
    "close":      [("Drop", "drop")],
}
# Of those, the ones Rust can satisfy with `#[derive]`, which emits no `fn`.
DERIVABLE = {"PartialEq", "Eq", "Hash", "Clone", "Copy", "Ord", "PartialOrd",
             "Default", "Debug"}

# Names that are Rust keywords: the port has no choice but to rename, so the
# rename is forced by the language rather than chosen. `TypeAttribute.type()`
# is `type_value()` for exactly this reason.
RUST_KEYWORDS = {
    "abstract", "as", "async", "await", "become", "box", "break", "const",
    "continue", "crate", "do", "dyn", "else", "enum", "extern", "false",
    "final", "fn", "for", "if", "impl", "in", "let", "loop", "macro", "match",
    "mod", "move", "mut", "override", "priv", "pub", "ref", "return", "self",
    "static", "struct", "super", "trait", "true", "try", "type", "typeof",
    "union", "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
}

# Java contracts whose Rust spelling is fixed by the language rather than by
# taste, and which therefore justify accepting an extended name at item scope.
# `clone()` on a Lucene reader or input means "an independent cursor over the
# same data", which the crate spells `clone_reader` / `clone_input` because
# `Clone` on a `Box<dyn Trait>` cannot express it.
SHAPES = {
    "clone": ("clone-shape", ("clone", "clone_")),
    "iterator": ("iterator-shape", ("iter", "into_iter", "iter_mut", "iter_")),
}

# Methods that exist only to serve a Java mechanism this port does not have.
JAVA_ONLY = {
    "finalize": "java.lang.Object finalizer; Rust frees on Drop",
    "readObject": "java.io.Serializable hook; the port has no Java serialization",
    "writeObject": "java.io.Serializable hook; the port has no Java serialization",
    "readResolve": "java.io.Serializable hook; the port has no Java serialization",
    "writeReplace": "java.io.Serializable hook; the port has no Java serialization",
}


def name_candidates(java):
    s = camel_to_snake(java)
    out = {s}
    for p in ACCESSOR_PREFIXES:
        if s.startswith(p):
            out.add(s[len(p):])
    return out


# --------------------------------------------------------------------------
# Rust-side evidence
# --------------------------------------------------------------------------

# A doc comment states which Lucene member an item discharges as either
# `Foo#bar` or `Foo.Bar.baz(...)`. Both the type and the member are captured:
# crediting `Foo#close` to `Bar.close` would turn one type's evidence into
# another's, which is precisely the error this triage exists to avoid.
# `$` is a legal Java identifier character and Lucene uses it in a member name:
# `GroupVIntUtil` declares `readGroupVInts$Baseline` and `readGroupVInt$Baseline`
# (`GroupVIntUtil.java:100,111`), the benchmark baselines. Stopping the member
# name at the `$` produced `readGroupVInts`, which is a *different* member of the
# same class, so the verbatim claim the port writes matched nothing.
RE_DOC_MEMBER = re.compile(
    r"([A-Z][A-Za-z0-9_]*)\s*[#.]\s*([a-z][A-Za-z0-9_$]*)\s*[(`]")
RE_DOC_HASH = re.compile(r"([A-Z][A-Za-z0-9_]*)#([a-z][A-Za-z0-9_$]*)")


def _doc_above(rec, trait):
    """The doc comment of a trait declaration, which `parse` keeps per file."""
    return rec.get("trait_docs", {}).get(trait["name"], "")


def doc_members(doc):
    """{(java type simple name, java member name)} the doc text names."""
    if not doc:
        return set()
    return set(RE_DOC_MEMBER.findall(doc)) | set(RE_DOC_HASH.findall(doc))


# The crate states which Java type an item ports in its own doc comment
# (`CLAUDE.md` 14.1). Reading that from the source is stronger than reading the
# `PORTS` edges, which exist only for the claims the graph loader resolved. A
# claim names a type of any port module -- Lucene's, randomizedtesting's and the
# ported libraries' (`port_modules_kg.JAVA_PREFIXES`) -- so a library member is
# triaged against its own type, not misreported for lack of a Lucene name.
RE_CLAIM = re.compile(
    r"(?:Equivalent to|Port of|Ported from|ported from)\s+(?:the\s+)?"
    r"(?:abstract |final |sealed |package-private |static |nested |private )*"
    r"(?:class |interface |enum |record )?"
    r"`((?:" + "|".join(re.escape(p) for p in JAVA_PREFIXES) + r")\.[A-Za-z0-9_.$]+)`")

def _own_claim(fqn, file):
    """Whether the claim of `file` on `fqn` may port it: always, except for a
    type only its own crate ports (`port_modules_kg.claim_crate_of`, the JDK's
    `org.xml.sax` emulated elsewhere), as `port_evidence_kg.load` rules."""
    own = claim_crate_of(fqn)
    return own is None or file.startswith(own + "/")


# The same claim, naming a *member*, and read from a declaration's doc rather
# than a function's. It is deliberately stricter than `doc_members`: a
# function's doc is about that one function, so a member it names is almost
# always the correspondence, but a *type's* doc is about the whole type and
# routinely discusses members the port decided not to have. `BytesRefBuilder`'s
# doc names `BytesRefBuilder.equals(Object)` and `BytesRefBuilder.hashCode()`
# precisely to declare that neither is implemented, because Lucene's throw
# `UnsupportedOperationException`; reading that as evidence of a port would be a
# false `renamed`, which hides real work. So the reference must be introduced by
# an equivalence lead-in, with nothing between it and the reference but the
# modifiers Lucene's own vocabulary uses.
RE_DECL_CLAIM = re.compile(
    r"(?:Equivalent to|Port of|Ported from|ported from)\s+(?:the\s+)?"
    r"(?:abstract |final |sealed |package-private |static |nested |private |"
    r"protected |public |anonymous |inherited |synthetic |generated )*"
    r"(?:class |interface |enum |record |method |field )?"
    r"`(?:[A-Za-z0-9_$]+\.)*([A-Z][A-Za-z0-9_$]*)\s*[#.]\s*"
    r"([a-z][A-Za-z0-9_$]*)")


# A doc comment that *opens* with the Java member is the crate's usual way of
# saying which member an item discharges when the type is already established by
# the item's owner: ``/// `toString()`: `delegate.toString()`.`` on a `fmt`,
# ``/// Overrides `verify(Method)`: `false` skips a method.`` on a builder, and
# ``/// `value()`, default `random`.`` on an annotation's field. Anchored at the
# start of the doc, after at most one lead-in and Java's modifiers, so a doc that
# merely *discusses* a member somewhere in its prose is not read as a claim.
RE_DOC_LEAD = re.compile(
    r"^(?:(?:Overrides|Override of|Port of|Ported from|Equivalent to|Java's|Java|The)"
    r"\s+(?:the\s+)?)?"
    r"(?:(?:private|protected|public|static|abstract|final|synchronized)\s+)*"
    r"`(?:@[A-Za-z]\w*\s+)?(?:(?P<t>[A-Z][\w$]*)[.#])?(?P<m>[a-z$][\w$]*)\s*(?:\(|`)")


# Words that turn a mention into a denial: the crate names a member it decided
# not to have in the same shape it names one it has.
RE_DENIAL = re.compile(r"\bnot (?:ported|implemented|reproduced|supported)\b|"
                       r"\bSkeleton of\b|\bomitted\b|\bunported\b|\bTODO\b", re.I)


# The same correspondence written as a parenthesis inside the doc:
# ``the `FieldMetadata` of the field being written (Java's `setField(FieldMetadata)`)``.
RE_DOC_PAREN = re.compile(
    r"\(Java's `(?:(?P<t>[A-Z][\w$]*)[.#])?(?P<m>[a-z$][\w$]*)\s*(?:\(|`)")
# And as a parenthesis that glosses a value by the member computing it: a heap
# key's "secondary key (`compareSecondary`: the field number ...)".
RE_DOC_PAREN_COLON = re.compile(
    r"\(`(?:(?P<t>[A-Z][\w$]*)[.#])?(?P<m>[a-z$][\w$]*)(?:\([^`]*\))?`:")


def doc_lead(doc):
    """`(java type simple name or None, java member name)` a doc opens with,
    or names in a `(Java's `...`)` parenthesis."""
    doc = (doc or "").strip()
    m = RE_DOC_LEAD.match(doc) or RE_DOC_PAREN.search(doc) or RE_DOC_PAREN_COLON.search(doc)
    if not m or RE_DENIAL.search(doc):
        return None
    return (m.group("t"), m.group("m"))


# A paragraph declaring a divergence (`CLAUDE.md` 14.5): the port states that
# it does not reproduce something of Lucene's, and why. When such a paragraph
# names a member -- `**Divergences.** `toString`, `equals` and `hashCode`
# (which Java ...)` -- the member's absence is the declared, approved
# adaptation, not open work.
RE_DIVERGENCE = re.compile(r"\*\*Divergences?\b|^#+\s*Divergences?\b", re.M)
# A paragraph that declares a divergence or refers to one declared elsewhere
# ("the divergence declared in `rucene::util::resource_loader`").
RE_DIVERGENCE_REF = re.compile(r"\bdivergen(?:ce|ces|t)\b", re.I)


def doc_paragraphs(text):
    """`[(first line, paragraph text)]` of the `///` and `//!` comments of a file."""
    out, cur, start = [], [], None
    for i, line in enumerate(text.splitlines(), 1):
        st = line.strip()
        body = None
        if st.startswith("///") or st.startswith("//!"):
            body = st[3:].strip()
        if body:
            if not cur:
                start = i
            cur.append(body)
            continue
        if cur:
            out.append((start, " ".join(cur)))
            cur = []
    if cur:
        out.append((start, " ".join(cur)))
    return out


def code_lines(text):
    """`[(line number, code)]` with `//` comments cut off, for call-site search."""
    out = []
    for i, line in enumerate(text.splitlines(), 1):
        code = line.split("//", 1)[0]
        if code.strip():
            out.append((i, code))
    return out


def decl_claim_members(doc):
    """{(java type simple name, java member name)} a declaration's doc claims."""
    if not doc:
        return set()
    return set(RE_DECL_CLAIM.findall(doc))


class Surface:
    """Everything the crate declares, indexed the way the triage asks for it."""

    def __init__(self, tree):
        self.tree = tree
        self.trait_decls = defaultdict(list)  # trait name -> [(file, record)]
        self.impls_of = defaultdict(list)   # rust type name -> [(file, impl)]
        self.type_decl = defaultdict(list)  # rust type name -> [(file, record)]
        self.file_fns = {}            # file -> {fn name -> [(where, doc)]}
        self.file_decls = {}          # file -> [(what it is, its doc)]
        self.type_claims = defaultdict(set)   # (rust type, file) -> {lucene fqn}
        self.fields_of = defaultdict(list)    # rust type name -> [(file, field)]
        for f, rec in tree.items():
            for name, td in rec["types"].items():
                for fd in td.get("fields", ()):
                    self.fields_of[name].append((f, fd))
            for t in rec["traits"]:
                self.trait_decls[t["name"]].append((f, t))
                for fqn in RE_CLAIM.findall(
                        _doc_above(rec, t) or ""):
                    if _own_claim(fqn.rstrip("."), f):
                        self.type_claims[(t["name"], f)].add(fqn.rstrip("."))
            for im in rec["impls"]:
                self.impls_of[im["self"]].append((f, im))
            for name, td in rec["types"].items():
                self.type_decl[name].append((f, td))
                for fqn in RE_CLAIM.findall(td["doc"] or ""):
                    if _own_claim(fqn.rstrip("."), f):
                        self.type_claims[(name, f)].add(fqn.rstrip("."))
            # Every declaration site, not just the first: `standard.rs`
            # declares `set_max_token_length` on three types, and keeping only
            # the first hid the one that answers `StandardTokenizer` behind the
            # one that answers `StandardTokenizerImpl`.
            got = defaultdict(list)
            for fn in rec["free"]:
                if fn.get("test"):
                    continue          # a test exercises a member; it does not port it
                got[fn["name"]].append(("free fn", fn["doc"]))
            for im in rec["impls"]:
                head = (f"impl {im['trait']} for {im['self']}" if im["trait"]
                        else f"impl {im['self']}")
                for fn in im["fns"]:
                    got[fn["name"]].append((head, fn["doc"]))
            for t in rec["traits"]:
                for fn in t["fns"]:
                    got[fn["name"]].append((f"trait {t['name']}", fn["doc"]))
            for mc in rec["macros"]:
                for fn in mc["fns"]:
                    got[fn["name"]].append((f"macro_rules! {mc['name']}",
                                            fn["doc"]))
            for c in rec["consts"]:
                got[c.lower()].append(("const", ""))
            self.file_fns[f] = dict(got)
            # A claim is not always written on a function. Where Java returns an
            # anonymous class from a factory method, the port declares a *type*
            # and writes the claim there; where Java distinguishes behaviours by
            # subclassing, the port distinguishes them by enum variant and
            # writes it on the variant. Those are declarations under
            # `CLAUDE.md` 14.1 exactly as a function is, so their docs are
            # indexed alongside.
            decls = []
            for t in rec["traits"]:
                decls.append((f"trait {t['name']}", t["doc"]))
            for name, td in rec["types"].items():
                decls.append((f"{td.get('kind', 'type')} {name}", td["doc"]))
                for v in td.get("variants", ()):
                    decls.append((f"variant {name}::{v['name']}", v["doc"]))
            self.file_decls[f] = decls
        # The files declaring each Rust type or trait name, so an `impl`, a
        # derive or a field is bound to the declaration it belongs to.
        self.decl_files = defaultdict(set)
        for name, sites in list(self.type_decl.items()) + list(self.trait_decls.items()):
            self.decl_files[name] |= {f for f, _ in sites}

    @staticmethod
    def nearest(files, near):
        """The members of `files` closest to the file `near`: the same file,
        then the same directory, then the same top-level module (`src/<top>`),
        and only then every one."""
        files = sorted(files)
        if len(files) <= 1 or not near:
            return files
        d = os.path.dirname(near)
        top = near.split("/")[1] if near.startswith("src/") else ""
        for pick in (lambda f: f == near,
                     lambda f: os.path.dirname(f) == d,
                     lambda f: f.startswith(f"src/{top}") if top else False):
            hit = [f for f in files if pick(f)]
            if hit:
                return hit
        return files

    def binds(self, rtype, rfile, site):
        """Whether an item at `site` (an `impl` of `rtype`, its declaration or
        a field of it) belongs to the `rtype` that `rfile` declares.

        Rust type names are not unique across the crate: `FieldValuesSource`
        is declared by both `long_values_source.rs` and
        `double_values_source.rs`. Keyed by simple name alone, the impls of one
        answered for the other, and `LongValuesSource$FieldValuesSource.hashCode`
        was credited to `impl DoubleValuesSource for FieldValuesSource`. When
        the name is declared in several files, an item binds to the
        declaration nearest to it (`nearest`); a name declared once, or a
        caller that names no file, keeps every site.
        """
        decls = self.decl_files.get(rtype, ())
        if rfile is None or len(decls) <= 1 or rfile not in decls:
            return True
        return rfile in self.nearest(decls, site)

    def fields(self, rtype, rfile=None):
        """`(file, field)` of the struct `rtype` that `rfile` declares."""
        return [(f, fd) for f, fd in self.fields_of.get(rtype, ())
                if self.binds(rtype, rfile, f)]

    def is_closure_type(self, ty, depth=0):
        """Whether a field type is a closure (`Fn`/`FnMut`/`FnOnce`), through
        at most two type aliases."""
        if re.search(r"\bFn(?:Mut|Once)?\s*\(|\bfn\s*\(", ty or ""):
            return True
        if depth >= 2:
            return False
        for name in re.findall(r"[A-Z][A-Za-z0-9_]*", ty or ""):
            for _f, td in self.type_decl.get(name, ()):
                if td.get("kind") == "type" and self.is_closure_type(td.get("alias"), depth + 1):
                    return True
        return False

    def traits_of(self, rtype, rfile=None):
        """Trait names `rtype` implements, plus their supertraits and derives."""
        out = set()
        for f, im in self.impls_of.get(rtype, ()):
            if not self.binds(rtype, rfile, f):
                continue
            if im["trait"]:
                out.add(im["trait"])
                for t, _tf, _rec in self.trait_closure(im["trait"], f):
                    out.add(t)
        return out | self.derived_traits(rtype, rfile)

    def derived_traits(self, rtype, rfile=None):
        out = set()
        for f, td in self.type_decl.get(rtype, ()):
            if self.binds(rtype, rfile, f):
                out |= set(td["derives"])
        return out

    def resolve_trait(self, name, near):
        """The declaration(s) of trait `name` closest to the file `near`.

        Five trait simple names are declared twice in the crate -- `Query`
        among them, which `src/index/documents_writer.rs` also declares. Taking
        whichever was seen first credited every query type with the wrong
        trait's members and hid its real ones, so resolution is by proximity:
        the same file, then the same directory, then the same top-level module,
        and only then every candidate.
        """
        cands = self.trait_decls.get(name, ())
        if len(cands) <= 1:
            return list(cands)
        if not near:
            return list(cands)
        d = os.path.dirname(near)
        top = near.split("/")[1] if near.startswith("src/") else ""
        for pick in (lambda f: f == near,
                     lambda f: os.path.dirname(f) == d,
                     lambda f: f.startswith(f"src/{top}") if top else False):
            hit = [c for c in cands if pick(c[0])]
            if hit:
                return hit
        return list(cands)

    def trait_closure(self, name, near):
        """`(name, file, record)` for `name` and every supertrait of it."""
        out, seen, frontier = [], set(), [(name, near)]
        while frontier:
            t, ctx = frontier.pop()
            if t in seen:
                continue
            seen.add(t)
            for f, rec in self.resolve_trait(t, ctx):
                out.append((t, f, rec))
                frontier += [(sup, f) for sup in rec["supers"]]
        return out

    def item_fns(self, rtype, rfile=None):
        """(name -> (where, doc)) for every fn bound to the Rust type itself.

        A Lucene interface is normally ported as a Rust *trait*, so the port of
        its members are the trait's own declarations. Reading only `impl`
        blocks missed every one of them and pushed the evidence down to file
        scope, where it is far weaker.
        """
        got = {}
        near = rfile
        if near is None:
            for f, _td in self.type_decl.get(rtype, ()):
                near = f
                break
        # (`trait_closure` resolves a trait by proximity already.)
        # The port of a Lucene interface is a Rust trait, so when the item
        # itself is a trait its own declarations are the members.
        for t, f, rec in self.trait_closure(rtype, near):
            for fn in rec["fns"]:
                got.setdefault(fn["name"], (f"{f}: trait {t}", fn["doc"]))
        for f, im in self.impls_of.get(rtype, ()):
            if not self.binds(rtype, rfile, f):
                continue
            head = (f"impl {im['trait']} for {im['self']}" if im["trait"]
                    else f"impl {im['self']}")
            for fn in im["fns"]:
                got.setdefault(fn["name"], (f"{f}: {head}", fn["doc"]))
            if im["trait"]:
                for t, tf, rec in self.trait_closure(im["trait"], f):
                    for fn in rec["fns"]:
                        got.setdefault(fn["name"],
                                       (f"{tf}: trait {t}", fn["doc"]))
        # A type the crate builds with a `macro_rules!` has no `impl` under its
        # own name: the template says `impl ... for $name`. The port of every
        # hppc container lives that way, so credit the macros of the files that
        # port the type with the members they generate. The test is that the
        # crate declares no such type *and* no such trait anywhere -- an empty
        # `impls_of` alone is true of every trait, which credited the `Query`
        # trait with every macro-built type in `src/search/`.
        if (rfile and not self.impls_of.get(rtype)
                and rtype not in self.type_decl and rtype not in self.trait_decls):
            for f, im in self.impls_of.get("$name", []) + \
                    self.impls_of.get("$map", []) + self.impls_of.get("$list", []):
                if os.path.dirname(f) != os.path.dirname(rfile):
                    continue
                head = (f"impl {im['trait']} for {im['self']}" if im["trait"]
                        else f"impl {im['self']}")
                for fn in im["fns"]:
                    got.setdefault(fn["name"], (f"{f}: {head}", fn["doc"]))
        return got


# --------------------------------------------------------------------------
# Rule ladder
# --------------------------------------------------------------------------

def split_of(java, have):
    """True when the port split one Java method into several Rust functions.

    Java overloads on argument type; Rust cannot, so a method Lucene declares
    once against an abstract parameter is spelled as one function per concrete
    type -- `getRandomVectorScorerSupplier(sim, KnnVectorValues)` becomes
    `_float` and `_byte`. Two or more functions extending the same Java name is
    evidence of that deliberate split, where a single one is just a longer name
    that happens to start the same way.
    """
    for c in name_candidates(java):
        if len(c) < 3:
            continue
        if sum(1 for h in have if h.startswith(c + "_")) >= 2:
            return True
    return False


def try_names(java, have, scope, allow_expansion, doc_types=()):
    """First name rule that fires, as (rule, rust name), or None.

    `have` maps a Rust function name to (where it is declared, its doc text).
    `doc_types` is the set of Java simple names whose members this type may be
    credited with: its own and those of its supertypes.
    """
    cands = name_candidates(java)
    s = camel_to_snake(java)
    if s in have:
        return ("exact", s)
    for c in sorted(cands - {s}):
        if c in have:
            return ("accessor-prefix", c)
    fl = {flat(c) for c in cands}
    for h in sorted(have):
        if flat(h) in fl:
            return ("separator", h)
    # A doc comment that names the Java member is the port stating the
    # correspondence itself (CLAUDE.md 14.1), which beats any name heuristic --
    # but only when it names *this* type or one this type inherits from.
    for h in sorted(have):
        for dt, dm in doc_members(have[h][1]):
            if dm == java and dt in doc_types:
                return ("doc-comment", h)
    # A Rust name that extends the Java one is how the port spells a Java
    # overload set, which has no equivalent in Rust: `BulkOperationPacked`
    # declares `decode` four times and the port names each one for the types it
    # moves. Allowing it for a method Java declares once would let any longer
    # name absorb any shorter one, so the rule is gated on the overload.
    if allow_expansion or (scope != "file" and split_of(java, have)):
        forbid = ()
        if java[:3] in ("get", "has") or java[:2] == "is":
            forbid = ("set_",)
        elif java[:3] == "set":
            forbid = ("get_", "is_", "has_")
        for c in sorted(cands):
            if len(c) < 3:
                continue
            for h in sorted(have):
                if h.startswith(forbid):
                    continue
                if h.startswith(c + "_") or h.endswith("_" + c):
                    return ("name-extension", h)
    return None


def classify(pair, ctx):
    """(class, rule, evidence) for one unmatched pair."""
    cls, java = pair["cls"], pair["method"]
    S, rtypes, supertypes, files = (ctx["surface"], ctx["rtypes"].get(cls, []),
                                    ctx["srtypes"].get(cls, []),
                                    ctx["files"].get(cls, []))
    doc_types = ctx["doctypes"].get(cls, {cls.rsplit(".", 1)[-1], cls.rsplit(".", 1)[-1].split("$")[-1]})
    # A newtype wrapping the port type in a porting file answers for it: the
    # port hands out `struct RAMTerms(Arc<RAMField>)` as the `Terms` Java's
    # `RAMField` is, so the wrapper's impls are the type's own.
    rtypes = list(rtypes)
    for rt, rf in list(rtypes):
        word = re.compile(r"\b" + re.escape(rt) + r"\b")
        for f in files:
            for wname, td in S.tree.get(f, {}).get("types", {}).items():
                tup = td.get("tuple")
                if tup and "," not in tup.strip().rstrip(",") and word.search(tup) \
                        and (wname, f) not in rtypes:
                    rtypes.append((wname, f))
    overloaded = ctx["overloads"].get((cls, java), 1) >= 2

    # 0. A member the port has decided against. `port_gaps_exclusions.json`
    #    records the decision and, since this triage, whether it is the absence
    #    of a Java construct (nothing to discharge) or work deferred (still an
    #    absence). Only the first is `na`.
    exc = ctx["excluded"].get(cls)
    if exc and java in exc.get("methods", ()) and exc.get("triageClass") == "na":
        return ("na", "excluded-construct", exc["triageReason"])
    #    A verdict curated for one member, where no evidence rule can read the
    #    port's shape (`memberVerdicts`): each carries its class and the file
    #    and line that show it. The last resort, as for `nested_port_verdicts`.
    mv = (exc or {}).get("memberVerdicts", {}).get(java)
    if mv:
        return (mv["triageClass"], "curated", mv["triageReason"])

    # 1. Java-only mechanisms: no contract to discharge at all.
    if java in JAVA_ONLY:
        return ("na", "java-only", JAVA_ONLY[java])

    # 2. Object/Comparable/Iterable contracts, answered by a Rust trait.
    #    Own types only: Rust has no inheritance, so `impl Display for
    #    FilterDirectory` says nothing about `NRTCachingDirectory`, which
    #    overrides `toString` in Java and has no `Display` here. Reading the
    #    supertype's impl as the subtype's answer reported both that and
    #    `IndexSortSortedNumericDocValuesRangeQuery.hashCode` -- a type that
    #    derives only `Debug` -- as present.
    if java in CONTRACT_TRAITS:
        for rt, rf in rtypes:
            impl_traits = S.traits_of(rt, rf)
            derived = S.derived_traits(rt, rf)
            if not impl_traits:
                # macro-generated: the impls carry the template's `$name`
                impl_traits = set()
                for w, _d in S.item_fns(rt, rf).values():
                    head = w.split(": ")[-1]
                    if head.startswith("impl ") and " for " in head:
                        impl_traits.add(head.split()[1])
            for trait, fn in CONTRACT_TRAITS[java]:
                if trait in impl_traits:
                    where = "own" if (rt, rf) in rtypes else "super"
                    if trait in derived and trait in DERIVABLE:
                        return ("na", f"derive[{where}]",
                                f"#[derive({trait})] on {rt}")
                    return ("renamed", f"contract-trait[{where}]",
                            f"{rt}: impl {trait} -> {fn}")

    # 2b. Shapes the language forces: a Java name that is a Rust keyword, and
    #     the `clone`/`iterator` contracts whose Rust spelling is idiomatic
    #     rather than optional. Item scope only -- at file scope any `iter`
    #     would answer any type's `iterator`.
    for tier, types in (("own", rtypes), ("super", supertypes)):
        for rt, rf in types:
            have = S.item_fns(rt, rf)
            if java in SHAPES:
                rule, prefixes = SHAPES[java]
                for h in sorted(have):
                    if h in prefixes or any(h.startswith(pre) for pre in prefixes
                                            if pre.endswith("_")):
                        return ("renamed", f"{rule}[{tier}]",
                                f"{h} in {have[h][0]}")
            for c in sorted(name_candidates(java)):
                if c not in RUST_KEYWORDS:
                    continue
                for h in sorted(have):
                    if h == f"r#{c}" or h == f"{c}_" or h.startswith(f"{c}_"):
                        return ("renamed", f"rust-keyword[{tier}]",
                                f"{h} in {have[h][0]} (`{c}` is a Rust keyword)")

    # 2c. A *field* of the port type. Java declares an annotation's members as
    #     methods (`Seeds.value()`, `Timeout.millis()`); the port keeps the
    #     annotation as a value whose fields are those members. And Java makes
    #     a behaviour overridable with a protected method a subclass replaces
    #     (`NoShadowingOrOverridesOnMethodsRule.verify`); the port, which has no
    #     subclassing, holds the replacement as a closure field set by a
    #     builder. Own tier only: the field must be on the type that declares it
    #     ports this one, and its name must be the member's.
    java_kind = ctx["kinds"].get(cls)
    for rt, rf in rtypes:
        for f, fd in S.fields(rt, rf):
            nm = fd["name"][2:] if fd["name"].startswith("r#") else fd["name"]
            if nm not in name_candidates(java):
                continue
            where = f"field {rt}.{fd['name']} in {f}:{fd['line']}"
            if java_kind == "annotation":
                return ("renamed", "annotation-member[own]", where)
            if S.is_closure_type(fd["ty"]):
                return ("renamed", "closure-hook[own]", f"{where} ({fd['ty'].rstrip(',')})")

    # 3. A name rule, tried at each tier from the tightest binding outward.
    # A supertype's Rust trait only answers for this type if this type's own
    # port implements it. `Transition.ramBytesUsed` was answered by `trait
    # Accountable`'s declaration although nothing implements `Accountable` for
    # `Transition`. When the crate declares no type of this name at all, the
    # port's decomposition is elsewhere and there is nothing to check against.
    own_declared, own_traits = False, set()
    for rt, rf in rtypes:
        if rt in S.type_decl or rt in S.trait_decls:
            own_declared = True
        own_traits.add(rt)
        own_traits |= S.traits_of(rt, rf)
        own_traits |= {t for t, _f, _r in S.trait_closure(rt, rf)}

    for tier, types in (("own", rtypes), ("super", supertypes)):
        for rt, rf in types:
            have = S.item_fns(rt, rf)
            if tier == "super" and own_declared:
                have = {n: w for n, w in have.items()
                        if not w[0].split(": ")[-1].startswith("trait ")
                        or w[0].split(": ")[-1][6:].strip() in own_traits}
            hit = try_names(java, have, tier, allow_expansion=overloaded,
                            doc_types=doc_types)
            if hit:
                rule, rname = hit
                where = have[rname][0]
                return ("renamed", f"{rule}[{tier}]", f"{rname} in {where}")

    # 4. Anywhere in the files that port the type, or the sibling macro file
    #    that stands in for Lucene's code generator -- except items another
    #    Lucene type already claims. `Terms.getTerms` was answered by `terms`
    #    in `impl Fields for EmptyFields`, which is the port of
    #    `Fields.terms`, not of anything on `Terms`.
    claimed, related = ctx["claimed"], ctx["related"].get(cls, {cls})
    # The type's own simple name: for a nested type the last `$` segment, the
    # name its Rust item and its doc comments use.
    simple, multiword = cls.rsplit(".", 1)[-1].split("$")[-1], "_" in camel_to_snake(java)
    full_simple = cls.rsplit(".", 1)[-1]
    have = {}
    for f in files:
        for n, sites in S.file_fns.get(f, {}).items():
            for owner, doc in sites:
                if " for " in owner:
                    own_ty = owner.split(" for ", 1)[1].strip()
                elif owner.startswith("impl "):
                    own_ty = owner[5:].strip()
                elif owner.startswith("trait "):
                    own_ty = owner[6:].strip()
                else:
                    own_ty = None
                # An item that says which Lucene type it ports answers for that
                # type, not for this one. The crate's own doc comment is the
                # authority here, not the `PORTS` edges, which exist only where
                # the graph loader resolved the claim.
                owned_by = (claimed.get((own_ty, f), set())
                            | S.type_claims.get((own_ty, f), set())) if own_ty else set()
                if owned_by and not (owned_by & related):
                    continue
                # A one-word Java name -- `close`, `flush`, `save`, `select` --
                # collides by accident with any sibling in the module, and file
                # scope has no binding to the type to rule the coincidence out.
                # `InputStreamDataInput.close` was answered by
                # `RandomAccessInputAdapter::close` that way. Accept it only
                # from a free function, which is where a Java static lands, or
                # from a type whose name says it belongs to this one
                # (`StopFilter` -> `StopFilterLogic`, `Selector` ->
                # `RadixSelector`). A name of two words or more cannot collide
                # by accident and needs no such guard.
                if not multiword and owner != "free fn":
                    if not (own_ty and (simple in own_ty or own_ty in full_simple)):
                        continue
                have.setdefault(n, (f"{f}: {owner}", doc))
                break
    # A one-word Java name -- `close`, `select`, `grow`, `values` -- collides
    # by accident with any sibling in the same module, and file scope has no
    # binding to the type to rule the coincidence out. `InputStreamDataInput`'s
    # `close` was answered that way by `RandomAccessInputAdapter::close`, which
    # is a different type's method. Only a name that carries more than one word,
    # or a doc comment that states the correspondence, settles it here.
    hit = try_names(java, have, "file", allow_expansion=False,
                    doc_types=doc_types)
    if hit:
        rule, rname = hit
        return ("renamed", f"{rule}[file]", f"{rname} in {have[rname][0]}")

    # 4b. An explicit claim written on the item itself, read whatever the
    #     enclosing `impl` block ports. The two guards above are proximity
    #     arguments: they exist because a bare name at file scope carries no
    #     binding to the type, so one type's function must not be credited to
    #     another by coincidence. A doc comment that names *this member of this
    #     type* is not a coincidence -- it is the port declaring the
    #     correspondence itself (`CLAUDE.md` 14.1), which is why the ladder
    #     ranks `doc-comment` above every heuristic. Gating it on the enclosing
    #     block let the weakest consideration veto the strongest evidence:
    #     `SegmentTermsEnumFrame#prefetchBlock` is written on a function inside
    #     `impl SegmentTermsEnum`, so the claim guard dropped the item before
    #     the rule could read it, and the method was reported absent.
    #
    #     Only the explicit claim is admitted this way. `exact`, `accessor-
    #     prefix` and `separator` at this tier keep their binding, because for
    #     them the coincidence is exactly the failure mode. The type check is
    #     kept too: the claim must name this type or one of its Java
    #     supertypes, which is what stops the credit crossing to another type.
    #     Every declaration site is read, not just the first the guards accept:
    #     `have` keeps one site per name, so a claim written on the second
    #     function of a name was invisible for the same reason.
    for f in files:
        for n, sites in S.file_fns.get(f, {}).items():
            for owner, doc in sites:
                for dt, dm in sorted(doc_members(doc)):
                    if dm == java and dt in doc_types:
                        return ("renamed", "doc-comment[file]",
                                f"{n} in {f}: {owner}")

    # 4c. The same claim, written on a *declaration* rather than on a function.
    #     A Java factory method that returns an anonymous class has no function
    #     to carry the correspondence in Rust: the port declares the type the
    #     anonymous class becomes, and the claim goes on that type.
    #     `KnnVectorValues.fromDISI` is `struct FromDisiDocIndexIterator`;
    #     `StringSorter.radixSorter` and `StableStringSorter.radixSorter` are
    #     `struct StringRadixOps` and `struct StableStringRadixOps`. Where Java
    #     distinguishes two behaviours by subclassing, the port distinguishes
    #     them by enum variant, and the claim goes on the variant --
    #     `LateInteractionRescorer.combine` on `LateInteractionFallback::Zero`.
    #     Nothing is loosened but the *shape of the item* the claim may sit on:
    #     it is the same rule, with the same type check, reading a declaration's
    #     doc comment instead of a function's. Both are documentation under
    #     `CLAUDE.md` 14.1; only one of them was being read.
    #     The claim this reads must be an equivalence statement, not a mention:
    #     see `RE_DECL_CLAIM`. And a claim naming *this* type is preferred over
    #     one naming a supertype, so the evidence points at the declaration that
    #     answers the override rather than at the one that answers the
    #     declaration it overrides -- `StableStringSorter.radixSorter` is
    #     `StableStringRadixOps`, not the `StringRadixOps` that answers
    #     `StringSorter.radixSorter`.
    for exact_type in (True, False):
        for f in files:
            for what, doc in S.file_decls.get(f, ()):
                for dt, dm in sorted(decl_claim_members(doc)):
                    if dm != java or dt not in doc_types:
                        continue
                    if exact_type and dt != simple:
                        continue
                    return ("renamed", "doc-comment[file]", f"{what} in {f}")

    # 4c'. A hand-written `impl Debug` on the type that ports this one, for
    #      `toString`. A Java `toString` that exists for diagnostics is answered
    #      in Rust by `{:?}`, and a hand-written `fmt` is a deliberate rendering
    #      of the value. A *derived* `Debug` is not: it prints Rust's field dump,
    #      not Lucene's text, so it never settles a `toString`, and this rule
    #      runs only after every claim and name rule has had its chance.
    if java == "toString":
        for rt, rf in rtypes:
            if "Debug" in S.traits_of(rt, rf) and "Debug" not in S.derived_traits(rt, rf):
                return ("renamed", "debug-impl[own]", f"{rt}: impl Debug -> fmt")

    # 4d. A doc comment that *opens* with this member (`RE_DOC_LEAD`), on a
    #     function, a field or a declaration bound to this type: an item of the
    #     type that ports it, or an item of a porting file whose owner claims
    #     no unrelated Lucene type. A free function or a field is accepted at
    #     file scope only when every Lucene type its file ports is related to
    #     this one, since nothing else binds it to the type.
    sites = []
    for rt, rf in rtypes:
        for n, (w, d) in S.item_fns(rt, rf).items():
            sites.append((f"{n} in {w}", d, True))
        for f, fd in S.fields(rt, rf):
            sites.append((f"field {rt}.{fd['name']} in {f}:{fd['line']}", fd["doc"], True))
    file_only_related = {f: ctx["file_classes"].get(f, set()) <= related for f in files}
    for f in files:
        for n, fsites in S.file_fns.get(f, {}).items():
            for owner, doc in fsites:
                own_ty = (owner.split(" for ", 1)[1].strip() if " for " in owner
                          else owner[5:].strip() if owner.startswith("impl ")
                          else owner[6:].strip() if owner.startswith("trait ") else None)
                if own_ty is None:
                    ok = file_only_related[f]
                else:
                    owned_by = claimed.get((own_ty, f), set()) | S.type_claims.get((own_ty, f), set())
                    ok = bool(owned_by & related) or (not owned_by and file_only_related[f])
                if ok:
                    sites.append((f"{n} in {f}: {owner}", doc, False))
        for what, doc in S.file_decls.get(f, ()):
            if file_only_related[f]:
                sites.append((f"{what} in {f}", doc, False))
        rec = S.tree.get(f, {})
        for tname, td in rec.get("types", {}).items():
            owned_by = claimed.get((tname, f), set()) | S.type_claims.get((tname, f), set())
            if (owned_by & related) or (not owned_by and file_only_related[f]):
                for fd in td.get("fields", ()):
                    sites.append((f"field {tname}.{fd['name']} in {f}:{fd['line']}", fd["doc"], False))
    for where, doc, own in sites:
        lead = doc_lead(doc)
        if lead and lead[1] == java and (lead[0] is None or lead[0] in doc_types):
            return ("renamed", f"doc-lead[{'own' if own else 'file'}]", where)

    # 4e. A non-public Java helper whose body the port inlined or shares: the
    #     porting file calls a function of the member's name (`read_line`,
    #     `parse_int_at`, `starts_with`) that the crate declares or the standard
    #     library provides. Private or package-private only -- neither is part
    #     of the API a client reaches, so the call in the file that ports the
    #     class is its use -- and the name must be two words or more, or be the
    #     name of a function the crate declares, so a one-word coincidence
    #     (`close(`) cannot settle it.
    # An interface's or annotation's member is implicitly public (JLS 9.4).
    if ctx["nonpublic"].get((cls, java)) and java_kind not in ("interface", "annotation"):
        cands = {c for c in name_candidates(java) if len(c) >= 3}
        declared = ctx["declared_fns"]
        for f in files:
            for ln, code in ctx["code"](f):
                for c in sorted(cands):
                    if re.search(r"(?<![\w:])(?:\.)?" + re.escape(c) + r"\s*\(", code) and \
                            ("_" in c or c in declared):
                        return ("renamed", "inlined-call[file]", f"`{c}(` called at {f}:{ln}")

    # 4d-ii. The same explicit claim written on a function of a *sibling* file
    #        of a porting file (same directory): the port moved the member to
    #        the type that uses it. `OrdsSegmentTermsEnum.initIndexInput()` is
    #        `SteCtx::init_index_input` in `ords_segment_terms_enum_frame.rs`,
    #        whose doc says "Port of `OrdsSegmentTermsEnum.initIndexInput()`".
    #        An equivalence lead-in (`RE_DECL_CLAIM`) naming this exact type is
    #        required, never a bare mention.
    dirs = {os.path.dirname(f) for f in files}
    for f in sorted(S.file_fns):
        if f in files or os.path.dirname(f) not in dirs:
            continue
        for n, fsites in S.file_fns[f].items():
            for owner, doc in fsites:
                if (simple, java) in decl_claim_members(doc):
                    return ("renamed", "doc-comment[sibling]", f"{n} in {f}: {owner}")

    # 4e-i. A body inlined where Java calls the member, and marked there with
    #       a line comment naming it: `// createFieldsConsumer` above the
    #       closure `fields_consumer` hands its shared body, which is what
    #       Java's virtual call dispatches to. A file whose ported types are all
    #       related to this one, and a comment that is the member's name alone,
    #       or its name followed by `(`, `:` or a backtick.
    marker = re.compile(r"^\s*//(?![/!])\s*`?(?:[A-Z][\w$]*[.#])?" + re.escape(java)
                        + r"(?:\s*$|\s*[(:`])")
    for f in files:
        if not file_only_related[f]:
            continue
        for ln, line in enumerate(ctx["text"](f).splitlines(), 1):
            if marker.match(line):
                return ("renamed", "inline-marker[file]", f"{f}:{ln}: {line.strip()[:80]}")

    # 4e-ii. A JUnit hook registered by name in the framework's class model:
    #        Lucene's `@BeforeClass createPostings()` is found by reflection,
    #        and the port's class model declares it as
    #        `Method::new("createPostings")` with the body as its closure.
    reflective = re.compile(r"\bMethod::new\(\s*\"" + re.escape(java) + r"\"")
    for f in files:
        for ln, code in ctx["code"](f):
            if reflective.search(code):
                return ("renamed", "reflective-method[file]", f"{f}:{ln}")

    # 4e-iii. A no-argument factory a subclass overrides (`protected abstract
    #         T newPosition()`), answered by `Default`: the port's position
    #         type is `T: Default` or implements it, so `T::default()` is the
    #         factory. The factory's name is `new`/`create` + the type's name,
    #         and that type (or a trait of that name with a `Default` bound) is
    #         declared in a porting file.
    fm = re.match(r"(?:new|create)([A-Z]\w*)$", java)
    if fm and ctx["noarg"].get((cls, java)):
        suffix = fm.group(1)
        # The porting files, then those of a Java supertype: an override of
        # `LookaheadTokenFilter.newPosition` uses the `Default` bound the base
        # declares.
        sup_files = []
        for st, sf in supertypes:
            sup_files += [sf] + list(ctx["files"].get(ctx["port_of"].get((st, sf)), ()))
        for f in list(files) + [x for x in sup_files if x not in files]:
            rec = S.tree.get(f, {})
            for t in rec.get("traits", ()):
                if t["name"].endswith(suffix) and "Default" in t["supers"]:
                    return ("renamed", "default-factory[file]",
                            f"trait {t['name']}: Default in {f}:{t['line']}")
            for im in rec.get("impls", ()):
                if im["trait"] == "Default" and (im["self"] or "").endswith(suffix):
                    return ("renamed", "default-factory[file]",
                            f"impl Default for {im['self']} in {f}:{im['line']}")

    # 4f. A declared divergence (`CLAUDE.md` 14.5) of a type that ports this
    #     one, stating that the port delegates where Java wraps: the type's doc
    #     names, as `[`T`]` or `` `T` ``, the Rust type that answers the
    #     member, and that type declares it. `FilterPath`'s doc says its path is
    #     a core `Path` of a filter filesystem and every method delegates, and
    #     `Path` declares `resolve`, `normalize` and the rest.
    delegate_files = set()
    for rt, rf in rtypes:
        tdoc = " ".join(td["doc"] for f2, td in S.type_decl.get(rt, ()) if f2 == rf or rf is None)
        if not RE_DIVERGENCE.search(tdoc):
            continue
        # In the order the doc names them: the first type a divergence names
        # is the one it says the port is (`FilterPath`: "this port's [`Path`]").
        for target in dict.fromkeys(t for t in re.findall(r"`([A-Z][A-Za-z0-9_]*)`", tdoc)
                                    if t != rt):
            if target not in S.type_decl and target not in S.trait_decls:
                continue
            delegate_files.update(f2 for f2, _td in S.type_decl.get(target, ()))
            delegate_files.update(f2 for f2, _td in S.trait_decls.get(target, ()))
            have = S.item_fns(target)
            if java in SHAPES:
                rule_, prefixes = SHAPES[java]
                for h in sorted(have):
                    if h in prefixes or any(h.startswith(pre) for pre in prefixes
                                            if pre.endswith("_")):
                        return ("renamed", "delegated[divergence]",
                                f"{rt} delegates to {target}::{h} ({have[h][0]}, {rule_})")
            hit = try_names(java, have, "own", allow_expansion=False, doc_types=doc_types)
            if hit:
                return ("renamed", "delegated[divergence]",
                        f"{rt} delegates to {target}::{hit[1]} ({have[hit[1]][0]})")
            for trait, fn in CONTRACT_TRAITS.get(java, ()):
                if trait in S.traits_of(target):
                    return ("renamed", "delegated[divergence]",
                            f"{rt} delegates to {target}: impl {trait} -> {fn}")

    # 4g. A declared divergence that names the member: the port states it
    #     does not reproduce it and why, which is the approved adaptation, not
    #     open work. The paragraph must sit in a porting file whose Lucene types
    #     are all related to this one, or name this type itself.
    #     Three places are read: the porting files (a paragraph there binds by
    #     the file, or by naming this type); the files porting a Java supertype,
    #     where a paragraph binds only by naming this type (`BlockWriter`'s
    #     divergence names `STBlockWriter`'s override of the factory it
    #     inlines); and the files declaring the Rust type a divergence of 4f
    #     delegates to, where the paragraph is about that type's own surface.
    named = re.compile(r"`(?:[A-Z][\w$]*[.#])?" + re.escape(java) + r"\s*(?:\(|`)")
    names_type = re.compile(r"\b" + re.escape(simple) + r"\b")
    super_files = set()
    for st, sf in supertypes:
        super_files |= set(ctx["files"].get(ctx["port_of"].get((st, sf)), ())) | {sf}
    # The files declaring the traits the port type implements: where Java
    # extends a JDK class the port implements its emulation's trait, and that
    # trait's module declares which of the JDK surface it leaves out
    # (`java_nio_file.rs`: "the attribute-view class hierarchy
    # (`getFileAttributeView`, `getFileStoreAttributeView`, ...) ... have no
    # counterpart").
    trait_files = set()
    for rt, rf in rtypes:
        for t in S.traits_of(rt, rf):
            trait_files |= {tf for tf, _rec in S.resolve_trait(t, rf)}
        # A port that is itself a trait: its supertraits.
        trait_files |= {tf for t, tf, _rec in S.trait_closure(rt, rf) if t != rt}
    for scope, fs in (("file", files), ("super", sorted(super_files - set(files))),
                      ("delegate", sorted(delegate_files - set(files))),
                      ("trait", sorted(trait_files - set(files) - delegate_files))):
        for f in fs:
            # A divergence paragraph that introduces a list ("The differences
            # that span the module are:") carries its bullets with it.
            in_div = False
            for ln, para in ctx["paragraphs"](f):
                in_div = bool(RE_DIVERGENCE_REF.search(para)) or (
                    in_div and para.lstrip().startswith("* "))
                if not in_div or not named.search(para):
                    continue
                if scope == "file" and not (file_only_related[f] or names_type.search(para)):
                    continue
                if scope == "super" and not names_type.search(para):
                    continue
                return ("na", f"declared-divergence[{scope}]", f"{f}:{ln}")

    # A `private` Java method is not part of any contract a caller can reach:
    # it exists to factor the body of a method that is. When every member of
    # its type that *is* reachable has been discharged, its absence is a
    # difference in decomposition, not in functionality.
    if ctx["private"].get((cls, java)) and cls in ctx["contract_clean"]:
        return ("na", "private-helper",
                f"private in Java; the reachable surface of "
                f"{cls.rsplit('.', 1)[-1]} carries no absence")
    return ("absent", "no-rule", "")


# --------------------------------------------------------------------------

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("snapshot")
    ap.add_argument("out")
    ap.add_argument("--root", default=".")
    ap.add_argument("--exclusions", default=str(Path(__file__).parent / "port_gaps_exclusions.json"),
                    help="the curated member decisions (default: port_gaps_exclusions.json)")
    args = ap.parse_args()

    S = Path(args.snapshot)
    load = lambda n: json.loads((S / f"{n}.json").read_text(encoding="utf-8"))
    root = Path(args.root).resolve()

    unmatched = json.loads((S / "unmatched.json").read_text(encoding="utf-8"))
    meta = {r[0]: {"package": r[1], "name": r[2]} for r in load("class_meta")}

    # Rust types that declare they port each Lucene type, and the files holding
    # them. `PORTS_CANDIDATE` is the weaker single-name-coincidence edge; it is
    # included here because the question is only "does an item discharge this",
    # and the tier the match lands in records how strong the binding was.
    rtypes, files = defaultdict(list), defaultdict(set)
    for qn, f, n, kind in load("ports_any"):
        if n:
            rtypes[qn].append((n, f))
        if f:
            files[qn].add(f)

    supers = defaultdict(set)
    for a, b in load("supers"):
        supers[a].add(b)

    # Supertypes contribute their Rust *items* (the `super` tier), never their
    # files. Admitting the file was how `BinaryDocValuesFieldUpdates.grow` came
    # to be answered by `BytesRefBuilder::grow`: its supertype `Accountable` is
    # ported in `src/util.rs`, so the whole of `src/util.rs` became evidence
    # for it. A large module ports dozens of types and its file scope would
    # then answer any of their members with any of the others'.
    srtypes = defaultdict(list)
    for qn in list(rtypes) + list(files):
        seen, frontier = {qn}, list(supers.get(qn, ()))
        while frontier:
            p = frontier.pop()
            if p in seen:
                continue
            seen.add(p)
            srtypes[qn] += rtypes.get(p, [])
            frontier += list(supers.get(p, ()))

    # The macro file that stands in for Lucene's code generator declares the
    # port of every type its sibling module invokes it for. And in Rust `foo.rs`
    # and the directory `foo/` are one module, so a type ported by
    # `src/util/packed.rs` is also ported by `src/util/packed/*.rs`:
    # `PackedInts.getDecoder` lives in `src/util/packed/bulk_operation.rs`.
    for qn in list(files):
        for p in list(files[qn]):
            sib = os.path.join(os.path.dirname(p), "macros.rs")
            if (root / sib).exists():
                files[qn].add(sib)
            submod = root / p[:-3]
            if p.endswith(".rs") and submod.is_dir():
                for child in sorted(submod.glob("*.rs")):
                    files[qn].add(str(child.relative_to(root)))

    # Simple names a type may be credited with in a doc comment: its own and
    # every Java supertype's, transitively.
    all_supers = defaultdict(set)
    for a, b in load("supers_all"):
        all_supers[a].add(b)
    doctypes = {}
    for qn in {u["cls"] for u in unmatched}:
        names = {qn.rsplit(".", 1)[-1], qn.rsplit(".", 1)[-1].split("$")[-1]}
        seen, frontier = {qn}, list(all_supers.get(qn, ()))
        while frontier:
            p2 = frontier.pop()
            if p2 in seen:
                continue
            seen.add(p2)
            names.add(p2.rsplit(".", 1)[-1])
            names.add(p2.rsplit(".", 1)[-1].split("$")[-1])
            frontier += list(all_supers.get(p2, ()))
        doctypes[qn] = names

    overloads, private = Counter(), {}
    for qn, name, kind, mods, sig, mqn, rt in load("member_kinds"):
        if kind != "method":
            continue
        overloads[(qn, name)] += 1
        # Every declaration of the name must be private: an overload set with
        # one public member is reachable, so it is not a private helper.
        priv = "private" in (mods or "")
        private[(qn, name)] = priv and private.get((qn, name), True)

    # Which Lucene types each Rust item claims, and which Lucene types count
    # as the same contract for the purpose of that claim (self, supertypes,
    # subtypes -- an override is the same contract as the declaration).
    claimed = defaultdict(set)
    for qn, f, n, kind in load("ports_any"):
        if n and f:
            claimed[(n, f)].add(qn)
    subs = defaultdict(set)
    for a, b in load("supers_all"):
        subs[b].add(a)
    related = {}
    all_classes = set(meta)
    for qn in {u["cls"] for u in unmatched}:
        rel, frontier = {qn}, [qn]
        while frontier:
            x = frontier.pop()
            for y in list(all_supers.get(x, ())) + list(subs.get(x, ())):
                if y not in rel:
                    rel.add(y)
                    frontier.append(y)
        # A type and the types nested in it, or enclosing it, answer for each
        # other: the port routinely puts an outer type's member on the Rust
        # type of an inner one and the reverse, and since nested types are
        # triaged too (`DEPTH_PAIRS`), a Rust item porting `Outer$Inner` must not
        # read as "another type's" item to `Outer`.
        head = qn.split("$", 1)[0]
        rel |= {c for c in all_classes if c == head or c.startswith(head + "$")}
        related[qn] = rel

    tree = parse_tree(root)
    kinds = {r[0]: r[3] for r in load("class_meta")}
    file_classes = defaultdict(set)
    for qn, f, n, kind in load("ports_any"):
        if f:
            file_classes[f].add(qn)
    declared_fns = set()
    for rec in tree.values():
        declared_fns |= {fn["name"] for fn in rec["free"]}
        for blk in rec["impls"] + rec["traits"]:
            declared_fns |= {fn["name"] for fn in blk["fns"]}
    text_cache = {}
    port_of = {}
    for qn, f, n, kind in load("ports_any"):
        if n and f:
            port_of.setdefault((n, f), qn)
    nonpublic, noarg = {}, {}
    for qn, name, kind, mods, sig, mqn, rt_ in load("member_kinds"):
        if kind != "method":
            continue
        m_ = set((mods or "").split())
        np_ = not (m_ & {"public", "protected"})
        nonpublic[(qn, name)] = np_ and nonpublic.get((qn, name), True)
        noarg[(qn, name)] = noarg.get((qn, name), False) or bool(re.search(r"\(\s*\)", sig or ""))

    def text_of(f):
        if f not in text_cache:
            try:
                text_cache[f] = (root / f).read_text(encoding="utf-8", errors="replace")
            except OSError:
                text_cache[f] = ""
        return text_cache[f]

    ctx = {"surface": Surface(tree), "claimed": claimed, "kinds": kinds,
           "file_classes": file_classes, "declared_fns": declared_fns,
           "code": lambda f: code_lines(text_of(f)),
           "text": text_of, "port_of": port_of, "nonpublic": nonpublic, "noarg": noarg,
           "paragraphs": lambda f: doc_paragraphs(text_of(f)), "related": related, "rtypes": rtypes, "srtypes": srtypes,
           "files": {k: sorted(v) for k, v in files.items()},
           "doctypes": doctypes, "overloads": overloads, "private": private,
           "contract_clean": set(),
           "excluded": json.loads(Path(args.exclusions).read_text(encoding="utf-8"))}

    # Two passes: the private-helper rule needs to know whether anything
    # *reachable* on the same type is still absent, which the first pass
    # decides. Running it before that would be a guess.
    first = {}
    for pair in unmatched:
        first[(pair["cls"], pair["method"])] = classify(pair, ctx)
    dirty = {c for (c, m), (k, _r, _e) in first.items()
             if k == "absent" and not private.get((c, m))}
    ctx["contract_clean"] = {c for c, _m in first} - dirty

    out = []
    for pair in unmatched:
        klass, rule, evidence = classify(pair, ctx)
        m = meta.get(pair["cls"], {})
        pkg = m.get("package", "")
        parts = pkg.split(".")
        out.append({
            "type": pair["cls"], "method": pair["method"],
            "package": pkg,
            "pkgTop": ".".join(parts[:4]) if len(parts) >= 4 else pkg,
            "class": klass, "rule": rule, "evidence": evidence,
            "tier": ("own" if "[own]" in rule else
                     "super" if "[super]" in rule else
                     "file" if "[file]" in rule else "-"),
        })
    Path(args.out).write_text(json.dumps(out, indent=1), encoding="utf-8")

    byc = Counter(o["class"] for o in out)
    print(f"classified {len(out)}: " +
          ", ".join(f"{k} {v}" for k, v in byc.most_common()))
    print("\nby rule:")
    for (k, r), v in sorted(Counter((o["class"], o["rule"]) for o in out).items(),
                            key=lambda kv: -kv[1]):
        print(f"  {v:5d}  {k:8s} {r}")
    print("\nabsent by package:")
    for k, v in Counter(o["pkgTop"] for o in out
                        if o["class"] == "absent").most_common():
        print(f"  {v:5d}  {k}")


if __name__ == "__main__":
    main()
