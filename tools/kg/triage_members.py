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
RE_DOC_MEMBER = re.compile(
    r"([A-Z][A-Za-z0-9_]*)\s*[#.]\s*([a-z][A-Za-z0-9_]*)\s*[(`]")
RE_DOC_HASH = re.compile(r"([A-Z][A-Za-z0-9_]*)#([a-z][A-Za-z0-9_]*)")


def _doc_above(rec, trait):
    """The doc comment of a trait declaration, which `parse` keeps per file."""
    return rec.get("trait_docs", {}).get(trait["name"], "")


def doc_members(doc):
    """{(java type simple name, java member name)} the doc text names."""
    if not doc:
        return set()
    return set(RE_DOC_MEMBER.findall(doc)) | set(RE_DOC_HASH.findall(doc))


# The crate states which Lucene type an item ports in its own doc comment
# (`CLAUDE.md` 14.1). Reading that from the source is stronger than reading the
# `PORTS` edges, which exist only for the claims the graph loader resolved.
RE_CLAIM = re.compile(
    r"(?:Equivalent to|Port of|Ported from|ported from)\s+(?:the\s+)?"
    r"(?:abstract |final |sealed |package-private |static |nested |private )*"
    r"(?:class |interface |enum |record )?"
    r"`(org\.apache\.lucene\.[A-Za-z0-9_.$]+)`")


class Surface:
    """Everything the crate declares, indexed the way the triage asks for it."""

    def __init__(self, tree):
        self.tree = tree
        self.trait_decls = defaultdict(list)  # trait name -> [(file, record)]
        self.impls_of = defaultdict(list)   # rust type name -> [(file, impl)]
        self.type_decl = defaultdict(list)  # rust type name -> [(file, record)]
        self.file_fns = {}            # file -> {fn name -> [(where, doc)]}
        self.type_claims = defaultdict(set)   # (rust type, file) -> {lucene fqn}
        for f, rec in tree.items():
            for t in rec["traits"]:
                self.trait_decls[t["name"]].append((f, t))
                for fqn in RE_CLAIM.findall(
                        _doc_above(rec, t) or ""):
                    self.type_claims[(t["name"], f)].add(fqn.rstrip("."))
            for im in rec["impls"]:
                self.impls_of[im["self"]].append((f, im))
            for name, td in rec["types"].items():
                self.type_decl[name].append((f, td))
                for fqn in RE_CLAIM.findall(td["doc"] or ""):
                    self.type_claims[(name, f)].add(fqn.rstrip("."))
            # Every declaration site, not just the first: `standard.rs`
            # declares `set_max_token_length` on three types, and keeping only
            # the first hid the one that answers `StandardTokenizer` behind the
            # one that answers `StandardTokenizerImpl`.
            got = defaultdict(list)
            for fn in rec["free"]:
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

    def traits_of(self, rtype, rfile=None):
        """Trait names `rtype` implements, plus their supertraits and derives."""
        out = set()
        for f, im in self.impls_of.get(rtype, ()):
            if im["trait"]:
                out.add(im["trait"])
                for t, _tf, _rec in self.trait_closure(im["trait"], f):
                    out.add(t)
        for f, td in self.type_decl.get(rtype, ()):
            out |= set(td["derives"])
        return out

    def derived_traits(self, rtype):
        out = set()
        for f, td in self.type_decl.get(rtype, ()):
            out |= {d for d in td["derives"]}
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
        # The port of a Lucene interface is a Rust trait, so when the item
        # itself is a trait its own declarations are the members.
        for t, f, rec in self.trait_closure(rtype, near):
            for fn in rec["fns"]:
                got.setdefault(fn["name"], (f"{f}: trait {t}", fn["doc"]))
        for f, im in self.impls_of.get(rtype, ()):
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
    doc_types = ctx["doctypes"].get(cls, {cls.rsplit(".", 1)[-1]})
    overloaded = ctx["overloads"].get((cls, java), 1) >= 2

    # 0. A member the port has decided against. `port_gaps_exclusions.json`
    #    records the decision and, since this triage, whether it is the absence
    #    of a Java construct (nothing to discharge) or work deferred (still an
    #    absence). Only the first is `na`.
    exc = ctx["excluded"].get(cls)
    if exc and java in exc["methods"] and exc.get("triageClass") == "na":
        return ("na", "excluded-construct", exc["triageReason"])

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
            derived = S.derived_traits(rt)
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
    simple, multiword = cls.rsplit(".", 1)[-1], "_" in camel_to_snake(java)
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
                    if not (own_ty and (simple in own_ty or own_ty in simple)):
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
        names, seen, frontier = {qn.rsplit(".", 1)[-1]}, {qn}, list(all_supers.get(qn, ()))
        while frontier:
            p2 = frontier.pop()
            if p2 in seen:
                continue
            seen.add(p2)
            names.add(p2.rsplit(".", 1)[-1])
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
    for qn in {u["cls"] for u in unmatched}:
        rel, frontier = {qn}, [qn]
        while frontier:
            x = frontier.pop()
            for y in list(all_supers.get(x, ())) + list(subs.get(x, ())):
                if y not in rel:
                    rel.add(y)
                    frontier.append(y)
        related[qn] = rel

    tree = parse_tree(root)
    ctx = {"surface": Surface(tree), "claimed": claimed, "related": related, "rtypes": rtypes, "srtypes": srtypes,
           "files": {k: sorted(v) for k, v in files.items()},
           "doctypes": doctypes, "overloads": overloads, "private": private,
           "contract_clean": set(),
           "excluded": json.loads(
               (Path(__file__).parent / "port_gaps_exclusions.json")
               .read_text(encoding="utf-8"))}

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
