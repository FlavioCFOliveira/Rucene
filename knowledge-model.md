# Knowledge Graph Model — rucene

Knowledge graph for the **Rucene** project. It represents:

1. The local **Rucene** Rust crate structure (`src/`, tests, build files), surveyed
   file by file, type by type and function by function.
2. The reference **Apache Lucene Core 10.5.0** library structure that Rucene is
   porting, including packages, subpackages, source files, types, members, and the
   dependencies between them.
3. **What is ported, what is missing, and what to do next**: the Lucene surface that
   is in scope for the port, the state of each type in it, and the edges that let a
   query rank the remaining work by how much it unblocks.

This model follows the Label-Property Graph (LPG) paradigm used by `rmp graph`.

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
A top-level module / crate / Maven module. Exactly two nodes: `rucene` (the crate)
and `lucene/core` (the Maven module).

| Property | Purpose |
|----------|---------|
| `name` | Module name: `"rucene"`, `"lucene/core"`. |
| `kind` | `"crate"`, `"maven-module"`. |
| `path` | Root path in the source tree. |
| `gitCommit` / `gitDate` | Provenance stamp. |

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
| `gitCommit` / `gitDate` | Provenance stamp. |

#### `portScope` — which Lucene types the port has to cover

`portScope` makes the denominator of port coverage explicit in the graph instead of
leaving it implied, so a coverage query is defensible rather than a guess.

| Value | `portScopeRule` | Meaning |
|---|---|---|
| `in` | `lucene-core-top-level` | A **top-level** type declared by a file of the `lucene/core` module (`src/java` and `src/java21`). 1,196 types. This is the port target. |
| `nested` | `nested-in-enclosing-type` | An inner type. Excluded from the denominator: it is ported together with the type that encloses it, not independently. 860 types. |
| `out` | `not-a-lucene-core-type` | Anything else carrying a Java label. Currently empty. |

The rule is deliberately simple and mechanical so that
`tools/kg/port_coverage_kg.py` reproduces it from a clean graph, and so that no
scope decision is smuggled in: `CLAUDE.md` §1 names Apache Lucene Core 10.5.0 as
the reference source and demands functional parity plus 100% index compatibility,
and §16.1 names `lucene/core` as the canonical source tree, so the whole module is
the target. The decision is recorded as the `Decision` node *"Port scope is every
top-level type of lucene/core 10.5.0"*, with its alternatives and evidence.

#### `portState` — what is ported and what is missing

| Value | Meaning |
|---|---|
| `ported` | A curated `PORTS` edge points at this type from a Rucene node. |
| `candidate` | No `PORTS` edge, but exactly one Rucene type carries the same simple name, recorded as a `PORTS_CANDIDATE` edge. A port that is very probably done and **not yet confirmed in the graph** — the to-do list for the graph itself. |
| `unported` | Neither. |

`candidate` exists because `CLAUDE.md` §14.1 requires Rucene to keep Lucene's names,
which makes an exact name match strong evidence — but still evidence, not a fact.
Recording it on its own edge type keeps the curated `PORTS` free of inference
(`CLAUDE.md` §7).

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
| `kind` | `"source"`, `"test"`, `"build"`, `"doc"`, `"config"`, `"module-descriptor"`. |
| `language` | `"Rust"` for the crate's `.rs` files; absent for Lucene and non-code files. |
| `modulePath` | For a crate file, its Rust module path (`rucene::index::terms`) or, for an integration test, its test-crate name. |
| `crate` | `"rucene"`, or the `[[test]]` crate name for a test file. |
| `loc` | Line count at the surveyed commit. |
| `package` | For a Java file, its package. |
| `role` / `moduleName` | For `module-info.java`, the JPMS module name. |
| `gitCommit` / `gitDate` | Provenance stamp. |

Every `.rs` file under `src/` and `tests/` has exactly one `File` node — 157 at
`2855d29`, verified by `tools/kg/load_rucene_kg.py --phase audit`.

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
| `scope` | `"crate"` for production code, `"test"` for a type declared inside a `#[cfg(test)]` module. |
| `language` | Always `"Rust"`. |
| `gitCommit` / `gitDate` | Provenance stamp. |

Types declared **inside a function body** are local to that function and are not
modelled.

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
one of Lucene's static utility classes. Identity is `name`. Three nodes at
`2855d29`: `IndexFileNames` (`src/index/index_file_names.rs`), `VectorUtil`
(`src/util/vector_util.rs`) and `reader_util` (the inline `pub mod reader_util` in
`src/index/multi_reader.rs`).

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
An `rmp` task. **Every** task is mirrored — all 144 at the time of the `2855d29`
survey — not only the closed ones: without the open tasks the graph cannot answer
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

A few pre-2026-08 `Commit` nodes use `commitHash` instead of `hash`; new nodes
always use `hash`. Two nodes written by the 2026-08-26 syncs carry `subject`
instead of `message` and no `date`/`task_id`; new nodes always use `message`,
`date` and `task_id`.

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

## Edge types

| Edge | Meaning |
|------|---------|
| `CONTAINS` | `Project` → `Module`, `Module` → `Package`, `Module` → `File` (the crate's `.rs` files), `Package` → `Package`, `Package` → `Class`/`File`. |
| `DECLARES` | `File` → `Class` (Java top-level type), `Class` → `Method`, and, for the crate, `File` → `RustStruct`/`RustTrait`/`RustEnum`/`RustAlias`/`RustFn` and `RustStruct`/`RustTrait`/`RustEnum`/`RustAlias` → `RustFn` (the type declares that method). |
| `NESTED_IN` | `Class` (inner type) → `Class` (enclosing top-level type). |
| `DEPENDS_ON` | `Package` → `Package` and `Class` → `Class` on the Lucene side, both derived from `import` declarations plus same-package references; `File` → `File` on the Rucene side, derived from `use` declarations; Rucene type → Rucene type for curated dependencies, optionally carrying a `note`. Also `Task` → `Task`, mirroring the dependency `rmp` records, and `Decision` → `Task`, which a `kind: "gap"` decision uses to name the task that will close it. |
| `EXTENDS` | `Class` → `Class` / `Class` → `Interface`. |
| `IMPLEMENTS` | `Class` → `Interface` on the Lucene side; Rucene type → `RustTrait` (an `impl Trait for Type` block, restricted to traits the crate itself declares). Also used as `Feature` → `File`/`Class`: the file or type that realises the feature — a few early syncs (including `b0e1a75`) wrote that one the other way round, as `File` → `Feature`, and those edges are still present. |
| `PORTS` | Rucene node (`RustStruct`/`RustTrait`/`RustEnum`/`RustAlias`/`RustFn`/`Component`) → Lucene `Class`. The Rust item is the port of that Lucene type. Optional `note` records that the port is partial, a placeholder, or a deliberate adaptation. Always points at the **type**, never at the Java file. |
| `PORTS_CANDIDATE` | Rucene type → Lucene `Class`. There is exactly one Rucene type with the same simple name, so this is very probably a port that the graph has not confirmed. `evidence` says how it was derived (`"exact-name-match"`). Promote to `PORTS` once verified. |
| `REQUIRES_PORT` | `Task` → `Class`. The task's statement names that Lucene type, so an unported type on the other end blocks the task. Derived mechanically from the task's title and its functional/technical/acceptance text, restricted to unambiguous simple names of at least four characters. |
| `EXPORTS` / `OPENS` / `REQUIRES` / `USES` / `PROVIDES` | `Feature` (`module-info`) → `Package` / `Feature` / `Class` (JPMS and SPI declarations). |
| `PROVIDED_BY` | `Class` (SPI interface) → `Class` (implementation). |
| `TESTS` | `File` / `Class` → `Feature` / `Class` / Rucene type / `Component` / `Defect`. A portability test file points at the harness `Feature` it belongs to and at the Rucene types whose behaviour it pins down; where it is also the regression test for a fixed bug, it points at the `Defect` too. The origin is normally a file under `tests/`; a `src/` file is the origin when the regression test is a `#[cfg(test)]` unit test in the module itself. |
| `SPECIFIED_IN` | `Feature` → `File` (specification document). |
| `REFERENCES` | `Project` → `Project` (Rucene references Apache Lucene Core 10.5.0), `Project` → `Feature` (the project specification), and `Feature` → `Package` (the Lucene packages a Rucene capability covers). It is **not** a port relation: two `RustEnum` → `Class` edges written this way by an early sync were converted to `PORTS` at `2855d29`, their claim verified against the `Equivalent to …` doc comments in `src/index/mod.rs`. |
| `IMPLEMENTED_IN` | `Component`/`Task`/`Decision`/`Defect` → `File`/`Commit` (where the thing lives, landed, or was fixed). |
| `COMMITTED_IN` | `File`/`Feature`/`Component`/`Decision`/`Defect` → `Commit`. |
| `CLOSES_TASK` | `Commit` → `Task`. |
| `DELIVERS` | `Task` → `Feature`: the capability the task delivered. |
| `TESTED_BY` / `MODIFIES` / `FULFILLS` / `IMPLEMENTED_BY` | Legacy provenance edges from the first syncs; not used by new work. |

---

## Answering the three standing questions

### What is ported, and what is missing

```cypher
MATCH (c:Class)
WHERE c.portScope = 'in'
RETURN c.portState AS state, count(c) AS types
ORDER BY types DESC
```

At `2855d29`: `unported` 727, `candidate` 300, `ported` 169, over an in-scope
surface of 1,196 top-level `lucene/core` types.

To list what is missing in one package, add `AND c.package = '…'` and return
`c.qualifiedName`.

### What to do next

Rank the unported surface by how much it blocks — tasks plus other in-scope types
that depend on it:

```cypher
MATCH (u:Class)
WHERE u.portScope = 'in' AND u.portState = 'unported'
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

Edges written by the first (2026-07-30) Lucene survey were never stamped and are
still unstamped: `File → Class` `DECLARES` (1,196), `Class → Method` `DECLARES`
(~19,200), `Package`/`Module` `CONTAINS` (1,283), `NESTED_IN` (854), `EXTENDS`
(596), `Package → Package` `DEPENDS_ON`, and the JPMS/SPI edges. Repairing them is
`rmp` task #136. Every edge written from `2855d29` onward is stamped.

---

## Reproducing the graph

The loaders under `tools/kg/` rebuild the whole graph from a clean store; see
`tools/kg/README.md` for the exact order and arguments. In summary:

1. `extract_lucene_kg.py` + `run_kg_batches.py` — packages, files, top-level types,
   `DEPENDS_ON`, `EXTENDS`, `IMPLEMENTS` for `lucene/core`.
2. `enrich_lucene_kg.py` + `load_members_unwind.py` — inner types and members.
3. `extract_rucene_kg.py` → `load_rucene_kg.py` — the crate: files, types,
   functions, `DECLARES`, `IMPLEMENTS`, `DEPENDS_ON`, plus the hygiene passes that
   collapse legacy labels and merge duplicate nodes, and an `--phase audit` that
   compares the graph against the survey.
4. `port_coverage_kg.py` — `portScope`, `portState`, Lucene type→type `DEPENDS_ON`,
   `PORTS_CANDIDATE`, the `Task` mirror with `REQUIRES_PORT`, `Component.status`,
   and the scope `Decision`.

---

## Materialization status

Counts are those measured at commit `2855d29` (2026-08-30).

| Label / Edge | Status |
|--------------|--------|
| `Project` | populated (2: Rucene, Apache Lucene Core 10.5.0) |
| `Module` | populated (2: `rucene`, `lucene/core`) |
| `Package` | populated (40 packages under `org.apache.lucene` in `lucene/core`) |
| `Class` | populated (2,056 Lucene types: 1,196 top-level `portScope='in'` + 860 `portScope='nested'`) |
| `Interface` / `Enum` / `Exception` / `Annotation` | declared but not materialised; Java types all carry `Class` with a `kind` |
| `Method` | populated (17,953 Lucene members) |
| `File` | populated (1,424: 1,232 Lucene sources, 157 crate `.rs` files, and the project's build/doc/spec/fixture files) |
| `RustStruct` / `RustTrait` / `RustEnum` / `RustAlias` | populated from the full crate survey (889 / 170 / 59 / 35) |
| `RustFn` | populated from the full crate survey (6,575: 3,013 methods of inherent `impl` blocks, 1,741 tests, 1,046 free functions, 775 trait-method declarations) |
| `Component` | populated (3: the modules of free functions), all with a non-null `status` |
| `Task` | populated (144: every `rmp` task, with its live status) |
| `Commit` | populated (23) for the commits that have been synced; not a complete history |
| `Decision` | populated (11), including `kind: "gap"` declared gaps and the port-scope principle |
| `Defect` | populated (9), from `fd36286`, `0dfc12d` and `4015b12` |
| `Feature` | populated (40): JPMS module descriptors, Lucene-side capability groupings, and Rucene features created per synced commit |
| `CONTAINS` | populated (project→module, module→package, module→crate file, package→file/type) |
| `DECLARES` | populated (30,716: Java file→type, Java class→member, crate file→type/function, crate type→method) |
| `DEPENDS_ON` | populated (9,894: 8,544 Lucene type→type, 1,003 crate file→file, plus package→package, curated type→type and task→task) |
| `EXTENDS` / `IMPLEMENTS` | populated (596 / 999) |
| `PORTS` | populated (222 curated edges), all Rucene node → Lucene `Class` |
| `PORTS_CANDIDATE` | populated (300 name-match candidates awaiting confirmation) |
| `REQUIRES_PORT` | populated (141 edges from 32 open tasks) |
| `TESTS` / `SPECIFIED_IN` | populated for the portability harness and the components it validates; extended per synced commit |
| `REFERENCES` | populated (13: `Project` → `Project`/`Feature`, and 11 `Feature` → `Package`) |
| `IMPLEMENTED_IN` / `COMMITTED_IN` / `CLOSES_TASK` / `DELIVERS` | populated per synced commit |

Labels that no longer exist, having been collapsed onto the canonical set by the
`2855d29` survey: `Struct`, `Trait`, `Enum` (Rust), `Interface` (Rust), `Test`,
`TestSuite`, `RustFile`. No node carries zero labels or more than one label.
