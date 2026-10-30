# Knowledge Graph Model — rucene

Knowledge graph for the **Rucene** project. It represents:

1. The local **Rucene** Rust crate structure (`src/`, tests, build files), surveyed
   file by file, type by type and function by function.
2. The reference **Apache Lucene 10.5.0** structure: all 35 modules of the
   distribution, and — in depth, because it is the port target — `lucene/core`'s
   packages, subpackages, source files, types, members, and the dependencies
   between them, plus its test surface.
3. **What is ported, what is missing, and what to do next**: the Lucene surface that
   is in scope for the port, the state of each type in it, *on what evidence*, how
   much of each ported type actually exists, and the edges that let a query rank
   the remaining work by how much it unblocks.

This model follows the Label-Property Graph (LPG) paradigm used by `rmp graph`.

Two rules govern everything below, and they are the reason the graph can be
trusted as an answer rather than an impression (`CLAUDE.md` §7, §8):

* **Every claim states its basis.** A `PORTS` edge carries `evidence`; a coverage
  figure carries `memberEvidence`; a scope decision carries `portScopeRule`; a
  hand correction carries `portStateNote`. Nothing asserts a fact without saying
  how it is known.
* **The graph is checked against the code, not against its own loaders.** The
  counts in *Materialization status* are verified by re-deriving them from the
  Apache Lucene 10.5.0 clone and from the compiler's own view of the crate.

---

## Node labels

### `Project`
A software project or library being modelled.

| Property | Purpose |
|----------|---------|
| `name` | Unique project name: `"Rucene"` or `"Apache Lucene Core 10.5.0"`. |
| `language` | Primary language, e.g. `"Rust"` or `"Java"`. |
| `version` | Version string, e.g. `"10.5.0"`. |
| `path` | Root path, for the local project. |
| `gitCommit` | Hash of the local commit at which this node was last confirmed. |
| `gitDate` | ISO date of `gitCommit`. |
| `referenceUrl` | Authoritative source URL for external projects (e.g. GitHub tag URL). |

### `Module`
A crate or Maven module. 36 nodes: `rucene` (the crate) and the **35 modules of the
Apache Lucene 10.5.0 distribution that ship Java sources**.

Registering all 35, rather than `lucene/core` alone, is what makes the port's
denominator explicit instead of implied (`CLAUDE.md` §6.1). `core` is 1,213 of the
distribution's 4,035 Java files — 30.1% — and every other module carries
`inScope: false` with the reason, so the exclusion is auditable rather than silent.

| Property | Purpose |
|----------|---------|
| `name` | Module name: `"rucene"`, `"lucene/core"`, `"lucene/facet"`, `"lucene/analysis/common"`, … |
| `shortName` | The path under `lucene/`, e.g. `"core"`, `"analysis/common"`. |
| `kind` | `"crate"`, `"maven-module"`. |
| `path` | Root path in the source tree. |
| `javaFiles` | Java source files the module declares, at the surveyed tag. |
| `inScope` | `true` only for `lucene/core`. |
| `scopeRule` | `"lucene-core-is-the-port-target"` or `"outside-lucene-core"`. |
| `role` | What the module is, in one line — why it is or is not the target. |
| `gitCommit` / `gitDate` | Provenance stamp. |

`tools/kg/lucene_modules_kg.py` reproduces them from the reference clone.

### `Package`
A Java package. Identity is `name`.

| Property | Purpose |
|----------|---------|
| `name` | Fully qualified package name, e.g. `"org.apache.lucene.index"`. |
| `shortName` | Last segment, e.g. `"index"`. |
| `path` | Directory path. |
| `gitCommit` / `gitDate` | Provenance stamp. |

### `Class`, `Interface`, `Enum`, `Exception`, `Annotation`
Java type declarations from `lucene/core`. In practice **every** Java type is
registered under `Class`, with `kind` telling the declarations apart; the other four
labels are declared here for completeness but are not currently materialised.
Identity is `qualifiedName`.

| Property | Purpose |
|----------|---------|
| `name` | Simple type name. |
| `qualifiedName` | Fully qualified name, e.g. `"org.apache.lucene.index.IndexWriter"` (identity). |
| `kind` | `"class"`, `"interface"`, `"enum"`, `"record"`, `"exception"`, `"annotation"`. |
| `package` | Enclosing Java package. |
| `file` | Source file path, relative to the Lucene repository root. |
| `extendsExternal` | List of external (`java.*`, `javax.*`) super-types not modelled as nodes. |
| `implementsExternal` | List of external interfaces not modelled as nodes. |
| `portScope` | **The port scope marker.** `"in"`, `"nested"` — see below. |
| `portScopeRule` | The mechanical rule that assigned `portScope`. |
| `portState` | For a top-level type: `"ported"`, `"candidate"`, `"unported"`. For a nested one: `"ported"`, `"absorbed"`, `"unported"` — see below. |
| `portStateEvidence` | On a nested type, on what basis its `portState` was decided: `"doc-comment"`, `"curated"`, `"not-nameable"`, `"verified-absorption"`, `"verified-absent"`. Never absent. |
| `javaVisibility` / `javaPublished` / `javaUsedAcrossFiles` | On a nested type, the Java facts the classification turns on: its effective visibility, whether it and every enclosing type are `public`/`protected`, and whether another `lucene/core` file names it as `Outer.Inner`. On a **top-level** type only `javaVisibility` is set, to `"public"` or `"package"`, read from the `ACC_PUBLIC` flag of its class file in `lucene-core-10.5.0.jar` by `tools/kg/lucene_calls_kg.py`. |
| `nestedHost` / `nestedHostFiles` | On a nested type, the nearest enclosing type that has a port, and the crate file(s) that port it. |
| `portDestination` | On an `unported` nested type, the crate file its port belongs in. |
| `portStateNote` | Why the mechanical `portState` was overridden by hand. Present only on the types where the heuristic was verified wrong. **A type carrying this note is skipped by the mechanical pass**, so a hand verification survives the next sync — see below. |
| `memberTotal` / `memberMatched` / `memberCoverage` | How much of the type is actually there — see *Depth* below. |
| `memberEvidence` | Always `"name-mapping"`: the basis of the coverage figure, never omitted, because the figure is evidence and not a fact. |
| `gitCommit` / `gitDate` | Provenance stamp. |

#### `portScope` — which Lucene types the port has to cover

`portScope` makes the denominator of port coverage explicit in the graph instead of
leaving it implied, so a coverage query is defensible rather than a guess.

| Value | `portScopeRule` | Meaning |
|---|---|---|
| `in` | `lucene-core-top-level` | A **top-level** type declared by a file of the `lucene/core` module (`src/java` and `src/java21`). 1,196 types. This is the port target. |
| `nested` | `nested-in-enclosing-type` | An inner type. Excluded from the denominator, because a nested type is not an independent port unit. 908 types. **The exclusion is measured, not assumed**: every one of the 908 carries a `portState` of its own — see *The 908 nested types* below. |
| `out` | `not-a-lucene-core-type` | Anything else carrying a Java label. 5 types: Rucene's own Java test fixtures under `tests/fixtures/java-codec-harness/`. |

**The rule keys on the declaring file, not on the package name.** Those five
fixtures are declared in `org.apache.lucene.rucene.codec`, so a name-prefix rule
counted them as Lucene inner types and inflated the `nested` bucket with code
Rucene itself wrote — the kind of quiet error that makes a denominator
indefensible.

The rule is deliberately simple and mechanical so that
`tools/kg/port_coverage_kg.py` reproduces it from a clean graph, and so that no
scope decision is smuggled in: `CLAUDE.md` §1 names Apache Lucene Core 10.5.0 as
the reference source and demands functional parity plus 100% index compatibility,
and §16.1 names `lucene/core` as the canonical source tree, so the whole module is
the target. The decision is recorded as the `Decision` node *"Port scope is every
top-level type of lucene/core 10.5.0"*, with its alternatives and evidence.

#### The 908 nested types

Keeping the nested types out of the denominator is a statement about the *unit of
porting*, not a claim that they are all there. For a while the model made both at
once, and only the first was checked: the rule read "it is ported together with the
type that encloses it", nothing verified that, and 814 of the 908 carried no `PORTS`
edge at all. `CLAUDE.md` §6.1 does not allow that — an exclusion resting on an
unverified rule is exactly the implicit denominator it forbids — and the model
already knew of counter-examples, `TotalHits.Relation` and `Sorter.DocMap` among the
nested types that needed a declared port of their own.

`tools/kg/nested_port_kg.py` measures it. Every nested type carries a `portState`
and the `portStateEvidence` that produced it.

| `portState` | Count | Meaning |
|---|---|---|
| `ported` | 527 | A Rucene item is asserted to be this type, by a `PORTS` edge. |
| `absorbed` | 354 | No independent Rust item, and none is owed — on evidence, not assumption. |
| `unported` | 27 | Neither. Real work; each carries a `portDestination`. |

| `portStateEvidence` | Count | What it rests on |
|---|---|---|
| `doc-comment` | 482 | The crate's own claim, read by `port_evidence_kg.py` — the same ladder the top-level surface uses. |
| `curated` | 45 | A hand-verified edge, with the file and line that show it, carried in `tools/kg/nested_port_verdicts.json`. |
| `not-nameable` | 301 | No client can name the type, so nothing can depend on it except through the type that encloses it. See below. |
| `verified-absorption` | 53 | Nameable, but reading the crate shows where its work went and why that is faithful. Each names the host, the file and the line. |
| `verified-absent` | 27 | Read and found missing, with the reason and the destination. |

**What `not-nameable` measures, and what it does not.** A nested type is *nameable*
when it is `public` or `protected` with every enclosing type likewise — so a
downstream caller can write `Outer.Inner`; JLS 9.5 makes a member type of an
interface implicitly `public` whatever the source says — **or** when another
`lucene/core` file names it as `Outer.Inner`, which is what a package-private inner
type used by its own package looks like. 357 of the 908 are nameable; 551 are not.

For the 551 the measurement establishes something precise and something limited. It
establishes that no contract of Lucene 10.5.0 mentions the type, so no caller can
depend on it and its absence can break nothing directly. It does **not** establish
that its behaviour is present: that is measured one level up, by the enclosing
type's `memberCoverage`, and nowhere per nested type. The residual is stated here
rather than hidden, and it is not hypothetical — reading the not-nameable types whose
enclosing port measures under 50% member coverage is how ten of the 27 gaps were found,
the `ExitableDirectoryReader` wrappers among them. Every nameable type the widened
recogniser still left without a declared claim — 95 of the 357 — was read
individually; the not-nameable ones were read where that signal pointed.

**Reading the crate's own claims first.** Before any of this was curated, the
recogniser was widened to read four spellings the crate already used and
`port_evidence_kg.py` could not see — the short `Outer.Inner` form, Java modifiers
quoted *inside* the backticks, `const` and `static` items, and the canonical
`| Rucene | Apache Lucene Core 10.5.0 |` mapping table. That took the nested types
with a declared port from 94 to 482 **without admitting any new kind of inference**:
a short name resolves only when exactly one type in `lucene/core` carries that dotted
suffix, and over the module no such suffix is ambiguous and no nested type shares a
name with a member of its own enclosing type. Curation was the last resort, not the
first, which is why it accounts for 45 and not 400.

The decision, its alternatives and its evidence are the `Decision` node *"Nested
types are excluded from the denominator by measurement, not by assumption"*.

#### `portState` — what is ported and what is missing

| Value | Count at `1fa3e92` | Meaning |
|---|---|---|
| `ported` | 1,196 | Something *asserts* that a Rucene item is the port of this type: a `PORTS` edge, whose `evidence` says on what basis. |
| `candidate` | 0 | No assertion, but exactly one Rucene type carries the same simple name, recorded as a `PORTS_CANDIDATE` edge. Probable, **unverified**, and the graph's own to-do list. **Empty at `1fa3e92`**: every in-scope type now carries an asserted edge, so no coverage claim rests on a name match. The value stays in the model because the candidate rule still runs at every sync and will use it again the moment a new Lucene type appears without a declared port. |
| `unported` | 0 | Neither. |

##### The evidence ladder

The point of `evidence` on the `PORTS` edge is that a coverage number can be read
back to the thing that justifies it. Every edge carries one; none is unstated.

| `evidence` | Edges | What it rests on |
|---|---|---|
| `doc-comment` | 1,625 | The Rust item's own doc comment says `Equivalent to \`org.apache.lucene.X\`` (or `Port of`, `Ported from`, `Lucene Core equivalent:`), or the module's canonical `| Rucene | Apache Lucene Core 10.5.0 |` table names it. `CLAUDE.md` §14.1 requires that line, so this is **the port declaring what it is**, attached to one specific item. `declaredAt` gives the `file:line`. Where the item is macro-generated the same claim exists only after expansion, and the edge says so with `declarationSource: "macro-expansion"` — see `origin` under `RustStruct`. |
| `curated` | 163 | A hand-verified edge, including the cases the mechanical rules provably get wrong. |

##### What counts as a declared claim

The lead-ins are deliberately few, but the *spellings* they accept are whatever the
crate actually writes, because refusing a spelling does not make the evidence
stricter — it makes it invisible, and an invisible claim becomes an assumption
somewhere else. Five forms are read, and each is admitted only where it cannot
resolve to more than one type:

| Form | Example |
|---|---|
| the fully-qualified name | ``Equivalent to `org.apache.lucene.search.TotalHits`.`` |
| the short `Outer.Inner` name of a nested type | ``Equivalent to `FreqProxFields.FreqProxDocsEnum`.`` |
| the Java declaration quoted whole | ``Equivalent to the `final class MultiTermQuery.TopTermsScoringBooleanQueryRewrite`.`` |
| a claim on a `const` or `static`, which names a member | ``Equivalent to `SortField.Provider.NAME`.`` |
| a row of the module's mapping table | `| [`PagedBytesReader`] | `PagedBytes.Reader` |` |

`NESTED_SHORT` — two or more segments, each beginning with a capital — is what keeps
the short form honest. Lucene's packages are lower-case, so such a chain can only
name `Outer.Inner`, never `pkg.Type` and never `Type.member`; and the name is
accepted only when exactly one type in `lucene/core` carries that dotted suffix.
Measured over the module, **no** two-or-more-segment capitalised suffix is ambiguous
and **no** nested type shares a name with a member of its own enclosing type, so the
short spelling denotes one type or none, never a guess between two (`CLAUDE.md` §7).
A bare simple name is still refused everywhere, table rows included.

Two forms remain unreadable and are closed by curation rather than by loosening the
rule: a claim written on an **enum variant** (`LuceneError::MergeAborted` states its
equivalence to `MergePolicy.MergeAbortedException`, and the item reader attributes a
doc block only to a declaration it precedes), and a claim whose Lucene simple name
**differs** from the Rust item's, which the paragraph reader refuses by design since
that guard is what stops a cross-reference being read as a claim.

**Why the declared evidence changed the picture.** When the graph first measured the
port it read *381 ported, 815 candidate*: two thirds of the coverage claim rested on
a bare simple-name coincidence, which the model already warned had to be quoted as
unverified. The crate, however, states what it ports. At `1fa3e92` it makes 2,576
equivalence claims across 584 files — 1,901 attached to a single item, 629 to a whole
module, 46 in a mapping table — naming 1,953 distinct Lucene targets, and they
resolve to 1,625 `PORTS` edges carrying `evidence: "doc-comment"` over 1,605 distinct
Lucene types, 1,516 of them declared by a literal source line and 109 legible only
after macro expansion. That is not an inference the graph makes; it is a statement
the code makes, which `CLAUDE.md` §7 admits as verified knowledge where a name match
never could be.

172 of those edges map a Rust item to a Lucene type of a **different** name, over 151
distinct types — `TotalHitsRelation` → `TotalHits.Relation`, `NoOpLock` → `NoLock`,
`DefaultIndexingChain` → `IndexingChain`, `SorterDocMap` → `Sorter.DocMap`,
`AutomatonType` → `CompiledAutomaton.AUTOMATON_TYPE`. A name-match heuristic could
never have produced any of them, and would never have been right to guess them.

`absorbed` exists only on the nested half of the module, and is defined there.

##### Depth — `ported` does not mean complete

A `PORTS` edge says a Rust item claims to be a Lucene type. It says nothing about
how much of that type exists, so a stub and a finished port read identically.
`memberCoverage` measures the difference: the fraction of the Lucene type's methods
whose name maps to a function the port provides, counting the type's inherent
methods, the free functions of its file, and the methods of every trait it
implements.

At `1fa3e92`, over the 1,160 ported types that declare methods: **mean coverage
79.5%**, distributed

| Coverage | Types |
|---|---|
| 100% | 518 |
| 75–99% | 303 |
| 50–74% | 200 |
| 1–49% | 79 |
| 0% | 60 |

**This is a lower bound, and `memberEvidence: "name-mapping"` says so.** One Rust
method can discharge several Java ones; a faithful port may rename deliberately;
and Java-only members (`equals`, `hashCode`, `clone`) have no Rust counterpart by
construction. Counting only inherent methods first reported
`PackedTokenAttributeImpl` — a complete port whose every method lives in a
`CharTermAttribute` impl — at 0%, which is why implemented-trait methods are
counted. Quote the figure as a floor on completeness, never as a percentage done.

##### Where the mechanical rules fail

**The heuristic is evidence, not truth, and it fails in both directions.** Three failures were
verified by hand on 2026-08-30 and corrected in the graph:

* **False positive.** `IndexWriter` was `candidate` because `src/index/directory_reader.rs:43`
  declares a Rust trait of that name. That trait is the NRT hook `DirectoryReader.open` takes,
  not a port of the class. Corrected, with the reason in `portStateNote`.
* **False negative from ambiguity.** `MergeState` and `BufferedUpdates` each have **two** Rust
  nodes of the same simple name, so the "exactly one" rule declined to propose either.
  Curated `PORTS` edges now record the real one.
* **False negative from an empty class.** `MultiLeafReader` is declared by Lucene 10.5.0 with
  nothing but a private constructor, so the faithful port declares no Rust type. It is a
  `Component` carrying a `PORTS` edge.

**A hand verification outranks the mechanical rule, and survives.** `portStateNote`
is not a comment: `tools/kg/port_coverage_kg.py` reads it, and a type that carries
one is excluded from both the `unported` sweep and the candidate proposal. 63 types
carry one. The exclusion is deliberately keyed on the note and not on a separate
flag, so a claim can never be made without its reason.

Further failure modes, all the same root cause — the extractor matches literal
declarations, and a declaration that no literal source line contains is invisible
to it:

* **Generated by a macro.** The `internal.hppc` containers, the 24 `util.packed`
  `BulkOperationPackedN` types, the four range fields, the range doc-values fields
  and the doc-values iterators — 118 types in all — are produced by `macro_rules!`
  macros standing in for the code generators that write Lucene's own files. The
  survey records them from the compiler's own expansion; see `origin` under
  `RustStruct` below. Their **doc comment** was invisible for exactly the same
  reason, and had to be read the same way: a claim written
  `#[doc = concat!("Equivalent to `", $equiv, "`…")]` is not text until the
  compiler folds it, so the crate stated the equivalence and the graph could not
  read it. 32 of these types therefore stood at `portState = "ported"` with no
  `PORTS` edge for `measure_depth` to resolve `(name, file)` through, and were
  left out of the depth measurement altogether.
* **Declared with a lifetime parameter.** `StringSorter` is `struct StringSorter<'a>`,
  which the regex does not match.
* **No type at all.** `CollectionUtil` and `HashContainers` are modules of free
  functions, so no Rust type carries their name and no item-level claim can. They
  are `Component` nodes, reached by the module-claim rule stated under
  `Component` below.
* **Ambiguous by simple name.** `LongValues` is declared twice by Lucene — in
  `org.apache.lucene.search` and in `org.apache.lucene.util` — and twice by the port, one
  for one; `PostingDecodingUtil` and `Stats` each have a second Rust type of the same name
  in another module. The "exactly one" rule declines all of them, correctly: it will not
  guess which node a name refers to.

`candidate` exists because `CLAUDE.md` §14.1 requires Rucene to keep Lucene's names,
which makes an exact name match strong evidence — but still evidence, not a fact.
Recording it on its own edge type keeps the curated `PORTS` free of inference
(`CLAUDE.md` §7). No type carries both a `PORTS` and a `PORTS_CANDIDATE` edge:
a declared equivalence supersedes the guess, and the loader deletes it.

### `Method`
A Java method, constructor, or field member of a `lucene/core` type. Identity is
`qualifiedName`.

| Property | Purpose |
|----------|---------|
| `name` | Member name. |
| `qualifiedName` | `<class>#<signature>` (identity). |
| `signature` | Simplified signature (return type + name + parameter list). |
| `kind` | `"method"`, `"constructor"`, `"field"`. |
| `modifiers` | Access and other modifiers (`public`, `protected`, `static`, `final`, etc.). |
| `returnType` | Return/field type string (when available). |
| `parentQualifiedName` | Enclosing class qualified name. |
| `overridesExternal` | List of strings, `"<jdk type>#<name>(<param types>)"` (e.g. `"java.io.Closeable#close()"`): the JDK members this method overrides or implements, read from the bytecode by `tools/kg/lucene_calls_kg.py`. Absent when there is none. Callers that dispatch through such a JDK type are outside the jar and have no `CALLS` edge. |
| `gitCommit` / `gitDate` | Provenance stamp. |

### `File`
A source, build, documentation, or configuration file — **of either side**. Identity
is `path`. A `.rs` file of the crate is a `File` like any other; `language`
distinguishes it. (An earlier sync used a separate `RustFile` label for a single
node; that label is gone, and `File` is the only file label.)

| Property | Purpose |
|----------|---------|
| `path` | Path relative to the repository root (identity). |
| `name` | File name. |
| `kind` | `"source"`, `"test"`, `"test-fixture"`, `"build"`, `"doc"`, `"config"`, `"module-descriptor"`. `"test-fixture"` is a Java program under `tests/fixtures/` that drives real Lucene 10.5.0 to emit reference data; those 43 files carry `language: "Java"` and `crate: "java-codec-harness"`. |
| `language` | `"Rust"` for the crate's `.rs` files; absent for Lucene and non-code files. |
| `modulePath` | For a crate file, its Rust module path (`rucene::index::terms`) or, for an integration test, its test-crate name. |
| `crate` | `"rucene"`, or the `[[test]]` crate name for a test file. |
| `loc` | Line count at the surveyed commit. |
| `package` | For a Java file, its package. |
| `role` / `moduleName` | For `module-info.java`, the JPMS module name. |
| `gitCommit` / `gitDate` | Provenance stamp. |

Every `.rs` file under `src/` and `tests/` has exactly one `File` node — 644 at
`1fa3e92`, verified by `tools/kg/load_rucene_kg.py --phase audit`.

### `RustStruct`, `RustTrait`, `RustEnum`, `RustAlias`
A type declared by the Rucene crate. Identity is `{name, file}`, so two types with
the same name in different modules stay distinct. `RustAlias` is a module-level
`type X = …;` alias (for example `rucene::index::reader_manager::ReaderManager`).

| Property | Purpose |
|----------|---------|
| `name` | Rust type name, e.g. `"DocumentsWriterFlushControl"` (identity, with `file`). |
| `file` | Declaring file, relative to the repository root (identity, with `name`). |
| `qualifiedName` | `rucene::<module path>::<Name>`, or `<test crate>::…` for a test file. |
| `kind` | `"struct"`, `"trait"`, `"enum"`, `"union"`, `"alias"`. |
| `visibility` | `"pub"` or `"private"`. |
| `scope` | `"crate"` for production code, `"test"` for a type gated by `#[cfg(test)]` — on the enclosing module **or on the item itself**. |
| `origin` | `"source"` when a literal declaration exists, `"macro"` when the type only exists after macro expansion. |
| `language` | Always `"Rust"`. |
| `gitCommit` / `gitDate` | Provenance stamp. |

Types declared **inside a function body** are local to that function and are not
modelled (`HasAnyHits` in `src/document/spatial_query.rs` is one).

#### `origin` — the types no source line declares

118 type nodes carry `origin: "macro"`. They are produced by the five
`macro_rules!` macros that stand in for the code generators Lucene runs at build
time: the `internal::hppc` containers and their cursors, the 24
`BulkOperationPackedN`, the four range fields, the range doc-values fields and
their slow range queries, and the doc-values iterators.

They are recorded because **the compiler is asked**, not because a regex was made
cleverer: `tools/kg/extract_rucene_kg.py --expand` runs
`cargo +nightly rustc --lib -- -Zunpretty=expanded` and reads the expanded crate.
The same pass supplies 896 macro-generated functions.

The same dump answers a second question the literal parser cannot: **what these
types say they are.** `tools/kg/expanded_evidence_kg.py` reads the doc comment
the compiler prints above each generated item — `#[doc = "…"]` for anything a
macro built, `///` for anything written literally — and applies the same test for
a claim that a hand-written doc comment has to pass. The resulting `PORTS` edge
carries `evidence: "doc-comment"`, because that is what it rests on, and
`declarationSource: "macro-expansion"`, which is what tells the two readings
apart:

```cypher
MATCH (t)-[e:PORTS]->(c:Class) WHERE e.declarationSource = 'macro-expansion'
RETURN count(*)
```

`declaredAt` on such an edge names a file and no line, because no line declares
the item — and the file is the one the compiler places the generated item in,
which is the macro's **invocation** site, never the file that declares the
`macro_rules!`. `LongArrayList` is attributed to
`src/internal/hppc/long_array_list.rs`, not to `src/internal/hppc/macros.rs`
where `define_array_list!` is written. That choice is what keeps `(name, file)`
resolvable: `measure_depth` pairs a ported type's `file` with the `file` of the
functions that implement it, and the expander places the type and its inherent
methods in the same module.

The converse also matters: a `struct $name { … }` **inside** a macro body is a
template, not a declaration, and is no longer recorded. Reading them literally had
invented 12 types and 231 functions in `src/internal/hppc/macros.rs` — a file that
declares nothing at all outside its macros.

Earlier syncs registered Rust types under `Struct`, `Trait`, `Enum`, `Interface`,
`Class` or `Component`. The `2855d29` survey collapsed every one of them onto these
four labels, so none of those spellings survives.

### `RustFn`
A function of the Rucene crate. Identity is `qualifiedName`.

| Property | Purpose |
|----------|---------|
| `qualifiedName` | `rucene::<module path>[::<Owner>]::<name>` (identity). |
| `name` | Function name. |
| `file` | Declaring file. |
| `owner` | The type of the inherent `impl` block, or the trait, that declares it; absent for a free function. |
| `kind` | `"function"` (free), `"method"` (inherent `impl`), `"trait-method"` (declared in a `trait`), `"test"` (carries `#[test]` / `#[tokio::test]`). |
| `visibility` | `"pub"` or `"private"`. |
| `scope` | `"crate"` or `"test"`. |
| `signature` | The declaration line, whitespace-normalised. |
| `origin` | `"source"` or `"macro"`, as for the type labels above. |
| `language` | Always `"Rust"`. |
| `gitCommit` / `gitDate` | Provenance stamp. |

What is modelled: free functions, methods of inherent `impl` blocks, methods
declared in a `trait`, and tests — public or private, because a private function can
be a load-bearing port in its own right (`field_hash_flush_order`, `write_points`,
`inflate_gens`). What is **not** modelled: the bodies of `impl Trait for Type`
methods, already covered by the `IMPLEMENTS` edge plus the trait's own declaration;
and functions nested inside another function.

### `Component`
A crate unit that **declares no Rust type**: a module of free functions, the port of
one of Lucene's static utility classes — or of a Lucene class that declares no members at
all. Identity is `name`. Ten nodes, among them `IndexFileNames` (`src/index/index_file_names.rs`),
`VectorUtil` (`src/util/vector_util.rs`), `reader_util` (the inline `pub mod reader_util` in
`src/index/multi_reader.rs`), and `MultiLeafReader` (`src/index/multi_leaf_reader.rs`), added
2026-08-30 for the empty-class case described under `portState`.

These were curated one at a time until `port_evidence_kg.load` was given the rule
that states them: **a module-level `//!` claim naming `pkg.Foo`, in a file that
declares no Rust type at all, lands on that file's `Component`** — the module
being then the only thing the claim can be about. The rule was not invented for
the two it added: run over the crate it reproduces six of the eight curated
nodes — every one whose module doc states the claim at all — and finds exactly
`CollectionUtil` and `HashContainers` missing. A file that already has a
`Component` keeps it under its existing name, so the rule cannot mint a second
node for the same module.

| Property | Purpose |
|----------|---------|
| `name` | Unit name (identity). |
| `kind` | `"module"`. |
| `file` | The file that declares it. |
| `status` | `"ported"` (confirmed in the survey and carrying a `PORTS` edge), `"present"` (confirmed, no `PORTS` yet), `"stale"` (no file confirmed at the surveyed commit). |
| `language` | Always `"Rust"`. |
| `gitCommit` / `gitDate` | Provenance stamp. |

Everything that used to be a `Component` and *is* a type or a function became a
`RustStruct`/`RustTrait`/`RustEnum`/`RustAlias`/`RustFn` in the `2855d29` survey —
including `ArrayUtil`, `BitUtil`, `BytesRef`, `IOUtils`, `NumericUtils`,
`compare_utf16`, `intro_sort`, `intro_select`, `field_hash_flush_order`,
`register_doc_values_format`, `inflate_gens` and `write_points`.

### `Task`
An `rmp` task. **Every** task is mirrored — all 144 that `rmp` reported at
`a5c1556` — not only the closed ones: without the open tasks the graph cannot answer
"what to do next". `tools/kg/port_coverage_kg.py --phase tasks` refreshes them.

**The mirror only adds.** The graph holds 183 `Task` nodes: the 144 live ones plus 39
whose `rmp` task no longer exists, left behind by earlier syncs — `rmp task get 97`
answers *resource not found* while the graph still shows that node `SPRINT`. Every
`BACKLOG` and `SPRINT` node in the graph is of that kind at `a5c1556`, because `rmp`
reports none in either state. Read a `Task` node's `status` as live only for a task
`rmp` still knows; the pruning pass that would close this gap does not exist yet.

| Property | Purpose |
|----------|---------|
| `id` | The `rmp` task number, as an **integer** (identity). |
| `name` | Task title. |
| `status` | `rmp` status: `BACKLOG`, `SPRINT`, `DOING`, `TESTING`, `COMPLETED`. |
| `priority` | `rmp` priority (0–9). |
| `components` | Comma-separated Rust paths (`rucene::<module>::<Type>`) delivered by a closed task. |
| `gitCommit` | Commit that closed the task; for a task first mirrored while still open, the commit at which it was recorded. |
| `gitDate` | ISO date of `gitCommit`. |

### `Commit`
A commit of the local repository. Identity is `hash` (full 40-char).

| Property | Purpose |
|----------|---------|
| `hash` | Full commit hash (identity). |
| `message` | Commit subject line. |
| `date` | Author date, ISO 8601 with offset. |
| `task_id` | `rmp` task the commit closes, when there is one. |
| `gitCommit` / `gitDate` | Same provenance stamp carried by every other node. |

**Every commit of the repository is mirrored** by `tools/kg/commits_kg.py`, which
also writes the commit's `COMMITTED_IN`, `CLOSES_TASK` and `REFERENCES_TASK`
edges by the rules in the relationship table: `rmp` decides which commit closed a
task (`--closings`, its export of `task`/`commit_close`, each hash resolved with
`git rev-parse`), and the message decides only for a task the export does not
list, from line-start `Closes`/`Fixes` footers, the first body line's task tag, a
`Task:` trailer, or a task tag in the subject; body prose never counts.
Until this pass only 23 existed, so most `gitCommit` stamps pointed at a commit
with no node — a provenance trail that could not be followed. The two legacy nodes
that carried `commitHash` or an abbreviated hash have been merged into the
canonical ones, with their edges moved first.

### `Decision`
An engineering decision that constrains the code — a dependency choice, a
backend selection, a deliberate deviation — recorded so it can be audited and
revisited (see `CLAUDE.md` §13.2). Identity is `name`.

| Property | Purpose |
|----------|---------|
| `name` | Short decision title (identity), e.g. `"flate2 deflate backend for BEST_COMPRESSION"`. |
| `kind` | `"dependency"`, `"algorithm"`, `"format"`, `"adaptation"`, `"principle"`, `"gap"`. |
| `summary` | What was decided, in one or two sentences. |
| `rationale` | Why, in terms of the component's objective. |
| `alternatives` | Options considered and why each was rejected. |
| `evidence` | The measurement or source that settles it (see `CLAUDE.md` §8). |
| `gitCommit` / `gitDate` | Provenance stamp. |

A `Decision` reaches the code through `IMPLEMENTED_IN` (→ `File`) and records
where it landed through `COMMITTED_IN` (→ `Commit`).

`kind: "principle"` is for a project-wide rule that governs every later task, rather
than a choice confined to one component. The first is `"Fidelity first - minimise
divergences (CLAUDE.md 14.5)"`, added by `fd36286`; the second is `"Port scope is
every top-level type of lucene/core 10.5.0"`, which defines `portScope` above.

`kind: "gap"` is for a **declared gap**: a part of a task's acceptance criteria
that could not be verified end to end, recorded when the task closes so that it
stays queryable instead of being lost in a commit message. It uses the same
properties as any other decision — `summary` says what is not verified,
`rationale` why it could not be, `alternatives` what was rejected, and `evidence`
what *is* proven — and it points at the follow-up work through `DEPENDS_ON`
(→ `Task`). The first is the `IndexFileDeleter` NRT-reader and post-merge
deletion gap, added by `0dfc12d`, which depends on task #137.

### `Defect`
A bug that was found and fixed, recorded so the finding survives the fix.
Introduced by the 2026-08-29 sync for the three defects of `fd36286`; `0dfc12d`
added a fourth. Identity is `name`. A defect is worth a node when it is
non-obvious — a divergence from Lucene 10.5.0 that only a portability or fuzz
test could have caught, or a hazard whose reachability is itself the finding —
not for every routine fix.

| Property | Purpose |
|----------|---------|
| `name` | Short defect title (identity). |
| `kind` | `"portability"` (a divergence from Lucene 10.5.0) or `"robustness"` (a panic, abort or unbounded allocation reachable from a corrupt file). |
| `summary` | What was wrong, in one or two sentences. |
| `cause` | Why the code behaved that way, and why it was not caught earlier. |
| `fix` | What changed, with the `file:line` of the corrected code. |
| `luceneReference` | The Apache Lucene Core 10.5.0 file and lines that define the correct behaviour (required for `kind: "portability"`, per `CLAUDE.md` §14.5). |
| `foundBy` | The test that exposed it. |
| `gitCommit` / `gitDate` | Commit that fixed it. |

A `Defect` reaches the code through `IMPLEMENTED_IN` (→ `File`, where the fix
landed), records where it landed through `COMMITTED_IN` (→ `Commit`), and is
pinned down by the regression test that points at it with `TESTS`.

### `Feature`
A high-level functional capability, used to link packages/types to what they
implement, and to model the JPMS `module-info` descriptors.

| Property | Purpose |
|----------|---------|
| `name` | Feature name. |
| `description` | Short description. |

---

### `TestClass`

A top-level type declared by a file of the Apache Lucene Core 10.5.0 **test**
trees — `lucene/test-framework/src/java` (the machinery every Lucene test uses)
or `lucene/core/src/test` (the test corpus itself). Identity is `qualifiedName`.

Modelling the test side is what makes "which tests are still missing" a query
rather than an impression, exactly as `portScope` did for the code side. The two
trees are kept apart by `module` because they are different obligations: the
framework is infrastructure, the corpus is coverage.

| Property | Purpose |
|----------|---------|
| `qualifiedName` | Fully qualified Java name (identity). |
| `name` | Simple type name. |
| `kind` | `"class"`, `"interface"`, `"enum"`, `"record"`, `"annotation"`. |
| `package` | Enclosing Java package. |
| `file` | Source file path, relative to the Lucene repository root. |
| `module` | `"lucene/test-framework"` or `"lucene/core"`. |
| `testKind` | `"framework"` or `"unit-test"`, following `module`. |
| `role` | What the type is, derived mechanically from its name and path — see below. |
| `isAbstract` | Whether the declaration carries `abstract`. |
| `testMethodCount` | Number of test methods the file declares. |
| `ruceneCoverage` | `"covered"`, `"uncovered"`, `"subject-unported"`, `"no-subject-resolved"` — see below. |
| `gitCommit` / `gitDate` | Provenance stamp. |

#### `role` — what the type is

Assigned by name and path only, never by reading the body, so that a wrong role
is visible and correctable rather than an unauditable guess.

| Value | Count | Meaning |
|---|---|---|
| `unit-test` | 740 | `TestX`, `XTest`, `XTests` — an actual test class. |
| `framework-util` | 64 | Test-framework machinery with no more specific role. |
| `base-test-case` | 41 | `BaseXTestCase` — a reusable conformance suite that a codec or directory implementation extends to inherit hundreds of tests. |
| `mock` | 28 | Carries `Mock` in the name (`MockDirectoryWrapper`, `MockAnalyzer`). |
| `asserting` | 23 | The assertion wrappers that check codec contracts on every call. |
| `mock-filesystem` | 20 | `org.apache.lucene.tests.mockfile`. |
| `test-codec` | 17 | A codec that exists only for testing. |
| `test-case-base` | 13 | Another `…TestCase` / `…TestBase`, including `LuceneTestCase` itself. |
| `test-helper` | 13 | A helper declared inside the core test tree. |
| `fault-injection` | 12 | The `cranky` codecs, which fail on purpose. |
| `annotation` | — | A JUnit or randomizedtesting annotation. |

#### `ruceneCoverage` — the correspondence with Rucene

| Value | Count | Meaning |
|---|---|---|
| `covered` | 206 | The Lucene type this test exercises is ported, and the Rust file declaring the port also declares at least one `#[test]`. A `COVERED_BY` edge names that file. |
| `uncovered` | 238 | The type is ported but its Rust file declares no test. |
| `subject-unported` | 0 | The type this test exercises is not ported. Empty at `1fa3e92`, since every in-scope type is ported; the value is kept because a newly surveyed Lucene type can bring it back. |
| `no-subject-resolved` | 527 | The class name follows none of Lucene's test-naming conventions, so no subject could be derived. The framework types are nearly all here. |

This is a *file-level* correspondence, and it is the strongest one available: a
Rust test does not name the Lucene type it covers, but it lives in the file that
declares the port of that type. `covered` therefore means "there is Rust test
code where this Lucene test's subject lives" — **not** that the Lucene test's
cases were ported. Nothing in the graph claims the latter, and no query should
be read as if it did.

### `TestMethod`

One test method of a `TestClass` — `public void testXxx()`, or any `void` method
carrying `@Test`. Identity is `qualifiedName` (`<class>#<method>`). 5,746 nodes.

| Property | Purpose |
|----------|---------|
| `qualifiedName` | `<TestClass qualifiedName>#<method name>` (identity). |
| `name` | Method name. |
| `parentQualifiedName` | The declaring `TestClass`. |
| `file` | Source file path. |
| `module` | `"lucene/test-framework"` or `"lucene/core"`. |
| `gitCommit` / `gitDate` | Provenance stamp. |

Methods are attributed to the **first top-level type** of their file, which is
the public one. That is exact for the overwhelming majority of Lucene test files
and conservative for the rest.

---

## Edge types

| Edge | Meaning |
|------|---------|
| `CONTAINS` | `Project` → `Module`, `Module` → `Package`, `Module` → `File` (the crate's `.rs` files), `Package` → `Package`, `Package` → `Class`/`File`. |
| `DECLARES` | `File` → `Class` (Java top-level type), `Class` → `Method`, and, for the crate, `File` → `RustStruct`/`RustTrait`/`RustEnum`/`RustAlias`/`RustFn` and `RustStruct`/`RustTrait`/`RustEnum`/`RustAlias` → `RustFn` (the type declares that method). Also `TestClass` → `TestMethod`. |
| `NESTED_IN` | `Class` (inner type) → `Class` (enclosing top-level type). |
| `DEPENDS_ON` | `Package` → `Package` and `Class` → `Class` on the Lucene side, both derived from `import` declarations plus same-package references; `File` → `File` on the Rucene side, derived from `use` declarations; Rucene type → Rucene type for curated dependencies, optionally carrying a `note`. Also `Task` → `Task`, mirroring the dependency `rmp` records, and `Decision` → `Task`, which a `kind: "gap"` decision uses to name the task that will close it. |
| `EXTENDS` | `Class` → `Class` / `Class` → `Interface`; also `TestClass` → `TestClass`, which is how the 562 core tests reach `LuceneTestCase` and how a codec test reaches the `Base…TestCase` suite whose cases it inherits. 799 edges. |
| `IMPLEMENTS` | `Class` → `Interface` on the Lucene side; Rucene type → `RustTrait` (an `impl Trait for Type` block, restricted to traits the crate itself declares). Also used as `Feature` → `File`/`Class`: the file or type that realises the feature — a few early syncs (including `b0e1a75`) wrote that one the other way round, as `File` → `Feature`, and those edges are still present. |
| `PORTS` | Rucene node (`RustStruct`/`RustTrait`/`RustEnum`/`RustAlias`/`RustFn`/`Component`) → Lucene `Class`. The Rust item is the port of that Lucene type. **`evidence` is mandatory** and is `"doc-comment"` or `"curated"` — see the evidence ladder above; `declaredAt` gives the `file:line` of a declared one, or the file alone when the declaration exists only after macro expansion, in which case `declarationSource` is `"macro-expansion"`. Optional `note` records that the port is partial, a placeholder, or a deliberate adaptation. Always points at the **type**, never at the Java file. 1,788 edges at `1fa3e92`. |
| `PORTS_CANDIDATE` | Rucene type → Lucene `Class`. There is exactly one Rucene type with the same simple name, so this is very probably a port that the graph has not confirmed. `evidence` says how it was derived (`"exact-name-match"`). Promote to `PORTS` once verified. **None survives at `1fa3e92`**: the candidate phase proposed one, and the declared evidence superseded it. |
| `REQUIRES_PORT` | `Task` → `Class`. The task's statement names that Lucene type, so an unported type on the other end blocks the task. Derived mechanically from the task's title and its functional/technical/acceptance text, restricted to unambiguous simple names of at least four characters. |
| `EXPORTS` / `OPENS` / `REQUIRES` / `USES` / `PROVIDES` | `Feature` (`module-info`) → `Package` / `Feature` / `Class` (JPMS and SPI declarations). |
| `PROVIDED_BY` | `Class` (SPI interface) → `Class` (implementation). |
| `TESTS` | `File` / `Class` → `Feature` / `Class` / Rucene type / `Component` / `Defect`. A portability test file points at the harness `Feature` it belongs to and at the Rucene types whose behaviour it pins down; where it is also the regression test for a fixed bug, it points at the `Defect` too. The origin is normally a file under `tests/`; a `src/` file is the origin when the regression test is a `#[cfg(test)]` unit test in the module itself. Also `TestClass` → `Class`: the `lucene/core` type a Lucene test exercises, derived from Lucene's naming convention (`TestFoo` → `Foo`, `FooTest` → `Foo`, `BaseFooTestCase` → `Foo`) and carrying `evidence: name-convention`. It is a deduction, not a fact, and the edge says so; a name matching no convention gets no edge rather than a guessed one. 444 edges. |
| `COVERED_BY` | `TestClass` → `File`. The Rust file that declares the port of the type this Lucene test exercises, and that declares at least one `#[test]`. `evidence` is `ported-type-file-has-tests`, which is exactly what it proves — see `ruceneCoverage` above. |
| `SPECIFIED_IN` | `Feature` → `File` (specification document). |
| `REFERENCES` | `Project` → `Project` (Rucene references Apache Lucene Core 10.5.0), `Project` → `Feature` (the project specification), and `Feature` → `Package` (the Lucene packages a Rucene capability covers). It is **not** a port relation: two `RustEnum` → `Class` edges written this way by an early sync were converted to `PORTS` at `2855d29`, their claim verified against the `Equivalent to …` doc comments in `src/index/mod.rs`. |
| `REFERENCES` (crate) | Rust item (`RustStruct`/`RustEnum`/`RustTrait`/`RustAlias`/`RustFn`) or `File` → Rust item. The source's code names the target: one edge per (referrer, target) pair. Written whole by `tools/kg/rust_refs_kg.py`, which owns every edge carrying one of its two `evidence` values and replaces them on each run. `evidence: "rust-analyzer-scip"` — the name was resolved by rust-analyzer (`rust-analyzer scip`); the referrer is the innermost KG item whose span contains the occurrence (a closure or nested fn credits its enclosing fn, an `impl Trait for Type` method its self type, a module-level `const`/`static` or macro invocation the `File`); `use` declarations and self references are not references. `evidence: "macro-body-lexical"` — a `macro_rules!` body **names** the target in its text and the referrer is the `File` of an invocation; rust-analyzer reports nothing inside generated code, so this is the only evidence there is for it, and it is weaker. `prodRefs` / `testRefs` (integers) count the occurrences at production sites and at test sites (a file outside `src/`, or a span gated by `#[cfg(test)]`, marked `#[test]`/`#[tokio::test]`, in a `mod tests`/`mod test`, or in a `#[cfg(test)] impl`). Carries `gitCommit`/`gitDate`. |
| `CALLS` | Lucene `Method` or `Class` → Lucene `Method`. The source's bytecode in the released `lucene-core-10.5.0.jar` (SHA-1 checked against Maven Central) uses the target: `kind: "invoke"` for an `invoke*` instruction or the method handle a lambda or method reference binds, `kind: "field"` for `get`/`put` `field`/`static`. The target is the member javac named, resolved up the class hierarchy to its declaring class — so a virtual call reaches the static receiver's declaration, and its overriders are reached through `OVERRIDES`. The source is a `Class` when the code belongs to no modelled member (a static initialiser). A lambda body is credited to the member that wrote it, an anonymous or local class's code to its `EnclosingMethod`; bridge methods are never sources; recursion is not recorded. `sites` (integer) counts the instructions; `resolution` says how the bytecode member was matched to the `Method` node: `"exact"` (name and parameter types, a type variable matching its erasure), `"synthetic"` (a constructor once the outer-instance or enum name/ordinal parameters are dropped), `"arity"`, `"name"`, `"ambiguous"`. `evidence: "bytecode"`. Written whole by `tools/kg/lucene_calls_kg.py`. Implicit members with no `Method` node — default constructors, record accessors, enum `values`/`valueOf` — are never targets. Carries `gitCommit`/`gitDate`. |
| `OVERRIDES` | Lucene `Method` → Lucene `Method`: the source overrides or implements the target, a declaration of an ancestor type — **transitively**, so one hop reaches every declaration a virtual call can dispatch from. Generic overrides are found through their bridge methods. `evidence: "bytecode"`; written whole by `tools/kg/lucene_calls_kg.py`. Carries `gitCommit`/`gitDate`. |
| `PORTS_MEMBER` | `RustFn` → Lucene `Method`: the fn is, by name, the port of that member. `evidence: "name-mapping"` — the rule `port_evidence_kg.py`'s depth measure uses (`fooBar` → `foo_bar`; `getBar`/`isBar`/`hasBar`/`setBar` → `bar`; a constructor → `new`; an acronym kept whole, `getUTF8` → `get_utf8`). `via` says where the member was looked for: `"declared"` (a member-level `PORTS` claim on the fn names it), `"owner"` (the class the fn's declaring Rust type `PORTS`), `"inherited"` (an ancestor of that class, only when the class itself has no match), `"free-fn"` (for a free fn, the classes ported by types of its own file). A fn matching several overloads has one edge to each. Evidence, not proof. Written whole by `tools/kg/no_caller_kg.py --phase members`. Carries `gitCommit`/`gitDate`. |
| `IMPLEMENTED_IN` | `Component`/`Task`/`Decision`/`Defect` → `File`/`Commit` (where the thing lives, landed, or was fixed). |
| `COMMITTED_IN` | `File`/`Feature`/`Component`/`Decision`/`Defect` → `Commit`. `tools/kg/commits_kg.py` writes it for every file a commit changed (`git log --name-only --no-renames`), matched by `File.path`; it creates no `File` node, so a changed file without one gets no edge, and a merge commit lists no files. Carries `gitCommit`/`gitDate`. |
| `CLOSES_TASK` | `Commit` → `Task`: the commit that closed the task, as decided by `rmp`. `commits_kg.py --closings` reads the `rmp` export (`task`, `commit_close`) and resolves each hash with `git rev-parse`; an unresolvable hash is reported as `BAD_CLOSING` and its task falls back to the messages. For a task the export does not list, the commit is read from the message: a line-start `Closes`/`Fixes` footer, the task tag of the first body line, a `Task:` trailer, or a task tag in the subject; body prose never counts. A `CLOSES_TASK` an exported task holds from any other commit is retired. Carries `gitCommit`/`gitDate`. |
| `REFERENCES_TASK` | `Commit` → `Task`: the commit message carries a `Refs #N` footer, so the commit contributes to the task without closing it. Derived by `tools/kg/commits_kg.py` from footer lines only; a task the same commit closes gets `CLOSES_TASK` instead: the model allows one task edge per (`Commit`, `Task`) pair, and `commits_kg.py` deletes the other type's edge before writing. Carries `gitCommit`/`gitDate`. |
| `DELIVERS` | `Task` → `Feature`: the capability the task delivered. |
| `TESTED_BY` / `MODIFIES` / `FULFILLS` / `IMPLEMENTED_BY` | Legacy provenance edges from the first syncs; not used by new work. |

---

## Answering the standing questions

### What is ported, and what is missing

```cypher
MATCH (c:Class)
WHERE c.portScope = 'in'
RETURN c.portState AS state, count(c) AS types
ORDER BY types DESC
```

At `a5c1556` (2026-09-05, surveyed 2026-09-06), over an in-scope surface of 1,196
top-level `lucene/core` types: **`ported` 1,196, `candidate` 0, `unported` 0.**

Read together with the evidence ladder, the honest sentence is: *every one of the
1,196 in-scope types carries an assertion that it is ported — 1,120 of them in the
crate's own doc comments, the remaining 76 on a hand-curated edge — and none rests on
a bare simple-name match.* (The ladder counts **edges**, of which there are more than
there are in-scope types: an edge may also point at a nested type, and one Lucene
type may be claimed by more than one Rust item.)

**The caveat has moved, not disappeared.** While the candidate bucket was full, the
open question was whether a type was ported *at all*; that question is now closed for
the whole in-scope surface. What is left is a question of **depth**: a `PORTS` edge
says a Rust item claims to be the Lucene type, and says nothing about how much of it
exists. `memberCoverage` — a floor, see below — is where that uncertainty now lives,
and it is the figure to quote when asked how far the port has got.

The coverage of a type is not its completeness. Pair the two:

```cypher
MATCH (c:Class) WHERE c.portScope = 'in' AND c.portState = 'ported'
RETURN c.package AS package,
       count(c) AS types,
       avg(c.memberCoverage) AS meanDepth
ORDER BY meanDepth ASC
```

The thinnest ports at `a5c1556` are `util.packed` (43%), `codecs.hnsw` (44%) and
`analysis.standard` (54%); the mean over all 1,098 measured types is 78.2%.
`internal.hppc`, which read 10% at `41051f8`, now reads 62%.

#### `memberCoverage` is a floor, and a loose one

The graph's `memberCoverage` counts a Java method as present when its name maps
to a function attached to *the one Rust node* the survey linked the type to. A
port that spreads its functions over several `impl` blocks, over a supertrait,
over a `macro_rules!` expansion, or under an idiomatic rename therefore reads as
far less complete than it is. Measured on `a5c1556` it still scores `Tessellator` at
4 of 42, while the triage settles all 38 it misses — 37 renamed, 1 not applicable —
and finds nothing absent.

The floor is left as it is, deliberately: it is cheap, it is reproducible, and
it never claims more than it can see. What was missing was a layer above it that
says which of its misses are real. `memberAbsent` is that layer — see *Where the
remaining work is, by package* — and it is the property to plan from.
`memberMatched` remains the property to audit with.

`tools/kg/port_gaps.py` re-derives the gap from the two sources of truth
instead: the Java members the graph records, against **every** Rust function
declared by the files that port the type — its own, its supertypes', and the
sibling `macros.rs` that stands in for Lucene's code generator. It also knows
the shapes Rust uses for Java's contracts:

| Java | Rust |
|---|---|
| `equals`, `equalsTo` | `PartialEq::eq`, or a `#[derive(PartialEq)]` |
| `hashCode` | `Hash::hash`, or a `#[derive(Hash)]` |
| `toString` | `Display::fmt` |
| `compareTo` | `Ord::cmp` |
| `iterator` | `IntoIterator::iter` / `into_iter` |
| `getX`, `isX` | `x` |
| `getPolyLat`, `toComponent2D` | `get_polylat`, `to_component2d` |

`tools/kg/port_gaps_exclusions.json` records the members that are **deliberately
not ported**, each with its reason, so that work decided against is not counted
as work still open. Four entries at `a5c1556`: the SIMD lane kernels
of `PanamaVectorUtilSupport`, which stable Rust has no portable SIMD to express;
the JFlex-generated tables and reader protocol of `StandardTokenizerImpl`, which
this port replaces with the same UAX #29 rules applied through
`unicode-segmentation`; `AbstractAnalysisFactory.getPattern`, which needs a
regular-expression dependency the user has not yet approved; and `LatLonPoint`'s
query factories, which are blocked on the geo query types implementing the
`Query` trait.

Run it as

```bash
python3 tools/kg/port_gaps.py /tmp/gaps.json
```

which prints the total and the worst types, and writes the per-type detail —
including the exact Java method names it believes absent — as JSON. It reads the Java
members from the graph but the Rust functions from the **working tree**, whose root
it hard-codes, so its output is attributable to a commit only when the tree is clean
at that commit.

**Its total does not agree with the triage, and the disagreement is the point.**
Measured on `a5c1556` it reports 85 missing methods where the triage — the same
question, decided per member against both sources — reports 89. The totals are now
close, but the two sets barely overlap: they share only 21 members. `port_gaps.py`
calls 68 of the triage's 89 present, and reports 64 that the triage settles. The
cause is that it searches a whole porting file with no check on which type owns the
function it finds, and applies the `toString`→`fmt`, `equals`→`eq` mapping
unconditionally: `src/geo/geometry.rs` contains exactly one `impl Display`, and every
geo type's `toString` is credited to it. Two totals that nearly match while naming
different members is the worst kind of agreement — do not read the closeness as
corroboration. Use it to list candidate names for one type; do not quote its total.
`memberAbsent` is the number, and *Where the remaining work is, by package* is where
it is explained.

**There is no unverified share left to locate.** The `portState` query at the head of
this section returns `ported` for all 1,196 in-scope types at `a5c1556`, so the
per-package breakdown of `candidate` this section used to carry is empty in every
row and has been dropped. The two per-package questions still worth asking are the
depth one above and the member one below.

**A port may carry a `note` on its `PORTS` edge recording a declared divergence,
a partial port or a deliberate adaptation** — read them before assuming a type is
complete:

```cypher
MATCH (t)-[e:PORTS]->(c:Class) WHERE e.note IS NOT NULL
RETURN c.qualifiedName AS lucene, e.note AS divergence
```

And to see what a port claim actually rests on:

```cypher
MATCH (t)-[e:PORTS]->(c:Class {qualifiedName: $type})
RETURN t.name, t.file, e.evidence, e.declaredAt, e.note
```

### What is in scope at all

```cypher
MATCH (m:Module) WHERE m.project = 'Apache Lucene Core 10.5.0'
RETURN m.inScope AS inScope, count(m) AS modules, sum(m.javaFiles) AS javaFiles
```

35 modules, 4,035 Java files; `lucene/core` alone — 1,213 files, 30.1% — is the
port target. Every excluded module carries `scopeRule` and `role`.

### Where the remaining work is, by package

Type-level coverage no longer discriminates at all: at `a5c1556` every in-scope type
is `ported`, so the answer is "all of them", and the question that decides the work
becomes *how much of each ported type is actually there*. `memberMatched`
answers that as a **floor** — it counts as missing every method the port renamed
— so the figure it leaves behind is a ceiling on the work, not a list of it.
Task #161 first classified those names against the Java and the Rust source, and the
triage is re-derived at every sync; `memberAbsent` is what survives. **That is the
number to plan from:**

```cypher
MATCH (c:Class)
WHERE c.portScope = 'in' AND c.memberTriage IS NOT NULL
WITH split(c.package, '.') AS p, c
WITH CASE WHEN size(p) >= 4
          THEN p[0]+'.'+p[1]+'.'+p[2]+'.'+p[3]
          ELSE c.package END AS pkg, c
RETURN pkg,
       count(c)                                   AS types,
       sum(c.memberTotal)                         AS members,
       sum(c.memberTotal) - sum(c.memberMatched)  AS unmatched,
       sum(c.memberAbsent)                        AS absent,
       sum(c.memberRenamed)                       AS renamed,
       sum(c.memberNa)                            AS notApplicable
ORDER BY absent DESC
```

At `a5c1556` it returns 8,569 method names over 1,098 measured types, 1,587
unmatched by the floor — of which **89 are real absences**, 1,354 are present
under another name and 144 do not apply:

| package | types | members | unmatched | **absent** | renamed | n/a |
|---|---:|---:|---:|---:|---:|---:|
| `org.apache.lucene.search` | 264 | 1,608 | 394 | **33** | 330 | 31 |
| `org.apache.lucene.index` | 186 | 1,814 | 256 | **18** | 224 | 14 |
| `org.apache.lucene.util` | 249 | 1,758 | 269 | **18** | 229 | 22 |
| `org.apache.lucene.internal` | 45 | 691 | 127 | **12** | 91 | 24 |
| `org.apache.lucene.analysis` | 52 | 336 | 74 | **3** | 49 | 22 |
| `org.apache.lucene.codecs` | 120 | 879 | 132 | **2** | 119 | 11 |
| `org.apache.lucene.document` | 104 | 765 | 198 | **2** | 177 | 19 |
| `org.apache.lucene.store` | 51 | 436 | 43 | **1** | 42 | 0 |
| `org.apache.lucene.geo` | 27 | 282 | 94 | **0** | 93 | 1 |

The ordering is still not the one the floor gives: `search` carries the largest
unmatched count (394) and `geo` the largest share of one (94 of 282), and neither is
where the work is. It is also no longer the ordering of the previous sync. At
`edfea04` the total stood at 370, with `index` (91) and `document` (75) the two big
holes and `search` at 44. Sprint 6 closed those two packages: at `a5c1556` `document`
is down to 2 and `index` to 18, `search` (33) is the largest remaining, and the total
has fallen to 89. `geo` now has no absence at all. Re-read the table after every sync rather than
carrying the last ordering forward — it has now been overturned twice.

**What `absent` is and is not.** It is: a Java method name on an in-scope type
for which no rule could find a Rust item that discharges it, checked against the
crate's source rather than against the graph's node set. It is **not** a count of
methods to write one for one — 1,029 of the 1,098 measured types have none at all,
69 types carry all 89 between them, 54 of those carry exactly one, and none carries
more than three. Nor is it uniform work: 43 of the 89 are Java's `Object`/`Iterable`
contract on a type that has neither the trait impl nor the `#[derive]` — `toString`
alone is 21 — which is one mechanical pattern, while the other 46 are domain methods
that each need reading. Rank types, not packages:

```cypher
MATCH (c:Class) WHERE c.portScope = 'in' AND c.memberAbsent > 0
RETURN c.qualifiedName AS type, c.memberTotal AS members,
       c.memberAbsent AS absent, c.memberRenamed AS renamed
ORDER BY absent DESC, type LIMIT 25
```

and read the individual verdicts, each of which names the rule that produced it:

```cypher
MATCH (c:Class {qualifiedName:'org.apache.lucene.index.TieredMergePolicy'})
      -[:DECLARES]->(m:Method)
WHERE m.portTriage IS NOT NULL
RETURN m.name, m.portTriage, m.portTriageRule, m.portTriageEvidence
```

#### How the triage decides, and what it is worth

`tools/kg/triage_members.py` reproduces `measure_depth` exactly — it asserts its
own per-type counts against the `memberMatched` the graph holds and fails if any
differ — then applies rules to the pairs the floor left over, strongest first,
recording which rule fired. The rules are tiered by how tightly the Rust item is
bound to the type:

| tier | evidence | share of `renamed` |
|---|---|---:|
| `own` | the Rust type that declares it ports this Lucene type, a trait it implements, or that trait's supertraits | 848 |
| `super` | the same, for the port of a Java supertype — Java re-declares inherited methods as overrides, the port declares them once | 182 |
| `file` | elsewhere in the files that port the type, its `X/` submodule, or the sibling `macros.rs` that stands in for Lucene's code generator | 324 |

Within a tier the rules run `exact` → `accessor-prefix` → `separator` →
`doc-comment` → `name-extension`. Three of them are worth stating because they
are what a name comparison alone cannot see:

* **`doc-comment`** reads the crate's own `Equivalent to \`Type.member(...)\``
  claim (`CLAUDE.md` §14.1) and requires the type it names to be this type or one
  of its Java supertypes. It is the port asserting the correspondence itself, so
  it outranks every heuristic — and the type check is what stops one type's
  evidence being credited to another.
* **`name-extension`** accepts a longer Rust name (`decode` →
  `decode_long_blocks_to_longs`) only where Java overloads the method or the port
  declares two or more functions extending it. Java overloads on argument type
  and Rust cannot, so a split is expected there and nowhere else; ungated, the
  rule let `getCalibrateSizeByDeletes` be answered by its own setter.
* **`contract-trait` / `derive`** answer `equals`, `hashCode`, `toString`,
  `clone`, `compareTo` and `iterator` from the Rust trait that carries them, and
  only from the type's **own** impls. Rust has no inheritance: `impl Display for
  FilterDirectory` says nothing about `NRTCachingDirectory`, which overrides
  `toString` in Java and has no `Display` here.

`na` has exactly three grounds, 144 in all at `a5c1556`: a `#[derive]` that
discharges a Java `Object` contract (34); a `private` Java method on a type whose
reachable surface carries no absence, so the difference is one of decomposition and
not of function (67); and a member `port_gaps_exclusions.json` marks
`"triageClass": "na"` because the Java construct that creates it does not exist
here — the Panama lane kernels and the JFlex scanner protocol (43). The other two
of that file's four entries are marked `"absent"`: work deferred is still work.

Anything no rule settles is `absent`. That direction is deliberate — a wrong
`absent` costs a check, a wrong `renamed` hides work — and it is why the count
should be read as an upper bound on real absence and a lower bound on what is
done.

**Verification.** At `edfea04`, 60 classifications were checked by hand against
`/tmp/lucene1050` and the crate, spread over all nine packages, all three
classes and every rule. All 60 verdicts hold. Getting there cost eight rule
defects, each found by a sample item and each now covered by a comment at the
rule: a whole supertype file admitted as evidence (`BytesRefHash.grow` answering
`BinaryDocValuesFieldUpdates.grow`); a trait resolved to a homonym
(`src/index/documents_writer.rs` also declares `Query`, and taking the first
match hid every query type's real members); a macro fallback that fired for any
type with no `impl` under its own name, which is true of every trait; a
supertype's `#[derive]` read as the subtype's; a supertype's trait *declaration*
read as proof the subtype implements it; one declaration site per name per file,
which hid `StandardTokenizer`'s `set_max_token_length` behind
`StandardTokenizerImpl`'s; a one-word name matched anywhere in a shared module
(`InputStreamDataInput.close` answering to `RandomAccessInputAdapter::close`);
and a module scope that stopped at `X.rs` and never looked in `X/`, which is the
same Rust module (`PackedInts.getDecoder` lives in
`src/util/packed/bulk_operation.rs`).

Re-derive it after any change to the crate:

```bash
python3 tools/kg/triage_dump.py rucene /tmp/triage          # snapshot the graph
python3 tools/kg/triage_repro.py /tmp/triage /tmp/triage/unmatched.json
python3 tools/kg/triage_members.py /tmp/triage /tmp/triage/triaged.json --root .
python3 tools/kg/triage_load.py rucene /tmp/triage/triaged.json \
    --commit "$(git rev-parse HEAD)" --date "$(git show -s --format=%cs HEAD)"
```

`triage_repro.py` exits non-zero if it cannot reproduce the graph's
`memberMatched` type for type, which is the guard that keeps the triage attached
to the floor it is classifying. Re-run at `a5c1556` it reproduces the graph exactly —
1,098 types, 8,569 members, 6,982 matched, 1,587 unmatched, with no per-type
disagreement — and the classification that follows yields the 89 / 1,354 / 144 the
table above reports.

**Do not sum the `Method`-level verdicts.** `portTriage` is written but never
cleared: `triage_load.py` sets the members the current run classified and leaves
every earlier verdict standing, so a member the port has since grown a function for
keeps the `absent` it was given at an older commit. It is also set on every `Method`
node of the class whose `name` matches, so a Java overload set of *n* members
receives *n* copies of one verdict. Summed, the `Method` verdicts therefore exceed
the classification — 1,696 renamed, 160 `na`, 90 absent at `a5c1556`, against the
rollup's 1,354 / 144 / 89 — and carry a fourth value, `ported` (136), which the
hand-verification tasks #167–#180 wrote and no rule of `triage_members.py` produces.
The per-`Class` rollup is rewritten in full at every load and is the only place the
current classification can be summed. Sum the rollup; read the `Method` verdicts one
at a time, for their rule and evidence.


### What to do next

**Both of the next two queries return nothing at `a5c1556`**, because no in-scope
type is `unported` or `candidate` any more. They are kept because they are the right
questions the moment a newly surveyed Lucene type arrives without a declared port,
and because an empty result is itself the answer to "is anything unported?". Rank the
unverified surface by how much it blocks — tasks plus other in-scope types that
depend on it:

```cypher
MATCH (u:Class)
WHERE u.portScope = 'in' AND u.portState <> 'ported'
OPTIONAL MATCH (t:Task)-[:REQUIRES_PORT]->(u)
OPTIONAL MATCH (d:Class)-[:DEPENDS_ON]->(u)
RETURN u.qualifiedName AS unported,
       count(DISTINCT t) AS blockedTasks,
       count(DISTINCT d) AS blockedTypes,
       count(DISTINCT t) + count(DISTINCT d) AS blockedTotal
ORDER BY blockedTotal DESC
LIMIT 15
```

The task-anchored form is much faster and answers "which open task is blocked by
what":

```cypher
MATCH (t:Task)-[:REQUIRES_PORT]->(u:Class)
WHERE t.status <> 'COMPLETED' AND u.portState <> 'ported'
RETURN u.qualifiedName AS unported, u.portState AS state,
       count(DISTINCT t) AS blockedTasks, collect(DISTINCT t.id) AS taskIds
ORDER BY blockedTasks DESC
```

With both buckets empty, the useful form of "what to do next" is the shallowest port
that the most work depends on — the depth question that *Answering the standing
questions* says is where the remaining uncertainty lives:

```cypher
MATCH (u:Class)
WHERE u.portScope = 'in' AND u.memberCoverage < 0.5
OPTIONAL MATCH (d:Class)-[:DEPENDS_ON]->(u)
RETURN u.qualifiedName AS thin, u.memberCoverage AS depth,
       count(DISTINCT d) AS dependants
ORDER BY dependants DESC LIMIT 15
```

### Where a Rucene item lives, and what it depends on

```cypher
MATCH (f:File)-[:DECLARES]->(t)
WHERE t.name = 'PointValuesWriter'
RETURN f.path, labels(t)[0], t.qualifiedName
```

```cypher
MATCH (a:File {path:'src/index/point_values_writer.rs'})-[:DEPENDS_ON]->(b:File)
RETURN b.path
```

### Who reaches a ported item — `pub` items no production code references

The question: which `pub` items of the crate does no production code reference,
what is each one's Lucene counterpart, and who calls that counterpart in Lucene
10.5.0? It joins four edge sets, each written whole by its tool and stamped with
the commit it was derived at (`tools/kg/README.md`, *No-production-reference
items*): `REFERENCES` (crate) from rust-analyzer, `CALLS` and `OVERRIDES` from the
10.5.0 bytecode, and `PORTS_MEMBER` from the name mapping.

* **`pub` item** — a `RustFn` or Rust type node with `visibility = "pub"`,
  `scope = "crate"`, `file` under `src/`, and not a test. `visibility` is what the
  survey reads off the declaration, so `pub(crate)` counts as `pub`, and a
  trait's methods — which carry no `pub` keyword — are `private` and out of the
  list.
* **No production reference** — no incoming `REFERENCES` with
  `evidence = "rust-analyzer-scip"` and `prodRefs > 0`; for a type, a reference
  from a fn the type itself `DECLARES` does not count. A `macro-body-lexical`
  edge is reported (`macroBodyMentions`) and never counted, because the compiler
  did not resolve it.
* **Counterpart** — for a fn, the `Method`s it `PORTS_MEMBER`, plus the `Class` it
  `PORTS` directly; for a type, the `Class` it `PORTS`.
* **Lucene callers** — of a member, every source of a `CALLS` edge into it or into
  a declaration it `OVERRIDES` (the dispatch a virtual call takes); of a type,
  every other `Class` that `DEPENDS_ON` it.
* **Triage class** — `W?`: the counterpart has Lucene callers, so the port lacks
  a link Lucene has. `P?`: no Lucene caller, and the counterpart is public API (a
  public or protected member — or a non-private interface member — of a type that
  is a public top-level type or a nested type with `javaPublished = true`; for a
  type, the type itself is). `D?`: no counterpart, or a counterpart that is
  neither called nor API — the item may not need to exist. `reason` refines it.
  A `W?` is *blocked* when none of its callers' classes has `portState` `ported`
  or `absorbed`, which `no_caller_kg.py --phase report` computes from the
  `callerClasses` column. The classes are a triage aid, not a verdict: the name
  mapping is evidence, and dispatch through a trait on the Rust side, or through
  a JDK type on the Lucene side, hides real references.

Functions:

```cypher
MATCH (x:RustFn)
WHERE x.visibility = 'pub' AND x.scope = 'crate' AND x.kind <> 'test'
  AND x.file STARTS WITH 'src/'
OPTIONAL MATCH (x)<-[e:REFERENCES]-() WHERE e.evidence = 'rust-analyzer-scip' AND e.prodRefs > 0
WITH x, count(e) AS prodReferrers
WHERE prodReferrers = 0
OPTIONAL MATCH (x)<-[lx:REFERENCES]-() WHERE lx.evidence = 'macro-body-lexical' AND lx.prodRefs > 0
WITH x, count(lx) AS macroBodyMentions
OPTIONAL MATCH (x)-[:PORTS_MEMBER]->(m:Method)<-[:DECLARES]-(mc:Class)
OPTIONAL MATCH (m)<-[:CALLS]-(c1)
OPTIONAL MATCH (m)-[:OVERRIDES]->(:Method)<-[:CALLS]-(c2)
WITH x, macroBodyMentions, m, mc, collect(DISTINCT c1) + collect(DISTINCT c2) AS cs
OPTIONAL MATCH (m)-[ov:OVERRIDES]->(:Method)
WITH x, macroBodyMentions, m, mc, cs, count(ov) AS overrides
WITH x, macroBodyMentions, collect(CASE WHEN m IS NULL THEN null ELSE {
       member: m.qualifiedName,
       api: (m.modifiers CONTAINS 'public' OR m.modifiers CONTAINS 'protected'
             OR (mc.kind = 'interface' AND NOT m.modifiers CONTAINS 'private'))
            AND (mc.javaPublished = true
                 OR (mc.javaPublished IS NULL AND mc.javaVisibility = 'public')),
       inherited: overrides > 0 OR m.overridesExternal IS NOT NULL,
       callers: [c IN cs | c.qualifiedName],
       callerClasses: [c IN cs | coalesce(c.parentQualifiedName, c.qualifiedName)]
     } END) AS cps
OPTIONAL MATCH (x)-[:PORTS]->(pc:Class)
WITH x, macroBodyMentions, cps, collect(pc.qualifiedName) AS portsClass
WITH x, macroBodyMentions, cps, portsClass,
     reduce(a = [], cp IN cps | a + cp.callers) AS callers,
     reduce(a = [], cp IN cps | a + cp.callerClasses) AS callerClasses
RETURN x.qualifiedName AS item, 'RustFn' AS label, x.file AS file,
       split(x.qualifiedName, '::')[1] AS subsystem,
       [cp IN cps | cp.member] + portsClass AS counterparts,
       callers, callerClasses, macroBodyMentions,
       CASE
         WHEN size(cps) = 0 AND size(portsClass) = 0 THEN 'D?'
         WHEN size(callers) > 0 THEN 'W?'
         WHEN any(cp IN cps WHERE cp.api) OR size(portsClass) > 0 THEN 'P?'
         ELSE 'D?'
       END AS class,
       CASE
         WHEN size(cps) = 0 AND size(portsClass) = 0 THEN 'no-counterpart'
         WHEN size(callers) > 0 THEN 'lucene-calls-counterpart'
         WHEN any(cp IN cps WHERE cp.api AND cp.inherited) THEN 'inherited-api'
         WHEN any(cp IN cps WHERE cp.api) OR size(portsClass) > 0 THEN 'public-api'
         ELSE 'counterpart-not-api-and-uncalled'
       END AS reason
```

Types — the same shape over the `PORTS` class and its `DEPENDS_ON` dependants;
run it once per label, with `{label}` replaced by `RustStruct`, `RustEnum`,
`RustTrait` and `RustAlias`:

```cypher
MATCH (x:{label})
WHERE x.visibility = 'pub' AND x.scope = 'crate' AND x.file STARTS WITH 'src/'
OPTIONAL MATCH (x)<-[e:REFERENCES]-(r)
  WHERE e.evidence = 'rust-analyzer-scip' AND e.prodRefs > 0 AND NOT (x)-[:DECLARES]->(r)
WITH x, count(e) AS prodReferrers
WHERE prodReferrers = 0
OPTIONAL MATCH (x)<-[lx:REFERENCES]-() WHERE lx.evidence = 'macro-body-lexical' AND lx.prodRefs > 0
WITH x, count(lx) AS macroBodyMentions
OPTIONAL MATCH (x)-[:PORTS]->(pc:Class)
OPTIONAL MATCH (pc)<-[:DEPENDS_ON]-(dc:Class) WHERE dc <> pc
WITH x, macroBodyMentions, pc, collect(DISTINCT dc.qualifiedName) AS deps
WITH x, macroBodyMentions, collect(CASE WHEN pc IS NULL THEN null ELSE {
       cls: pc.qualifiedName,
       api: pc.javaPublished = true
            OR (pc.javaPublished IS NULL AND pc.javaVisibility = 'public'),
       callers: deps } END) AS cps
WITH x, macroBodyMentions, cps, reduce(a = [], cp IN cps | a + cp.callers) AS callers
RETURN x.qualifiedName AS item, '{label}' AS label, x.file AS file,
       split(x.qualifiedName, '::')[1] AS subsystem,
       [cp IN cps | cp.cls] AS counterparts,
       callers, callers AS callerClasses, macroBodyMentions,
       CASE
         WHEN size(cps) = 0 THEN 'D?'
         WHEN size(callers) > 0 THEN 'W?'
         WHEN any(cp IN cps WHERE cp.api) THEN 'P?'
         ELSE 'D?'
       END AS class,
       CASE
         WHEN size(cps) = 0 THEN 'no-counterpart'
         WHEN size(callers) > 0 THEN 'lucene-depends-on-counterpart'
         WHEN any(cp IN cps WHERE cp.api) THEN 'public-api'
         ELSE 'counterpart-not-api-and-unused'
       END AS reason
```

By subsystem (the crate's top-level module) and class: replace the final
`RETURN` of either query by `WITH` and append

```cypher
WITH subsystem, class, count(*) AS items
RETURN subsystem, class, items ORDER BY subsystem, class
```

`no_caller_kg.py --phase report` runs exactly these statements, checks the
grouped variant against the list, and writes every row to
`<kg-target>/no_caller/list_<commit>.jsonl`.

---

## Query pitfalls

**`NOT EXISTS { MATCH … WHERE … }` returns the wrong answer on this engine.** Asking which
in-scope unported types have no open task with

```cypher
WHERE NOT EXISTS { MATCH (t:Task)-[:REQUIRES_PORT]->(u) WHERE t.status <> 'COMPLETED' }
```

returned **all 56** types of `org.apache.lucene.index`, including ones a direct `MATCH` proves
are required by an open task. Verified 2026-08-30, when the in-scope surface still held
unported types; at `a5c1556` both forms return an empty result, but the engine behaviour
they expose is unchanged. Use `OPTIONAL MATCH` and count instead:

```cypher
MATCH (u:Class) WHERE u.portScope='in' AND u.portState='unported'
OPTIONAL MATCH (t:Task)-[:REQUIRES_PORT]->(u) WHERE t.status <> 'COMPLETED'
RETURN u.name AS type, count(DISTINCT t) AS openTasks
```

That form gave 50 covered and 6 orphaned on 2026-08-30, which matched the task texts.

**An alias that collides with a pattern variable silently returns the node id.**
`MATCH (t)-[:PORTS]->(c) RETURN c.memberTotal AS t` returns `t`'s node id, not the
member count, because the alias rebinds the pattern variable. The query neither
fails nor warns. Name result aliases so they cannot collide with anything bound in
the pattern. Verified 2026-09-05.

**`REQUIRES_PORT` misses ambiguous simple names.** The edge is derived from unambiguous simple
names of at least four characters, so a type whose name collides across packages — `Sorter`,
for instance, which blocks 27 in-scope types — carries no edge even when a task's text names
it. An apparent orphan must be checked against the task texts before a task is created for it.

---

## Provenance convention

Every node and edge carries `gitCommit` (full 40-char hash) and `gitDate`
(`YYYY-MM-DD`) stamping when the fact was last confirmed. For nodes describing the
external Apache Lucene source, the provenance records the **local Rucene commit** at
the time of discovery/registration.

**A pair may hold edges of several types.** On rmp 1.19.0, `MERGE (a)-[:X]->(b)`
over a pair already joined by an edge of another type creates the `X` edge, and
a `MERGE` of a type the pair already holds creates nothing. Measured on
2026-10-30 against throwaway `ZzProbe` nodes in the `rucene` graph, deleted
afterwards (`nodesDeleted: 2, relationshipsDeleted: 3`):

```
CREATE (a:ZzProbe {k:1}), (b:ZzProbe {k:2}), (a)-[:R1]->(b)
  -> counters {nodesCreated: 2, relationshipsCreated: 1, propertiesWritten: 2, labelsAdded: 2}
MATCH (a:ZzProbe {k:1}) MATCH (b:ZzProbe {k:2}) MERGE (a)-[:R2]->(b)
  -> counters {relationshipsCreated: 1}
MATCH (a:ZzProbe {k:1}) MATCH (b:ZzProbe {k:2}) MERGE (a)-[:R1]->(b)
  -> {"ok": true}, no counters
MATCH (a:ZzProbe {k:1}) MATCH (b:ZzProbe {k:2}) MERGE (a)-[:R3]->(b)
  -> counters {relationshipsCreated: 1}
MATCH (a:ZzProbe {k:1})-[e]->(b:ZzProbe {k:2}) RETURN type(e)
  -> R1, R2, R3
```

Earlier engines behaved otherwise: there, such a `MERGE` matched the other type's
edge and created nothing, which is what hid the `DocValuesType` and
`IndexOptions` ports behind their `REFERENCES` edges until `2855d29`. Where a pair
is joined by more than one edge, set or delete an edge's properties only after
re-binding it with `MATCH (a)-[e:X]->(b)`, never through a matched `MERGE`.
Undirected patterns report the forward edge's type for a reverse traversal, so
every read must be written outgoing.

**Edges written by the first (2026-07-30) Lucene survey were never stamped, and
most still are not.** `rmp` task #136 tracks repairing them. Each sync stamps what it
actually re-derived from the reference clone — `NESTED_IN` (908, all of them) and the
Lucene member `DECLARES` — and deliberately leaves the rest alone: a stamp means
*this fact was last confirmed at this commit*, so stamping an edge a sync did not
re-verify would be a false claim, which is exactly what the convention exists to
prevent.

Unstamped at `a5c1556`, by edge type — 10,787 edges in all:

| Edge | Unstamped | Why it is still open |
|---|---:|---|
| `DECLARES` | 6,942 | 5,746 `TestClass → TestMethod` from the test survey, 1,196 Java `File → Class` not re-derived |
| `EXTENDS` | 1,395 | not re-derived by this sync |
| `CONTAINS` | 1,309 | ditto |
| `TESTS` | 444 | written by the earlier test survey |
| `DEPENDS_ON` | 238 | 231 Lucene `Package → Package`, 6 curated crate `File → File`, 1 `Decision → Task` |
| `COVERED_BY` | 208 | written by the earlier test survey |
| `IMPLEMENTS` (Lucene) | 117 | not re-derived by this sync |
| JPMS/SPI (`EXPORTS`, `USES`, …) | 63 | ditto |
| `COMMITTED_IN` | 32 | written before the convention existed |
| the remainder | 39 | `REFERENCES` 11, `CLOSES_TASK` 8, `SPECIFIED_IN` 8, `IMPLEMENTED_IN` 3, `DELIVERS` 1, and the 8 legacy `IMPLEMENTED_BY`/`TESTED_BY`/`MODIFIES`/`FULFILLS` edges |

Every edge the `1fa3e92` sync wrote — the 1,788 `PORTS` (no `PORTS_CANDIDATE`
survives it), the 908 `NESTED_IN`, the crate's
`DECLARES`/`IMPLEMENTS`/`DEPENDS_ON`, the `Task` and `Commit` mirrors — carries
`gitCommit` and `gitDate`. **Nodes** are fully stamped: the audit's *nodes without a
provenance stamp* check reads 0.

---

## Reproducing the graph

The loaders under `tools/kg/` rebuild the whole graph from a clean store; see
`tools/kg/README.md` for the exact order and arguments. In summary:

0. `lucene_modules_kg.py` — the module inventory that states the port's denominator.
1. `extract_lucene_kg.py` + `run_kg_batches.py` — packages, files, top-level types,
   `DEPENDS_ON`, `EXTENDS`, `IMPLEMENTS` for `lucene/core`.
2. `enrich_lucene_kg.py` (generation) + `lucene_structure_kg.py` — inner types and
   members, loaded in `UNWIND` batches.
3. `extract_rucene_kg.py --expand` → `load_rucene_kg.py` — the crate: files, types,
   functions, `DECLARES`, `IMPLEMENTS`, `DEPENDS_ON`, plus the hygiene passes that
   collapse legacy labels, merge duplicate nodes, **prune nodes the survey no longer
   confirms**, and an `--phase audit` that compares the graph against the survey.
4. `port_coverage_kg.py` — `portScope`, `portState`, Lucene type→type `DEPENDS_ON`,
   `PORTS_CANDIDATE`, the `Task` mirror with `REQUIRES_PORT`, `Component.status`,
   and the scope `Decision`.
5. `port_evidence_kg.py --phase extract` then `--phase load` — the `PORTS` edges
   the crate's own doc comments declare.
   `expanded_evidence_kg.py` — the same for the types that exist only after macro
   expansion, whose doc comment no source line contains.
   `port_evidence_kg.py --phase depth` — the depth measurement, which must run
   after both, since it resolves a ported type's functions through the
   `(name, file)` of a `PORTS` edge.
   Re-run step 4's `scope` and `candidates` phases afterwards so `portState`
   reflects them — as two separate invocations, since `--phase` takes a single
   value and rejects a comma-separated list.
5b. `nested_port_kg.py` — the `portState` of each of the 908 nested types, the
   curated `PORTS` edges its verdict file carries, and the `Decision` that records
   why the exclusion is now a measurement. It must run **after** step 5, since a
   nested type is `ported` on the strength of the edges step 5 writes.
6. `commits_kg.py --closings <rmp export>` — the full commit history, `COMMITTED_IN`, `CLOSES_TASK` and `REFERENCES_TASK`.
7. `lucene_tests_kg.py` — the Lucene test surface and its correspondence with the crate.
8. `rust_refs_kg.py` (rust-analyzer SCIP, crate `REFERENCES`), `lucene_calls_kg.py`
   (the 10.5.0 bytecode, `CALLS` / `OVERRIDES` / `Method.overridesExternal` / a
   top-level `Class.javaVisibility`), then `no_caller_kg.py` (`PORTS_MEMBER` and the
   no-production-reference report). `no_caller_kg.py` must run after steps 3, 5
   and both of the others, since it reads `DECLARES`, `PORTS` and the edges they
   write.

Steps 3 and 5 both need a **clean** tree: never survey a dirty working directory,
or another task's half-finished work enters the graph as fact. Use
`git worktree add --detach`.

### Defects this pipeline had, and what they cost

Recorded because each one made the graph state something false about the code, and
each is now covered by a check that would catch it again.

| Defect | Consequence | Fix |
|---|---|---|
| The Java declaration regex admitted no `record` component list and no `permits` clause | 54 nested Lucene types missing, `BooleanClause.Occur` and `TotalHits.Relation` among them | `enrich_lucene_kg.py`, all three declaration regexes |
| The crate extractor could not see macro-generated types | 67 real types absent; the audit proposed deleting four that carried `PORTS` edges | `--expand`, via the compiler's own expansion |
| The crate extractor read `macro_rules!` bodies as declarations | 12 types and 231 functions invented in `macros.rs` | `blank_macro_bodies` |
| The claim recogniser required the `org.apache.lucene.` prefix, refused Java modifiers quoted inside the backticks, ignored `const`/`static` items and did not read the module mapping table | 388 nested types the crate *declares* it ports were invisible, and their absence was then read as evidence that the `nested-in-enclosing-type` rule held | `NESTED_SHORT` plus the unique-dotted-suffix resolution, the quoted-modifier span, `const`/`static` in `RE_ITEM`, and `_table_claims` |
| The nested exclusion asserted that every inner type is ported with its enclosing type, and nothing checked it | 814 of the 908 carried no `PORTS` edge; the certified 1,196/1,196 answered a narrower question than the one it appeared to answer, and 27 real gaps were invisible | `nested_port_kg.py`, plus two audit checks that fail on an unclassified nested type |
| Nothing could read a macro-generated type's **doc comment** | 32 in-scope types `ported` with no `PORTS` edge to anchor, so `measure_depth` never measured them and their Lucene members stayed outside the depth denominator | `expanded_evidence_kg.py`, reading the doc from the same expansion |
| A module claim on a file that declares no type produced no edge | `CollectionUtil` and `HashContainers` — ported, and unmeasurable — while six identical cases had been curated by hand one at a time | the module-as-`Component` rule in `port_evidence_kg.load` |
| `#[cfg(test)]` was read on the enclosing module only | test-only types recorded as production code | item-level attribute check |
| `portScope` was assigned by package-name prefix | Rucene's own five Java test fixtures counted as Lucene inner types | classify by the declaring file |
| `extract_lucene_kg.py` and `enrich_lucene_kg.py` hard-coded `/tmp/lucene-10.5.0` | the Lucene half of the pipeline could not run at all against the path `CLAUDE.md` §16.1 names | `--lucene-root`, defaulting to `/tmp/lucene1050` |
| No loader retried a busy store | an `rmp web` session was enough to leave a load half-applied, indistinguishable from real drift | bounded backoff in `kgio.rmp`; since rmp 1.18.0 a server owns the store, and `kgio.client` retries only a `graph write conflict` |
| The loaders called `rmp graph query`/`create`/`update`, which rmp 1.17 replaced with `serve`/`client` | from 2026-10-02 no sync could run, and the graph stayed at `5c68398` | every call through `kgio.client`; batches sized to the server's 5 s statement budget |
| One `rmp` invocation per statement | ~19,000 process launches to load the Lucene member surface | `UNWIND` batching in `kgio.unwind` |

## Materialization status

Counts measured at commit `1fa3e92` (surveyed 2026-09-06),
verified against the two real code trees by an audit that reads the Apache Lucene
10.5.0 clone and the crate directly rather than trusting the loaders.

| Label / Edge | Status |
|--------------|--------|
| `Project` | populated (2: Rucene, Apache Lucene Core 10.5.0) |
| `Module` | populated (36: the `rucene` crate and all 35 Lucene modules with Java sources, each with `inScope`) |
| `Package` | populated (39: 37 with Java files, plus `codecs.lucene103` and `internal`, which hold only subpackages) |
| `Class` | populated (2,109 Java types: 1,196 top-level `portScope='in'`, 908 `portScope='nested'`, 5 `portScope='out'` — Rucene's own Java test fixtures). Every one carries a `portState`: 1,196 in-scope `ported`, and on the nested half 527 `ported`, 354 `absorbed`, 27 `unported` |
| `Interface` / `Enum` / `Exception` / `Annotation` | declared but not materialised; Java types all carry `Class` with a `kind` |
| `Method` | populated (22,542 Lucene members: 13,228 methods, 2,103 constructors, 7,003 fields, 208 enum constants) |
| `File` | populated (1,946: 1,232 Lucene sources, 644 crate `.rs` files, 46 Java test fixtures, and 24 build, documentation, specification and KG-tooling files) |
| `RustStruct` / `RustTrait` / `RustEnum` / `RustAlias` | populated from the full crate survey (2,245 / 368 / 148 / 73 = 2,834), 118 of them `origin: "macro"` |
| `RustFn` | populated (15,174), 896 of them `origin: "macro"` |
| `Component` | populated (10), all with a non-null `status` |
| `Task` | populated (183): the 144 tasks `rmp` reported at `a5c1556`, with their live status, plus 39 nodes whose `rmp` task has since been deleted — see `Task` above |
| `Commit` | populated (276) — **the complete history**, matching `git rev-list --count HEAD` |
| `Decision` | populated (17), including `kind: "gap"` declared gaps, the port-scope principle, the measurement that replaced the nested-type assumption, and the evidence rule that governs `portState` |
| `Defect` | populated (9) |
| `Feature` | populated (40) |
| `CONTAINS` | populated (1,962: project→module, module→package, module→crate file, package→file/type) |
| `DECLARES` | populated (58,579: Java file→type, Java class→member, crate file→type/function, crate type→method, test class→test method) |
| `DEPENDS_ON` | populated (13,116: Lucene type→type and package→package, crate file→file, curated type→type, task→task) |
| `EXTENDS` / `IMPLEMENTS` | populated (1,395 / 2,194) |
| `NESTED_IN` | populated (908), one per nested Lucene type |
| `PORTS` | populated (1,788), **every edge carrying `evidence`**: 1,625 `doc-comment`, 163 `curated` |
| `PORTS_CANDIDATE` | **empty** (0): every candidate was superseded by a declared edge, and no type carries both |
| `REQUIRES_PORT` | populated (209 edges from tasks to the Lucene types their statements name) |
| `TESTS` / `SPECIFIED_IN` | populated (594 / 16) |
| `TestClass` | populated (971: 759 from `lucene/core/src/test`, 212 from `lucene/test-framework/src/java`), every one carrying a `role` and a `ruceneCoverage` |
| `TestMethod` | populated (5,746) |
| `COVERED_BY` | populated (208 edges from a `TestClass` to the Rust file that tests its subject) |
| `CLOSES_TASK` / `REFERENCES_TASK` | populated; `CLOSES_TASK` is decided by the `rmp` closings export, and by the messages only for tasks the export does not list (see the relationship table) |
| `REFERENCES` | populated (13) |
| `IMPLEMENTED_IN` / `COMMITTED_IN` / `DELIVERS` | populated (139 / 200 / 4) |

### What the audit checks, and what it found

`tools/kg/audit_kg.py` verifies the graph against reality, not against its own
loaders: it re-parses the Lucene clone with a brace-depth scanner written
independently of `extract_lucene_kg.py` (the two agreed on all 908 nested types in
both directions), and takes the crate's type census from the compiler's own
expansion. Run it after every sync; `problems: 0` is the pass condition.

At `1fa3e92` it passes on every check, and ends in `problems: 0`:

| Check | Graph | Reality |
|---|---|---|
| `lucene/core` Java files | 1,232 | 1,232 |
| top-level types (`portScope='in'`) | 1,196 | 1,196 |
| nested types (`portScope='nested'`) | 908 | 908 |
| Lucene modules registered | 35 | 35 |
| crate `.rs` files | 644 | 644 |
| crate types | 2,834 | 2,834 |
| crate functions | 15,174 | 15,174 |
| types the compiler sees that the graph lacks | 0 | — |
| `Class` without a `portScope` | 0 | — |
| nested types without a measured port state | 0 | — |
| `unported` nested types without a destination | 0 | — |
| `PORTS` edges without `evidence` | 0 | — |
| types with both `PORTS` and `PORTS_CANDIDATE` | 0 | — |
| `PORTS` edges from a node no survey confirms | 0 | — |
| commits | 276 | 276 |
| nodes without a provenance stamp | 0 | — |

The two nested checks are what keeps the exclusion from sliding back into an
assumption: a nested type with no `portState`, or an `unported` one with no
`portDestination`, fails the audit.

**Deleting is as much of the audit's job as counting.** At `a5c1556` the prune pass
of `load_rucene_kg.py` removed 5 type nodes and 41 `RustFn` that the commit itself
had deleted — `FlushIOContext`, `MergeIOContext`, `LabelQuery`, `ClonedQuery` and
`ResettableByteArrayOutput`, the invented `IOContext` types and their companions that
the fidelity fix dropped. A graph that only ever added would still be asserting those
types exist, and a coverage query would still be counting them. The two safeguards on
that phase, stated in `tools/kg/README.md` — never delete a node carrying a `PORTS`
edge, never accept an empty survey — are what make the deletion safe to run
unattended.

Types declared **inside a function body** are excluded on both sides, by the rule
stated under `RustStruct`, so they are not a divergence.

Labels that no longer exist, having been collapsed onto the canonical set:
`Struct`, `Trait`, `Enum` (Rust), `Interface` (Rust), `Test`, `TestSuite`,
`RustFile`. No node carries zero labels or more than one label.
