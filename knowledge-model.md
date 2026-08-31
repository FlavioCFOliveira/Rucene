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
| `portState` | `"ported"`, `"candidate"`, `"unported"` — see below. |
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
| `nested` | `nested-in-enclosing-type` | An inner type. Excluded from the denominator: it is ported together with the type that encloses it, not independently. 908 types. |
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

#### `portState` — what is ported and what is missing

| Value | Count at `41051f8` | Meaning |
|---|---|---|
| `ported` | 1,003 | Something *asserts* that a Rucene item is the port of this type: a `PORTS` edge, whose `evidence` says on what basis. |
| `candidate` | 193 | No assertion, but exactly one Rucene type carries the same simple name, recorded as a `PORTS_CANDIDATE` edge. Probable, **unverified**, and the graph's own to-do list. |
| `unported` | 0 | Neither. |

##### The evidence ladder

The point of `evidence` on the `PORTS` edge is that a coverage number can be read
back to the thing that justifies it. Every edge carries one; none is unstated.

| `evidence` | Edges | What it rests on |
|---|---|---|
| `doc-comment` | 940 | The Rust item's own doc comment says `Equivalent to \`org.apache.lucene.X\`` (or `Port of`, `Ported from`, `Lucene Core equivalent:`). `CLAUDE.md` §14.1 requires that line, so this is **the port declaring what it is**, attached to one specific item. `declaredAt` gives the `file:line`. |
| `curated` | 137 | A hand-verified edge, including the cases the mechanical rules provably get wrong. |

**Why the declared evidence changed the picture.** Before this pass the graph read
*381 ported, 815 candidate*: two thirds of the coverage claim rested on a bare
simple-name coincidence, which the model already warned had to be quoted as
unverified. The crate, however, states 1,147 equivalence claims across 530 files —
997 distinct Lucene types — and 940 of them resolve to an item-level `PORTS` edge.
That is not an inference the graph makes; it is a statement the code makes, which
`CLAUDE.md` §7 admits as verified knowledge where a name match never could be.

38 of those claims map a Rust item to a Lucene type of a **different** name —
`TotalHitsRelation` → `TotalHits.Relation`, `NoOpLock` → `NoLock`,
`DefaultIndexingChain` → `IndexingChain`, `SorterDocMap` → `Sorter.DocMap`. A
name-match heuristic could never have produced any of them, and would never have
been right to guess them.

##### Depth — `ported` does not mean complete

A `PORTS` edge says a Rust item claims to be a Lucene type. It says nothing about
how much of that type exists, so a stub and a finished port read identically.
`memberCoverage` measures the difference: the fraction of the Lucene type's methods
whose name maps to a function the port provides, counting the type's inherent
methods, the free functions of its file, and the methods of every trait it
implements.

At `41051f8`, over the 857 ported types that declare methods: **mean coverage
68.7%**, distributed

| Coverage | Types |
|---|---|
| 100% | 289 |
| 75–99% | 172 |
| 50–74% | 191 |
| 1–49% | 131 |
| 0% | 74 |

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
  and the doc-values iterators — 67 types in all — are produced by `macro_rules!`
  macros standing in for the code generators that write Lucene's own files. The
  survey now records them from the compiler's own expansion; see `origin` under
  `RustStruct` below.
* **Declared with a lifetime parameter.** `StringSorter` is `struct StringSorter<'a>`,
  which the regex does not match.
* **No type at all.** `CollectionUtil` is a module of free functions; `MergedIterator`
  pre-existed under a different module than the one the name search reached.
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
| `kind` | `"source"`, `"test"`, `"test-fixture"`, `"build"`, `"doc"`, `"config"`, `"module-descriptor"`. `"test-fixture"` is a Java program under `tests/fixtures/` that drives real Lucene 10.5.0 to emit reference data; those 24 files carry `language: "Java"` and `crate: "java-codec-harness"`. |
| `language` | `"Rust"` for the crate's `.rs` files; absent for Lucene and non-code files. |
| `modulePath` | For a crate file, its Rust module path (`rucene::index::terms`) or, for an integration test, its test-crate name. |
| `crate` | `"rucene"`, or the `[[test]]` crate name for a test file. |
| `loc` | Line count at the surveyed commit. |
| `package` | For a Java file, its package. |
| `role` / `moduleName` | For `module-info.java`, the JPMS module name. |
| `gitCommit` / `gitDate` | Provenance stamp. |

Every `.rs` file under `src/` and `tests/` has exactly one `File` node — 622 at
`41051f8`, verified by `tools/kg/load_rucene_kg.py --phase audit`.

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
The same pass supplies 860 macro-generated functions.

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
all. Identity is `name`. Eight nodes, among them `IndexFileNames` (`src/index/index_file_names.rs`),
`VectorUtil` (`src/util/vector_util.rs`), `reader_util` (the inline `pub mod reader_util` in
`src/index/multi_reader.rs`), and `MultiLeafReader` (`src/index/multi_leaf_reader.rs`), added
2026-08-30 for the empty-class case described under `portState`.

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
An `rmp` task. **Every** task is mirrored — all 152 at `41051f8` — not only the
closed ones: without the open tasks the graph cannot answer
"what to do next". `tools/kg/port_coverage_kg.py --phase tasks` refreshes them.

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

**Every commit of the repository is mirrored** — 165 at `41051f8`, reproduced by
`tools/kg/commits_kg.py`, which also derives `CLOSES_TASK` from the conventions
the history actually uses (`Task #N`, `Tasks #N and #M`, `Closes rmp task #N`).
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
| `covered` | 164 | The Lucene type this test exercises is ported, and the Rust file declaring the port also declares at least one `#[test]`. A `COVERED_BY` edge names that file. |
| `uncovered` | 259 | The type is ported but its Rust file declares no test. |
| `subject-unported` | 21 | The type this test exercises is not ported. |
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
| `EXTENDS` | `Class` → `Class` / `Class` → `Interface`; also `TestClass` → `TestClass`, which is how the 564 core tests reach `LuceneTestCase` and how a codec test reaches the `Base…TestCase` suite whose cases it inherits. 799 edges. |
| `IMPLEMENTS` | `Class` → `Interface` on the Lucene side; Rucene type → `RustTrait` (an `impl Trait for Type` block, restricted to traits the crate itself declares). Also used as `Feature` → `File`/`Class`: the file or type that realises the feature — a few early syncs (including `b0e1a75`) wrote that one the other way round, as `File` → `Feature`, and those edges are still present. |
| `PORTS` | Rucene node (`RustStruct`/`RustTrait`/`RustEnum`/`RustAlias`/`RustFn`/`Component`) → Lucene `Class`. The Rust item is the port of that Lucene type. **`evidence` is mandatory** and is `"doc-comment"` or `"curated"` — see the evidence ladder above; `declaredAt` gives the `file:line` of a declared one. Optional `note` records that the port is partial, a placeholder, or a deliberate adaptation. Always points at the **type**, never at the Java file. 1,077 edges. |
| `PORTS_CANDIDATE` | Rucene type → Lucene `Class`. There is exactly one Rucene type with the same simple name, so this is very probably a port that the graph has not confirmed. `evidence` says how it was derived (`"exact-name-match"`). Promote to `PORTS` once verified. |
| `REQUIRES_PORT` | `Task` → `Class`. The task's statement names that Lucene type, so an unported type on the other end blocks the task. Derived mechanically from the task's title and its functional/technical/acceptance text, restricted to unambiguous simple names of at least four characters. |
| `EXPORTS` / `OPENS` / `REQUIRES` / `USES` / `PROVIDES` | `Feature` (`module-info`) → `Package` / `Feature` / `Class` (JPMS and SPI declarations). |
| `PROVIDED_BY` | `Class` (SPI interface) → `Class` (implementation). |
| `TESTS` | `File` / `Class` → `Feature` / `Class` / Rucene type / `Component` / `Defect`. A portability test file points at the harness `Feature` it belongs to and at the Rucene types whose behaviour it pins down; where it is also the regression test for a fixed bug, it points at the `Defect` too. The origin is normally a file under `tests/`; a `src/` file is the origin when the regression test is a `#[cfg(test)]` unit test in the module itself. Also `TestClass` → `Class`: the `lucene/core` type a Lucene test exercises, derived from Lucene's naming convention (`TestFoo` → `Foo`, `FooTest` → `Foo`, `BaseFooTestCase` → `Foo`) and carrying `evidence: name-convention`. It is a deduction, not a fact, and the edge says so; a name matching no convention gets no edge rather than a guessed one. 444 edges. |
| `COVERED_BY` | `TestClass` → `File`. The Rust file that declares the port of the type this Lucene test exercises, and that declares at least one `#[test]`. `evidence` is `ported-type-file-has-tests`, which is exactly what it proves — see `ruceneCoverage` above. |
| `SPECIFIED_IN` | `Feature` → `File` (specification document). |
| `REFERENCES` | `Project` → `Project` (Rucene references Apache Lucene Core 10.5.0), `Project` → `Feature` (the project specification), and `Feature` → `Package` (the Lucene packages a Rucene capability covers). It is **not** a port relation: two `RustEnum` → `Class` edges written this way by an early sync were converted to `PORTS` at `2855d29`, their claim verified against the `Equivalent to …` doc comments in `src/index/mod.rs`. |
| `IMPLEMENTED_IN` | `Component`/`Task`/`Decision`/`Defect` → `File`/`Commit` (where the thing lives, landed, or was fixed). |
| `COMMITTED_IN` | `File`/`Feature`/`Component`/`Decision`/`Defect` → `Commit`. |
| `CLOSES_TASK` | `Commit` → `Task`. |
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

At `41051f8` (2026-08-31), over an in-scope surface of 1,196 top-level
`lucene/core` types: **`ported` 1,003, `candidate` 193, `unported` 0.**

Read together with the evidence ladder, the honest sentence is: *1,003 types
(83.9%) carry an assertion that they are ported — 940 of them stated by the code
itself, 137 curated by hand — and 193 (16.1%) rest only on a unique simple-name
match and are unverified.*

The coverage of a type is not its completeness. Pair the two:

```cypher
MATCH (c:Class) WHERE c.portScope = 'in' AND c.portState = 'ported'
RETURN c.package AS package,
       count(c) AS types,
       avg(c.memberCoverage) AS meanDepth
ORDER BY meanDepth ASC
```

The thinnest ports at `41051f8` are `internal.hppc` (10%), `codecs.hnsw` (23%)
and `document` (44%); the mean over all 857 measured types is 68.7%.

Where the unverified share still sits:

| Package | ported | candidate | in scope |
|---|---|---|---|
| `org.apache.lucene.search` | 136 | 85 | 221 |
| `org.apache.lucene.analysis.tokenattributes` | 1 | 23 | 24 |
| `org.apache.lucene.util` | 103 | 15 | 118 |
| `org.apache.lucene.util.fst` | 10 | 14 | 24 |

To list what is unverified in one package, add `AND c.package = '…'` and return
`c.qualifiedName`.

**Around sixty ports carry a `note` on their `PORTS` edge recording a declared
divergence** — read them before assuming a type is complete:

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

### What to do next

Rank the unverified surface by how much it blocks — tasks plus other in-scope
types that depend on it:

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

With `unported` now empty, the more useful form of "what to do next" is the
shallowest port that the most work depends on:

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

---

## Query pitfalls

**`NOT EXISTS { MATCH … WHERE … }` returns the wrong answer on this engine.** Asking which
in-scope unported types have no open task with

```cypher
WHERE NOT EXISTS { MATCH (t:Task)-[:REQUIRES_PORT]->(u) WHERE t.status <> 'COMPLETED' }
```

returned **all 56** types of `org.apache.lucene.index`, including ones a direct `MATCH` proves
are required by an open task. Verified 2026-08-30. Use `OPTIONAL MATCH` and count instead:

```cypher
MATCH (u:Class) WHERE u.portScope='in' AND u.portState='unported'
OPTIONAL MATCH (t:Task)-[:REQUIRES_PORT]->(u) WHERE t.status <> 'COMPLETED'
RETURN u.name AS type, count(DISTINCT t) AS openTasks
```

That form gives 50 covered and 6 orphaned, which matches the task texts.

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

**Engine quirk that shapes the graph.** `MERGE (a)-[:X]->(b)` matches an existing
relationship of a *different* type between the same ordered pair and creates
nothing. Two node pairs may therefore hold only one edge, and a second edge type
between an already-connected pair has to be added by deleting the first. This is
what hid the `DocValuesType` and `IndexOptions` ports behind their `REFERENCES`
edges until `2855d29`. Undirected patterns are affected too: they report the
forward edge's type for a reverse traversal, so every read must be written
outgoing.

**Edges written by the first (2026-07-30) Lucene survey were never stamped, and
most still are not.** `rmp` task #136 tracks repairing them. The `41051f8` sync
stamped what it actually re-derived from the reference clone — `NESTED_IN` (908,
all of them) and the Lucene member `DECLARES` — and deliberately left the rest
alone: a stamp means *this fact was last confirmed at this commit*, so stamping an
edge this sync did not re-verify would be a false claim, which is exactly what the
convention exists to prevent.

Unstamped at `41051f8`, by edge type:

| Edge | Unstamped | Why it is still open |
|---|---|---|
| `DECLARES` | Java `File → Class` | not re-derived by this sync |
| `EXTENDS` | 1,395 | ditto |
| `CONTAINS` | 1,309 | ditto |
| `TESTS` | 444 | written by the earlier test survey |
| `IMPLEMENTS` (Lucene) | 117 | not re-derived by this sync |
| `COVERED_BY` | 165 | written by the earlier test survey |
| JPMS/SPI (`EXPORTS`, `USES`, …) | 63 | not re-derived by this sync |

Every edge this sync wrote — the 1,077 `PORTS`, the 193 `PORTS_CANDIDATE`, the 908
`NESTED_IN`, the crate's `DECLARES`/`IMPLEMENTS`/`DEPENDS_ON`, the `Task` and
`Commit` mirrors — carries `gitCommit` and `gitDate`.

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
5. `port_evidence_kg.py` — the declared `PORTS` edges and the depth measurement.
   Re-run step 4's `scope` and `candidates` phases afterwards so `portState`
   reflects them.
6. `commits_kg.py` — the full commit history and `CLOSES_TASK`.
7. `lucene_tests_kg.py` — the Lucene test surface and its correspondence with the crate.

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
| `#[cfg(test)]` was read on the enclosing module only | test-only types recorded as production code | item-level attribute check |
| `portScope` was assigned by package-name prefix | Rucene's own five Java test fixtures counted as Lucene inner types | classify by the declaring file |
| `extract_lucene_kg.py` and `enrich_lucene_kg.py` hard-coded `/tmp/lucene-10.5.0` | the Lucene half of the pipeline could not run at all against the path `CLAUDE.md` §16.1 names | `--lucene-root`, defaulting to `/tmp/lucene1050` |
| No loader retried a busy store | an `rmp web` session was enough to leave a load half-applied, indistinguishable from real drift | bounded backoff in `kgio.rmp` |
| One `rmp` invocation per statement | ~19,000 process launches to load the Lucene member surface | `UNWIND` batching in `kgio.unwind` |

## Materialization status

Counts measured at commit `41051f8` (2026-08-31), verified against the two real
code trees by an audit that reads the Apache Lucene 10.5.0 clone and the crate
directly rather than trusting the loaders.

| Label / Edge | Status |
|--------------|--------|
| `Project` | populated (2: Rucene, Apache Lucene Core 10.5.0) |
| `Module` | populated (36: the `rucene` crate and all 35 Lucene modules with Java sources, each with `inScope`) |
| `Package` | populated (39: 37 with Java files, plus `codecs.lucene103` and `internal`, which hold only subpackages) |
| `Class` | populated (2,109 Java types: 1,196 top-level `portScope='in'`, 908 `portScope='nested'`, 5 `portScope='out'` — Rucene's own Java test fixtures) |
| `Interface` / `Enum` / `Exception` / `Annotation` | declared but not materialised; Java types all carry `Class` with a `kind` |
| `Method` | populated (18,020 Lucene members: 12,045 methods, 1,552 constructors, 4,423 fields) |
| `File` | populated (1,901: 1,232 Lucene sources, 622 crate `.rs` files, and the project's build/doc/spec/fixture files, including the KG tooling) |
| `RustStruct` / `RustTrait` / `RustEnum` / `RustAlias` | populated from the full crate survey (2,050 / 362 / 138 / 61), 118 of them `origin: "macro"` |
| `RustFn` | populated (13,081), 860 of them `origin: "macro"` |
| `Component` | populated (8), all with a non-null `status` |
| `Task` | populated (152: every `rmp` task, with its live status — 113 COMPLETED, 24 SPRINT, 14 BACKLOG, 1 DOING) |
| `Commit` | populated (165) — **the complete history**, matching `git rev-list --count HEAD` |
| `Decision` | populated (15), including `kind: "gap"` declared gaps, the port-scope principle, and the evidence rule that governs `portState` |
| `Defect` | populated (9) |
| `Feature` | populated (40) |
| `CONTAINS` | populated (1,940: project→module, module→package, module→crate file, package→file/type) |
| `DECLARES` | populated (50,308: Java file→type, Java class→member, crate file→type/function, crate type→method, test class→test method) |
| `DEPENDS_ON` | populated (12,677: Lucene type→type and package→package, crate file→file, curated type→type, task→task) |
| `EXTENDS` / `IMPLEMENTS` | populated (1,395 / 1,841) |
| `NESTED_IN` | populated (908), one per nested Lucene type |
| `PORTS` | populated (1,077), **every edge carrying `evidence`**: 940 `doc-comment`, 137 `curated` |
| `PORTS_CANDIDATE` | populated (193 name-match candidates awaiting confirmation); no type carries both a `PORTS` and a `PORTS_CANDIDATE` |
| `REQUIRES_PORT` | populated (199 edges from the open tasks) |
| `TESTS` / `SPECIFIED_IN` | populated (594 / 16) |
| `TestClass` | populated (971: 759 from `lucene/core/src/test`, 212 from `lucene/test-framework/src/java`), every one carrying a `role` and a `ruceneCoverage` |
| `TestMethod` | populated (5,746) |
| `COVERED_BY` | populated (165 edges from a `TestClass` to the Rust file that tests its subject) |
| `CLOSES_TASK` | populated (108), derived from the commit messages |
| `REFERENCES` | populated (13) |
| `IMPLEMENTED_IN` / `COMMITTED_IN` / `DELIVERS` | populated (137 / 200 / 4) |

### What the audit checks, and what it found

`tools/kg/audit_kg.py` verifies the graph against reality, not against its own
loaders: it re-parses the Lucene clone with a brace-depth scanner written
independently of `extract_lucene_kg.py` (the two agreed on all 908 nested types in
both directions), and takes the crate's type census from the compiler's own
expansion. Run it after every sync; `problems: 0` is the pass condition.

At `41051f8` it passes on every check:

| Check | Graph | Reality |
|---|---|---|
| `lucene/core` Java files | 1,232 | 1,232 |
| top-level types (`portScope='in'`) | 1,196 | 1,196 |
| nested types (`portScope='nested'`) | 908 | 908 |
| crate `.rs` files | 622 | 622 |
| crate types | 2,611 | 2,611 |
| crate functions | 13,081 | 13,081 |
| types the compiler sees that the graph lacks | 0 | — |
| Lucene modules registered | 35 | 35 |
| commits | 165 | 165 |
| `Class` without a `portScope` | 0 | — |
| `PORTS` edges without `evidence` | 0 | — |
| types with both `PORTS` and `PORTS_CANDIDATE` | 0 | — |
| `PORTS` edges from a node no survey confirms | 0 | — |
| nodes without a provenance stamp | 0 | — |

Types declared **inside a function body** are excluded on both sides, by the rule
stated under `RustStruct`, so they are not a divergence.

Labels that no longer exist, having been collapsed onto the canonical set:
`Struct`, `Trait`, `Enum` (Rust), `Interface` (Rust), `Test`, `TestSuite`,
`RustFile`. No node carries zero labels or more than one label.
