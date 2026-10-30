#!/usr/bin/env python3
"""Port evidence the crate states about itself.

`CLAUDE.md` 14.1 requires every ported item to name its Lucene Core equivalent in
its doc comment, and the crate does: 852 `Equivalent to
`org.apache.lucene...`` claims across 530 files at 41051f8. That is an
*assertion by the port*, attached to one specific Rust item -- categorically
stronger than the `PORTS_CANDIDATE` heuristic, which only knows that exactly one
Rust type somewhere in the crate happens to share a simple name.

This tool reads those assertions and turns them into `PORTS` edges carrying
`evidence: "doc-comment"`, so that the answer to "what is ported" rests on what
the code declares rather than on a name coincidence. Curated edges keep
`evidence: "curated"` and always win.

It is deliberately conservative (`CLAUDE.md` 7): only the explicit lead-ins below
count as a claim. A bare `/// `org.apache.lucene.Foo`` mention is a cross
reference, not a statement that this item is Foo, and is ignored.

Two forms beyond the original lead-ins are read, both because the crate uses them
and both without loosening what counts as an assertion:

* **`<subject>, ported from `org.apache.lucene.Foo`.`** -- the crate's usual
  module-doc phrasing (121 types at 82c5b19). It asserts identity exactly as
  `Port of` does, only with the subject stated first, so `MODULE_ASSERTIONS`
  accepts it. Module docs are read a *paragraph* at a time rather than a line at
  a time, because the assertion and the name it asserts are routinely split
  across two `//!` lines.
* **a module claim resolved to a declared item** -- see `load`, which states the
  resolution rule. Before this, a `//!` claim produced no edge at all, so a file
  whose module doc named its Lucene type while its items did not was invisible
  to coverage; the type sat in `candidate` and was left out of the depth
  measurement, understating the port and misdirecting the work `CLAUDE.md` 6.2
  says the graph must direct.

Usage:
    python3 tools/kg/port_evidence_kg.py rucene --source-root . \
        --commit "$COMMIT" --date "$DATE" [--phase extract|load|all]
"""
import argparse
import json
import os
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from kgio import PER_ROW_SCAN_BATCH, kg_target, read as kg_read, unwind

# A Lucene type named in full, and the short Java spelling the crate uses for a
# *nested* one. `CLAUDE.md` 14.1 asks each item to name its Lucene equivalent; it
# does not prescribe the spelling, and for an inner class the crate writes what a
# Java reader writes -- ``Equivalent to `FreqProxFields.FreqProxDocsEnum``` --
# because the package is already established by the file around it. Requiring the
# `org.apache.lucene.` prefix therefore did not make the evidence stricter, it
# made 319 declared ports of nested types unreadable, which is precisely the
# assumption `CLAUDE.md` 6.1 forbids a denominator to rest on.
#
# The short form is admitted without loosening what counts as a claim. It must be
# a chain of two or more segments each beginning with a capital -- Lucene packages
# are lower-case, so such a chain can only name `Outer.Inner`, never `pkg.Type`
# and never `Type.member` -- and `resolve_lucene` then accepts it *only* when
# exactly one type in the graph carries that dotted suffix. Measured over
# lucene/core 10.5.0 that rule is not merely usually unambiguous: of every
# two-or-more-segment capitalised suffix in the module, **zero** are ambiguous,
# and **zero** nested types share a name with a member of their own enclosing
# type. So the short spelling denotes one type or no type, never a guess between
# two, and never a member mistaken for a type (`CLAUDE.md` 7).
FQN_FULL = r"org\.apache\.lucene\.[A-Za-z0-9_.$]+"
NESTED_SHORT = r"[A-Z][A-Za-z0-9_]*(?:[.$][A-Z][A-Za-z0-9_]*)+"
# The crate often quotes the Java declaration rather than just the name --
# ``Equivalent to the `final class MultiTermQuery.TopTermsScoringBooleanQueryRewrite`.``
# or ``the record `BayesianScoreEstimator.Parameters(float alpha, ...)``` -- so the
# modifiers and the declaring keyword sit *inside* the backticks. That is the same
# assertion with more of Java quoted, not a weaker one, so the span is allowed to
# carry them. Only Java's own modifier and declaration keywords are tolerated; any
# other word before the name still refuses the match.
_QUOTED_MODS = (r"(?:(?:abstract|final|sealed|non-sealed|static|public|protected|"
                r"private|package-private|strictfp)\s+)*"
                r"(?:(?:class|interface|enum|record|annotation)\s+)?")
# A trailing type-parameter list or parameter list inside the span is part of the
# Java declaration being quoted, not part of the name: ``Equivalent to
# `PairOutputs.Pair<A, B>``` and ``the `FST.FSTMetadata` constructor`` name the
# same types as the bare spellings do.
FQN = (r"`" + _QUOTED_MODS + r"(" + FQN_FULL + r"|" + NESTED_SHORT + r")"
       r"(?:<[^`]*>)?(?:\([^`]*\))?`")

# The lead-in is matched case-insensitively; the *name* never is, because the
# short form's whole guard is that every segment begins with a capital. `re.I` on
# the whole pattern would have silently voided it and let `ArrayUtil.copyArray`
# through as a type. Scoped `(?i:...)` keeps the two apart.
_CI = lambda pat: "(?i:" + pat + ")"

# Lead-ins that assert "this item is the port of that Lucene type". Ordered
# most specific first; each is anchored at the start of the doc text.
CLAIM_PATTERNS = [
    # `public`/`protected` and `functional interface` are spellings the crate
    # writes for a type claim (`Equivalent to the protected abstract class
    # ...TieredMergePolicy.MergeScore`, `Equivalent to the functional interface
    # ...IndexWriter.IndexReaderWarmer`); a functional interface is an interface.
    re.compile("^" + _CI(r"Equivalent to (?:the )?"
               r"(?:abstract |final |sealed |package-private |private |protected |public "
               r"|static |nested |inner )*"
               r"(?:class |functional interface |interface |enum |record |annotation )?") + FQN),
    re.compile("^" + _CI(r"Port of (?:the )?"
               r"(?:abstract |final |sealed |package-private |private |static |nested |inner )*"
               r"(?:class |interface |enum |record |annotation )?") + FQN),
    re.compile("^" + _CI(r"Ported from ") + FQN),
    re.compile("^" + _CI(r"Lucene Core equivalent:\s*") + FQN),
    re.compile(r"^(?:This module )?[Pp]orts " + FQN),
]
# An explicit statement that the port is NOT complete. Recorded as a note on the
# edge rather than silently promoting the type to `ported`.
PARTIAL_PATTERNS = [
    re.compile("^" + _CI(r"Placeholder for ") + FQN),
]
# A module doc states its subject in prose rather than with a lead-in: "Block-max
# conjunction scoring, ported from `org.apache.lucene.search.BlockMaxConjunctionScorer`."
# The assertion is as explicit as `Port of`, so it counts -- but only as a claim
# about the *file*; which type inside it the claim lands on is decided in `load`
# by the resolution rule documented there, never by this regex.
# The same assertion read on an item doc. An item doc states its own target --
# the declaration it precedes -- so the paragraph is admitted only when the name
# it asserts is *that* declaration's name. That guard is what keeps a sentence
# like "Equivalent to the inherited `Enum#toString` of `pkg.CompressionAlgorithm`"
# on a method from being read as a claim about the enum.
ITEM_ASSERTIONS = [
    re.compile(r"\b(?:ported from|equivalent to|port of|ports)\b", re.I),
    re.compile(r"^\s*Lucene Core equivalent:", re.I),
]
# A paragraph that *denies* an equivalence uses the same words as one that
# asserts it -- "not declared as the port of `org.apache.lucene.store.IOContext`"
# -- so reading the words alone would turn a denial into evidence. A paragraph
# carrying a negation is therefore refused outright rather than parsed: refusing
# a real claim costs a type its evidence, while accepting a denial puts a
# falsehood in the graph, and `CLAUDE.md` 7 does not trade those off.
NEGATION = re.compile(
    r"\b(?:not|never|no longer|neither|nor|rather than|instead of|unlike|"
    r"isn't|aren't|doesn't)\b", re.I)


def _clause_before(text: str, idx: int) -> str:
    """The sentence in which the name at `idx` is introduced.

    Both the assertion and the negation are judged on this clause and not on the
    whole paragraph, because a paragraph that legitimately asserts a port
    routinely goes on to say what the port does *not* do -- `src/util/selector.rs`
    states its claim in the first sentence and qualifies it three sentences
    later. Judging the paragraph would throw that evidence away.
    """
    masked, inside = [], False
    for ch in text[:idx]:
        if ch == "`":
            inside = not inside
        masked.append("_" if (inside and ch == ".") else ch)
    masked = "".join(masked)
    start = 0
    # `:` does not end the clause: the crate's own canonical form is
    # "Lucene Core equivalent: `pkg.Foo`.", where the assertion sits before the
    # colon and the name after it.
    for m in re.finditer(r"[.;]\s", masked):
        start = m.end()
    return text[start:idx]
# The same set serves module docs. A module doc asserts in the same words as an
# item doc -- "This module ports `pkg.FieldInfo` and `pkg.FieldInfos`" -- and the
# guard that keeps a module claim honest is not the wording but the resolution
# rule in `load`: it lands only on an item of that exact name declared by that
# same file, or on nothing at all.
MODULE_ASSERTIONS = ITEM_ASSERTIONS
RE_FQN = re.compile(FQN)

RE_DOC_OUTER = re.compile(r"^\s*///!?(.*)$")
RE_DOC_INNER = re.compile(r"^\s*//!(.*)$")
RE_ATTR = re.compile(r"^\s*#\[")
# `const` and `static` are items too, and the crate uses them to port Lucene's
# constants -- ``Equivalent to `TokenizerFactory.Holder.LOADER``` on
# `static TOKENIZER_LOADER`. Such a claim names a *member*, which `load` already
# knows how to resolve to the type that declares it, so admitting these items
# adds no new kind of inference; leaving them out simply threw the claim away.
RE_ITEM = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?"
    r"(?:default\s+|async\s+|unsafe\s+|extern\s+\"[^\"]*\"\s+)*"
    r"(struct|enum|trait|union|type|fn|const|static)\s+([A-Za-z_]\w*)"
)


def build_class_index(all_classes):
    """`(exact, short)` name indexes for resolving a claim to a Lucene type.

    `exact` accepts the graph's own `pkg.Outer$Inner` key and the `pkg.Outer.Inner`
    spelling a doc comment writes. `short` accepts the bare `Outer.Inner` the crate
    uses for a nested type, and holds only the dotted suffixes that identify
    exactly one type -- so a short name resolves positively or not at all
    (`CLAUDE.md` 7). See `NESTED_SHORT` for why that is not a name-match guess.
    """
    exact = {c: c for c in all_classes}
    for c in all_classes:
        if "$" in c:
            exact.setdefault(c.replace("$", "."), c)
    suffixes = {}
    for c in all_classes:
        segs = re.split(r"[.$]", c)
        for k in range(2, len(segs) + 1):
            suffixes.setdefault(".".join(segs[-k:]), set()).add(c)
    short = {k: next(iter(v)) for k, v in suffixes.items() if len(v) == 1}
    return exact, short


def resolve_type(name, exact, short):
    """The Lucene type a claim names, or `None`. Never a guess."""
    hit = exact.get(name)
    if hit is not None:
        return hit
    if re.fullmatch(NESTED_SHORT, name):
        return short.get(name.replace("$", "."))
    return None


def claim_in(text: str):
    for pat in CLAIM_PATTERNS:
        m = pat.match(text.strip())
        if m:
            return m.group(1), None
    for pat in PARTIAL_PATTERNS:
        m = pat.match(text.strip())
        if m:
            return m.group(1), "declared a placeholder, not a complete port"
    return None, None


# The crate's canonical mapping table, written in a module doc:
#
#     //! | Rucene | Apache Lucene Core 10.5.0 |
#     //! | --- | --- |
#     //! | [`PagedBytesReader`] | `PagedBytes.Reader` |
#
# The header names both sides, so a row *is* the equivalence statement -- the
# most explicit form the crate has, and the one it uses where a whole family of
# nested types is renamed at once. Reading it needs no new judgement: the header
# is matched literally, the Rust side must be an item that file declares (the
# rule module claims already run under), and the Lucene side is admitted only in
# the same two spellings every other claim is -- a full `org.apache.lucene…` name
# or a capitalised `Outer.Inner` chain. A bare simple name in a table row is
# still refused, so the table can never resolve a name by coincidence.
RE_TABLE_HEADER = re.compile(r"^\s*\|\s*Rucene\s*\|\s*Apache Lucene Core 10\.5\.0\s*\|\s*$")
RE_TABLE_ROW = re.compile(r"^\s*\|\s*\[`([^`\]]+)`\](?:\([^)]*\))?\s*\|\s*`([^`]+)`\s*\|\s*$")


def _table_claims(rel, doc_lines):
    """Claims stated by the module doc's canonical mapping table."""
    out, inside = [], False
    for ln, text in doc_lines:
        if RE_TABLE_HEADER.match(text):
            inside = True
            continue
        if not inside:
            continue
        m = RE_TABLE_ROW.match(text)
        if m:
            java = re.sub(r"<[^>]*>", "", m.group(2)).split("(")[0].split("#")[0].strip()
            if re.fullmatch(FQN_FULL, java) or re.fullmatch(NESTED_SHORT, java):
                out.append({"file": rel, "line": ln, "target": m.group(1),
                            "lucene": java, "scope": "table", "note": None})
        elif not text.strip().startswith("|"):
            inside = False
    return out


def scan_file(root: Path, rel: str):
    """Equivalence claims of one file, attributed to the item they document."""
    out = []
    lines = (root / rel).read_text(encoding="utf-8", errors="ignore").split("\n")
    out.extend(_table_claims(rel, [(i + 1, m.group(1).strip())
                                   for i, line in enumerate(lines)
                                   if (m := RE_DOC_INNER.match(line))]))

    # Module-level `//!` claims describe the file itself. They are read a
    # paragraph at a time, not a line at a time, because the assertion and the
    # name it asserts are routinely split across two lines:
    #
    #     //! Boosting a wrapped query, ported from
    #     //! `org.apache.lucene.search.BoostQuery`.
    #
    # A paragraph that asserts a port names every Lucene type that file ports,
    # so each name in it becomes a claim about the file.
    para, para_line = [], None
    for i, line in enumerate(lines + [""]):
        m = RE_DOC_INNER.match(line)
        if m:
            text = m.group(1).strip()
            if text:
                para.append(text)
                if para_line is None:
                    para_line = i + 1
                continue
        joined = " ".join(para)
        if joined:
            note = None
            for pat in PARTIAL_PATTERNS:
                if pat.match(joined):
                    note = "declared a placeholder, not a complete port"
            asserted, seen = False, set()
            for m in RE_FQN.finditer(joined):
                clause = _clause_before(joined, m.start())
                if any(a.search(clause) for a in MODULE_ASSERTIONS):
                    if NEGATION.search(clause):
                        continue
                    asserted = True
                elif not asserted:
                    # A name listed before any assertion is a cross reference.
                    continue
                if m.group(1) in seen:
                    continue
                seen.add(m.group(1))
                out.append({"file": rel, "line": para_line, "target": None,
                            "lucene": m.group(1), "scope": "module",
                            "note": note})
        para, para_line = [], None
        if not m and line.strip() and not RE_ATTR.match(line):
            break

    # Item-level `///` claims describe the declaration that follows the block.
    block = []
    for i, line in enumerate(lines):
        m = RE_DOC_OUTER.match(line)
        if m:
            block.append((i + 1, m.group(1)))
            continue
        if RE_ATTR.match(line) or not line.strip():
            if not line.strip():
                block = []
            continue
        if block:
            item = RE_ITEM.match(line)
            if item:
                seen = set()
                for ln, text in block:
                    fqn, note = claim_in(text)
                    if fqn:
                        seen.add(fqn)
                        out.append({"file": rel, "line": ln,
                                    "target": item.group(2),
                                    "targetKind": item.group(1),
                                    "lucene": fqn, "scope": "item",
                                    "note": note})
                for ln, fqn in _item_paragraph_claims(block, item.group(2)):
                    if fqn not in seen:
                        seen.add(fqn)
                        out.append({"file": rel, "line": ln,
                                    "target": item.group(2),
                                    "targetKind": item.group(1),
                                    "lucene": fqn, "scope": "item",
                                    "note": None})
            block = []
    return out


def _item_paragraph_claims(block, item_name):
    """Claims an item's doc makes across line breaks.

    `claim_in` reads one line, but the crate routinely states the assertion on
    one line and the name on the next -- "Builds [`TokenStream`]s that analyze
    text, equivalent to" / "`org.apache.lucene.analysis.Analyzer`." -- and just
    as routinely states the subject before the assertion. Reading the paragraph
    recovers both. What keeps this from becoming a guess is the name guard: only
    an FQN whose simple name is the documented declaration's own name counts.

    A paragraph that *opens* with an explicit lead-in (`CLAIM_PATTERNS`) is
    the one-line claim wrapped after the lead-in -- "Equivalent to the
    functional interface" / "`org.apache.lucene.index.IndexWriter.IndexReaderWarmer`"
    -- and is read exactly as `claim_in` reads one line: the anchored lead-in is
    the assertion, so no name guard applies, as none applies on one line.
    """
    out, para, para_line = [], [], None
    for ln, text in list(block) + [(None, "")]:
        t = text.strip()
        if t:
            para.append(t)
            if para_line is None:
                para_line = ln
            continue
        joined = " ".join(para)
        lead = next((m.group(1) for pat in CLAIM_PATTERNS
                     if (m := pat.match(joined))), None) if joined else None
        if lead:
            out.append((para_line, lead))
        elif joined:
            for m in RE_FQN.finditer(joined):
                fqn = m.group(1)
                if re.split(r"[.$]", fqn)[-1] != item_name:
                    continue
                clause = _clause_before(joined, m.start())
                if (any(a.search(clause) for a in ITEM_ASSERTIONS)
                        and not NEGATION.search(clause)):
                    out.append((para_line, fqn))
                    break
        para, para_line = [], None
    return out


def extract(root: Path):
    claims = []
    for base in ("src",):
        for dp, _d, names in os.walk(root / base):
            for fn in sorted(names):
                if fn.endswith(".rs"):
                    rel = str(Path(dp, fn).relative_to(root)).replace(os.sep, "/")
                    claims.extend(scan_file(root, rel))
    return claims


def q(roadmap, cypher):
    return kg_read(roadmap, cypher)


def run_unwind(roadmap, kind, rows, cypher, label, batch=200):
    unwind(roadmap, kind, rows, cypher, label, batch)


def load(roadmap, claims, commit, date):
    """Write the declared equivalences as `PORTS` edges.

    An item-level `///` claim states its own target: the declaration it
    documents. A `//!` module claim describes a file, so it names a type only
    indirectly, and attributing it by guess is exactly what this graph must not
    contain. It is therefore resolved by a rule that admits one reading and no
    other: **a module claim naming `pkg.Foo` lands on the item named `Foo`
    declared by that same file.** That is not an inference about which type was
    meant -- the file says it ports `Foo` and declares `Foo` -- and it never
    reaches outside the file, so it cannot attach to a same-named type in
    another module.

    One case remains, and the model already has a node for it. Lucene's static
    utility classes are ported as a Rust *module* of free functions, so the file
    declares no type at all and there is nothing named `Foo` for the claim to
    land on: `src/util/collection_util.rs` says it ports
    `org.apache.lucene.util.CollectionUtil` and declares six functions and no
    type. `knowledge-model.md` models such a unit as a `Component` -- "a crate
    unit that declares no Rust type: a module of free functions, the port of one
    of Lucene's static utility classes" -- so **a module claim whose file
    declares no Rust type at all lands on that file's `Component`**, which is
    then the only thing the claim can be about. The rule is not a new judgement:
    run over the crate it reproduces six of the eight `Component` ports curated
    by hand -- every one whose module doc states the claim at all -- and finds
    exactly the two that were missing, `CollectionUtil` and `HashContainers`. An
    existing `Component` for the file is reused whatever it is named, so the rule
    cannot mint a second node for a module that already has one.

    The Rust end is matched on `(name, file)` -- the identity the model gives a
    Rust type and a `Component` alike -- for every kind of claim, which is also
    what lets `measure_depth` reach a module port's free functions.

    The Lucene end is a type, but a claim may legitimately name a *member* of
    one, and `CLAUDE.md` 14.1 asks for the equivalent of the documented item:
    `Equivalent to `org.apache.lucene.index.TwoPhaseCommitTool.execute`` on a
    free function is more precise than the bare type name, and the tool has to
    read it rather than the crate having to blunt it. So the FQN is tried as a
    type first and, only if that fails, against the enclosing type -- and then
    only when the graph confirms that type declares a member of that name, so
    a nested type never travels this path and an unknown name is left
    unresolved rather than guessed at.

    Such an edge records what it is: `claimLevel: "member"` and
    `declaredMember`, the Java member named. `measure_depth` reads them and
    credits the type with the declared item alone, since a claim about one
    member is not a claim about the file that holds it.
    """
    # A nested Lucene type is keyed `pkg.Outer$Inner` in the graph, but a doc
    # comment naturally writes `pkg.Outer.Inner`. Accept both spellings.
    all_classes = [r[0] for r in q(roadmap, "MATCH (c:Class) RETURN c.qualifiedName")
                   if r[0]]
    classes, short_classes = build_class_index(all_classes)
    # `CLAUDE.md` 14.1 asks each *item* to name its Lucene equivalent, and the
    # crate obliges at member granularity: `src/index/index_commit.rs:313` says
    # the free function `execute` is
    # `org.apache.lucene.index.TwoPhaseCommitTool.execute`. That FQN names no
    # type, so the claim used to be dropped -- eight of them at 9cf23e5, taking
    # `TwoPhaseCommitTool` with them. The member is nonetheless a statement
    # about a type the graph holds, so the resolution falls back to the
    # enclosing type.
    #
    # The fallback must never swallow a *nested type* claim: `TotalHits.Relation`
    # and `Sorter.DocMap` are types and resolve as types above, so the fallback
    # only ever sees an FQN the class index does not know. That alone is not
    # enough to call the last segment a member, so the fallback is positive
    # rather than residual: it fires only when the graph confirms the enclosing
    # type *declares* a member of that name. A name the graph cannot confirm
    # either way stays unresolved and is reported, per `CLAUDE.md` 7.
    members_of = {}
    for cls, mname in q(roadmap,
                        "MATCH (c:Class)-[:DECLARES]->(m:Method) "
                        "RETURN c.qualifiedName, m.name"):
        members_of.setdefault(cls, set()).add(mname)
    types = {(r[0], r[1]) for r in q(
        roadmap,
        "MATCH (t) WHERE t:RustStruct OR t:RustTrait OR t:RustEnum OR t:RustAlias "
        "RETURN t.name, t.file")}
    # Which files declare a type at all, and which already have a `Component`.
    # Both are read before the function names are folded in below, because the
    # module-as-`Component` rule turns on the absence of a *type*, not of a
    # function: a module of free functions is exactly the case it serves.
    files_with_types = {f for _n, f in types}
    component_of_file = {r[1]: r[0] for r in q(
        roadmap, "MATCH (co:Component) RETURN co.name, co.file") if r[1]}
    # A free function can be the port of a Lucene class in its own right --
    # `by_byte_size` is `LogByteSizeMergePolicy` -- so accept a RustFn target too,
    # but only where `(name, file)` identifies exactly one, since RustFn identity
    # is the qualified name and a file may hold several `new`.
    fn_counts = {}
    for name, file in q(roadmap, "MATCH (f:RustFn) RETURN f.name, f.file"):
        fn_counts[(name, file)] = fn_counts.get((name, file), 0) + 1
    types |= {k for k, n in fn_counts.items() if n == 1}

    rows, unresolved_lucene, unresolved_rust = [], set(), set()
    module_resolved = module_as_component = member_resolved = short_resolved = 0
    for c in claims:
        target, component = c.get("target"), False
        if c["scope"] == "module" and not target:
            simple = c["lucene"].replace("$", ".").rsplit(".", 1)[-1]
            if (simple, c["file"]) in types:
                target = simple
                module_resolved += 1
            elif c["file"] in files_with_types:
                continue
            else:
                target = component_of_file.get(c["file"], simple)
                component = True
                module_as_component += 1
            c = dict(c, target=target)
        if not target:
            continue
        target_class, member = classes.get(c["lucene"]), None
        if target_class is None:
            target_class = resolve_type(c["lucene"], classes, short_classes)
            if target_class is not None:
                short_resolved += 1
        if target_class is None:
            enclosing, _, last = c["lucene"].rpartition(".")
            owner = resolve_type(enclosing, classes, short_classes)
            if owner and last in members_of.get(owner, ()):
                target_class, member = owner, last
                member_resolved += 1
        if target_class is None:
            unresolved_lucene.add(c["lucene"])
            continue
        if not component and (c["target"], c["file"]) not in types:
            unresolved_rust.add((c["target"], c["file"]))
            continue
        rows.append({"name": c["target"], "file": c["file"],
                     "lucene": target_class, "line": c["line"],
                     "note": c.get("note"), "commit": commit, "date": date,
                     "component": component,
                     "claimLevel": "member" if member else "type",
                     "member": member})

    # De-duplicate: one edge per (rust item, lucene type). A claim about the
    # type wins over one about a member of it, whatever order they were read
    # in: the type claim is the stronger statement and subsumes the other.
    seen, uniq = {}, []
    for r in rows:
        k = (r["name"], r["file"], r["lucene"])
        if k not in seen:
            seen[k] = len(uniq)
            uniq.append(r)
        elif r["claimLevel"] == "type" and uniq[seen[k]]["claimLevel"] == "member":
            uniq[seen[k]] = r

    print(f"  module claims resolved to a declared item: {module_resolved}",
          file=sys.stderr)
    print(f"  module claims resolved to a Component: {module_as_component}",
          file=sys.stderr)
    print(f"  member-level claims resolved to their enclosing type: "
          f"{member_resolved}", file=sys.stderr)
    print(f"  short `Outer.Inner` claims resolved to one type: {short_resolved}",
          file=sys.stderr)
    print(f"  resolvable item claims: {len(uniq)} "
          f"(lucene unresolved {len(unresolved_lucene)}, "
          f"rust unresolved {len(unresolved_rust)})", file=sys.stderr)

    # The `Component` nodes the rule above needs. Creating one here is safe
    # because it is created only for a file the survey confirms and that
    # declares no type, and `MERGE` on the name reuses the node when it already
    # exists; a file that already has a `Component` under another name keeps it,
    # since `component_of_file` supplied that name as the target.
    comps = [{k: r[k] for k in ("name", "file", "commit", "date")}
             for r in uniq if r["component"]]
    seen_c, comp_rows = set(), []
    for r in comps:
        if r["name"] not in seen_c:
            seen_c.add(r["name"])
            comp_rows.append(r)
    run_unwind(roadmap, "create", comp_rows,
               "MERGE (co:Component {name:row.name})", "Component nodes")
    run_unwind(roadmap, "update", comp_rows,
               "MATCH (co:Component {name:row.name}) "
               "SET co.file = row.file, co.kind = 'module', "
               "co.language = 'Rust', co.gitCommit = row.commit, "
               "co.gitDate = row.date",
               "Component provenance")

    # `MERGE (a)-[:PORTS]->(b)` silently matches a relationship of a *different*
    # type between the same ordered pair (see knowledge-model.md), so a pair
    # already joined by PORTS_CANDIDATE would never gain its PORTS edge. Drop
    # the candidate edge first: a declared equivalence supersedes a name guess.
    uniq = [{k: v for k, v in r.items() if k != "component"} for r in uniq]

    # Retire the edges whose declaration has gone. `load_rucene_kg.py --phase
    # prune` deletes the *node* the survey no longer confirms, which takes its
    # edges with it; this closes the other half, where the node survives but
    # the claim does not -- a doc comment edited, moved, or deleted with its
    # file before the next survey runs. Without it a `PORTS` edge outlives its
    # own evidence and `portState` keeps reading `ported` on a claim the crate
    # no longer makes, which is the coverage inflation `CLAUDE.md` 6.1 forbids.
    #
    # Only what this pass is the authority on is retired: an edge whose
    # `evidence` is `doc-comment` and that carries no `declarationSource`, i.e.
    # one this same extraction would have produced. A `curated` edge is
    # asserted by hand and a `macro-expansion` edge is written by
    # `expanded_evidence_kg.py` from a dump this pass never reads; retiring
    # either would be deleting a fact on the strength of not having looked for
    # it. And an extraction that produced nothing retires nothing, for the same
    # reason `phase_prune` refuses an empty survey: a failed run is not
    # evidence that the crate declares no ports.
    if uniq:
        live = {(r["name"], r["file"], r["lucene"]) for r in uniq}
        dead = [{"name": n, "file": f, "lucene": l} for n, f, l in q(
            roadmap,
            "MATCH (t)-[e:PORTS]->(c:Class) WHERE e.evidence = 'doc-comment' "
            "AND e.declarationSource IS NULL "
            "RETURN t.name, t.file, c.qualifiedName")
            if (n, f, l) not in live]
        for d in dead:
            print(f"  RETIRED (no live declaration): {d['name']} @ {d['file']} "
                  f"-> {d['lucene']}", file=sys.stderr)
        run_unwind(roadmap, "delete", dead,
                   "MATCH (t {name:row.name, file:row.file})-[e:PORTS]->"
                   "(c:Class {qualifiedName:row.lucene}) DELETE e",
                   "PORTS edges retired", batch=PER_ROW_SCAN_BATCH)

    run_unwind(roadmap, "delete", uniq,
               "MATCH (t {name:row.name, file:row.file})-[e:PORTS_CANDIDATE]->"
               "(c:Class {qualifiedName:row.lucene}) DELETE e",
               "superseded PORTS_CANDIDATE removed", batch=PER_ROW_SCAN_BATCH)
    run_unwind(roadmap, "create", uniq,
               "MATCH (t {name:row.name, file:row.file}) "
               "WITH t, row MATCH (c:Class {qualifiedName:row.lucene}) "
               "MERGE (t)-[:PORTS]->(c)",
               "PORTS edges", batch=PER_ROW_SCAN_BATCH)
    run_unwind(roadmap, "update", uniq,
               "MATCH (t {name:row.name, file:row.file})-[e:PORTS]->"
               "(c:Class {qualifiedName:row.lucene}) "
               "SET e.evidence = coalesce(e.evidence, 'doc-comment'), "
               "e.declaredAt = row.file + ':' + toString(row.line), "
               "e.claimLevel = row.claimLevel, e.declaredMember = row.member, "
               "e.gitCommit = row.commit, e.gitDate = row.date",
               "PORTS evidence stamped", batch=PER_ROW_SCAN_BATCH)
    return uniq


# ---------------------------------------------------------------------------
# Depth: how much of a ported type is actually there
# ---------------------------------------------------------------------------
#
# `portState = 'ported'` says a Rust item claims to be a Lucene type. It says
# nothing about how much of that type exists, so a one-field stub and a complete
# port read identically. This phase measures the gap the only mechanical way
# available: match each Lucene member name against the ported type's Rust
# functions, under Java-to-Rust naming.
#
# The result is evidence, not proof -- one Rust method can discharge several Java
# ones, and a faithful port may deliberately name things differently -- so it is
# stored under `memberEvidence: 'name-mapping'` and must be quoted as such.

RE_CAMEL = re.compile(r"(?<!^)(?=[A-Z])")


def snake(name: str) -> str:
    return RE_CAMEL.sub("_", name).lower()


def rust_candidates(java_name: str):
    """Rust names a Java member could reasonably have been ported to."""
    s = snake(java_name)
    out = {s}
    for prefix in ("get_", "set_", "is_", "has_"):
        if s.startswith(prefix):
            out.add(s[len(prefix):])
    out.add("new" if java_name == "<init>" else s)
    return out


def measure_depth(roadmap, commit, date):
    """Member-level coverage of every in-scope Lucene type that has a port."""
    pairs = q(roadmap,
              "MATCH (t)-[e:PORTS]->(c:Class) WHERE c.portScope = 'in' "
              "RETURN c.qualifiedName, t.name, t.file, e.claimLevel")
    members = q(roadmap,
                "MATCH (c:Class)-[:DECLARES]->(m:Method) WHERE c.portScope = 'in' "
                "AND m.kind = 'method' RETURN c.qualifiedName, m.name")
    rust_fns = q(roadmap,
                 "MATCH (f:RustFn) WHERE f.scope = 'crate' "
                 "RETURN f.file, f.owner, f.name")
    # A Rust port often discharges a Lucene class's methods through
    # `impl Trait for Type` blocks, whose bodies this model does not record as
    # RustFn. Counting only inherent methods therefore reported a complete port
    # such as PackedTokenAttributeImpl -- every method of which lives in a
    # CharTermAttribute impl -- as 0%. Add the methods each implemented trait
    # declares, which the graph does hold.
    trait_methods = q(roadmap,
                      "MATCH (t)-[:IMPLEMENTS]->(tr:RustTrait) "
                      "WITH t, tr MATCH (tr)-[:DECLARES]->(m:RustFn) "
                      "RETURN t.name, t.file, m.name")
    by_impl = {}
    for tname, tfile, mname in trait_methods:
        by_impl.setdefault((tname, tfile), set()).add(mname)

    by_class = {}
    for cls, name in members:
        by_class.setdefault(cls, set()).add(name)
    by_owner = {}
    for file, owner, name in rust_fns:
        by_owner.setdefault((owner, file), set()).add(name)

    ports = {}
    for cls, tname, tfile, level in pairs:
        ports.setdefault(cls, []).append((tname, tfile, level))

    rows = []
    for cls, impls in ports.items():
        java = by_class.get(cls, set())
        if not java:
            continue
        rust = set()
        for tname, tfile, level in impls:
            rust |= by_owner.get((tname, tfile), set())
            rust |= by_impl.get((tname, tfile), set())
            if level == "member":
                # A member-level claim says this one item is the port of one
                # member -- it says nothing about the rest of the file, so
                # crediting the file's other free functions to the type would
                # state more than the crate declared. Credit the declared item
                # itself and stop.
                rust.add(tname)
                continue
            # Free functions of the same file count too: Lucene's static utility
            # methods are ported as module-level functions, with no owner.
            rust |= by_owner.get((None, tfile), set())
        matched = sum(1 for j in java if rust_candidates(j) & rust)
        rows.append({"cls": cls, "total": len(java), "matched": matched,
                     "coverage": round(matched / len(java), 3),
                     "commit": commit, "date": date})

    run_unwind(roadmap, "update", rows,
               "MATCH (c:Class {qualifiedName:row.cls}) "
               "SET c.memberTotal = row.total, c.memberMatched = row.matched, "
               "c.memberCoverage = row.coverage, "
               "c.memberEvidence = 'name-mapping', "
               "c.gitCommit = row.commit, c.gitDate = row.date",
               "member coverage")
    if rows:
        full = sum(1 for r in rows if r["coverage"] >= 0.999)
        thin = sum(1 for r in rows if r["coverage"] < 0.5)
        avg = sum(r["coverage"] for r in rows) / len(rows)
        print(f"  measured {len(rows)} ported types: mean coverage {avg:.1%}, "
              f"{full} complete, {thin} under half", file=sys.stderr)
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("roadmap")
    ap.add_argument("--source-root", default=".")
    ap.add_argument("--out", default=str(kg_target() / "rucene_kg" / "port_evidence.json"))
    ap.add_argument("--commit")
    ap.add_argument("--date")
    ap.add_argument("--phase", default="extract",
                    choices=["extract", "load", "depth", "all"])
    args = ap.parse_args()

    root = Path(args.source_root).resolve()
    if args.phase in ("extract", "all"):
        claims = extract(root)
        Path(args.out).parent.mkdir(parents=True, exist_ok=True)
        Path(args.out).write_text(json.dumps(claims, indent=1), encoding="utf-8")
    else:
        claims = json.loads(Path(args.out).read_text(encoding="utf-8"))
    item = [c for c in claims if c["scope"] == "item"]
    mod = [c for c in claims if c["scope"] == "module"]
    print(f"claims: {len(claims)} ({len(item)} item-level, {len(mod)} module-level), "
          f"distinct lucene types: {len({c['lucene'] for c in claims})}",
          file=sys.stderr)

    if args.phase in ("load", "all"):
        if not (args.commit and args.date):
            raise SystemExit("--commit and --date are required to load")
        load(args.roadmap, claims, args.commit, args.date)

    if args.phase in ("depth", "all"):
        if not (args.commit and args.date):
            raise SystemExit("--commit and --date are required to measure depth")
        measure_depth(args.roadmap, args.commit, args.date)


if __name__ == "__main__":
    main()
