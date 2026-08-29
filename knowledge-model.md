# Knowledge Graph Model — rucene

Knowledge graph for the **Rucene** project. It represents:

1. The local **Rucene** Rust crate structure (`src/`, tests, build files).
2. The reference **Apache Lucene Core 10.5.0** library structure that Rucene is porting, including packages, subpackages, source files, and the dependencies between them.

This model follows the Label-Property Graph (LPG) paradigm used by `rmp graph`.

---

## Node labels

### `Project`
A software project or library being modelled.

| Property | Purpose |
|----------|---------|
| `name` | Unique project name, e.g. `"rucene"` or `"Apache Lucene Core 10.5.0"`. |
| `language` | Primary language, e.g. `"Rust"` or `"Java"`. |
| `version` | Version string, e.g. `"10.5.0"`. |
| `gitCommit` | Hash of the local commit at which this node was last confirmed. |
| `gitDate` | ISO date of `gitCommit`. |
| `referenceUrl` | Authoritative source URL for external projects (e.g. GitHub tag URL). |

### `Module`
A top-level module / crate / Maven module.

| Property | Purpose |
|----------|---------|
| `name` | Module name, e.g. `"lucene/core"`, `"lucene/demo"`. |
| `kind` | `"crate"`, `"maven-module"`, `"java-module"`. |
| `path` | Root path in the source tree. |
| `gitCommit` | Last confirmed commit hash. |
| `gitDate` | ISO date of `gitCommit`. |

### `Package`
A Java package or Rust module namespace. Identity is scoped by its parent `Module` / `Project`.

| Property | Purpose |
|----------|---------|
| `name` | Fully qualified package name, e.g. `"org.apache.lucene.index"`. |
| `shortName` | Last segment, e.g. `"index"`. |
| `path` | URL or directory path. |
| `gitCommit` | Last confirmed commit hash. |
| `gitDate` | ISO date of `gitCommit`. |

### `Class`, `Interface`, `Enum`, `Exception`, `Annotation`
Java type declarations (and Rust types when the local crate is populated).

| Property | Purpose |
|----------|---------|
| `name` | Simple type name. |
| `qualifiedName` | Fully qualified name, e.g. `"org.apache.lucene.index.IndexWriter"`. |
| `kind` | `"class"`, `"interface"`, `"enum"`, `"record"`, `"exception"`, `"annotation"`, `"struct"`, `"trait"`. |
| `file` | Source file path or URL. |
| `extendsExternal` | List of external (`java.*`, `javax.*`, inner-class) super-types not yet modelled as nodes. |
| `implementsExternal` | List of external interfaces not yet modelled as nodes. |
| `gitCommit` | Last confirmed commit hash. |
| `gitDate` | ISO date of `gitCommit`. |

### `Method`
A method, constructor, or field member.

| Property | Purpose |
|----------|---------|
| `name` | Member name. |
| `signature` | Simplified signature (return type + name + parameter list). |
| `kind` | `"method"`, `"constructor"`, `"field"`. |
| `modifiers` | Access and other modifiers (`public`, `protected`, `static`, `final`, etc.). |
| `returnType` | Return/field type string (when available). |
| `parentQualifiedName` | Enclosing class qualified name. |
| `gitCommit` | Last confirmed commit hash. |
| `gitDate` | ISO date of `gitCommit`. |

### `File`
A source, build, documentation, or configuration file.

| Property | Purpose |
|----------|---------|
| `path` | Relative path in the repository. |
| `name` | File name. |
| `kind` | `"source"`, `"test"`, `"build"`, `"doc"`, `"config"`, `"module-descriptor"`. |
| `role` | Additional discriminator, e.g. `"module-descriptor"`. |
| `moduleName` | For `module-info.java`, the JPMS module name. |
| `gitCommit` | Last confirmed commit hash. |
| `gitDate` | ISO date of `gitCommit`. |

### `RustStruct`, `RustTrait`, `RustEnum`
A type declared by the local Rucene crate. Identity is `{name, file}`, so two
types with the same name in different modules stay distinct.

| Property | Purpose |
|----------|---------|
| `name` | Rust type name, e.g. `"DocumentsWriterFlushControl"`. |
| `file` | Path of the declaring file, relative to the repository root, e.g. `"src/index/documents_writer.rs"`. |
| `kind` | `"struct"`, `"trait"`, `"enum"`. |
| `language` | Always `"Rust"`. |
| `gitCommit` | Last confirmed commit hash. |
| `gitDate` | ISO date of `gitCommit`. |

Older syncs registered Rust types under the Java labels (`Class`, `Interface`,
`Struct`, `Trait`) or under `Component`; those nodes are still present and are
told apart from the Java ones by a `file` that starts with `src/`. When such a
legacy node turns out to be an exact duplicate of a current-convention node —
same type name, same `src/` file — the sync that touches that file removes the
legacy one, so that a type is never represented twice. The 2026-08-27 sync did
this for the four Lucene90 term-vectors types.

### `Component`
A named unit of the local crate registered before the `Rust*` labels existed
(mostly `src/util.rs` and `src/store.rs` items). Identity is `name`. New work
uses `RustStruct`/`RustTrait`/`RustEnum` for anything that declares a Rust type,
and keeps `Component` for a crate unit that declares no Rust type: a module of
free functions — the ports of Lucene's static utility classes, such as
`ArrayUtil`, `BitUtil`, `IOUtils`, `NumericUtils` and `VectorUtil` — or a single
free function that is a load-bearing port in its own right, which carries
`kind: "function"` and a `file`, and reaches its file through `IMPLEMENTED_IN`.
`compare_utf16` in `src/util/string_helper.rs` is the first of these: it ports
the `String.compareTo` ordering Lucene writes field names in. The 2026-08-29
sync added two more: `doc_values_flush_order` in `src/index/indexing_chain.rs`,
which reproduces the field-hash table order `IndexingChain.writeDocValues`
flushes in and so fixes the order of the field entries inside the `.dvm`, and
`register_doc_values_format` in `src/codecs/doc_values.rs`, which fills the
global registry Rucene uses in place of the Java service loader behind
`DocValuesFormat.forName`. `0dfc12d` added `inflate_gens` in
`src/index/index_file_deleter.rs`, the port of the private static
`IndexFileDeleter.inflateGens`, which pushes a `SegmentInfos` name counter and
its per-segment generations past everything already present in the directory.

### `Task`
An `rmp` task, mirrored into the graph when it is closed.

| Property | Purpose |
|----------|---------|
| `id` | The `rmp` task number (identity). |
| `name` | Task title. |
| `status` | `rmp` status at the time of the sync, e.g. `"COMPLETED"`. |
| `components` | Comma-separated Rust paths (`rucene::<module>::<Type>`) delivered by the task. |
| `gitCommit` | Commit that closed the task. |
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
| `gitCommit` | Last confirmed commit hash. |
| `gitDate` | ISO date of `gitCommit`. |

A `Decision` reaches the code through `IMPLEMENTED_IN` (→ `File`) and records
where it landed through `COMMITTED_IN` (→ `Commit`).

`kind: "principle"` is for a project-wide rule recorded in `CLAUDE.md` that
governs every later task, rather than a choice confined to one component. The
first is `"Fidelity first - minimise divergences (CLAUDE.md 14.5)"`, added by
`fd36286`.

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
| `name` | Short defect title (identity), e.g. `"read_sorted_set always took the multi-valued SORTED_SET layout"`. |
| `kind` | `"portability"` (a divergence from Lucene 10.5.0) or `"robustness"` (a panic, abort or unbounded allocation reachable from a corrupt file). |
| `summary` | What was wrong, in one or two sentences. |
| `cause` | Why the code behaved that way, and why it was not caught earlier. |
| `fix` | What changed, with the `file:line` of the corrected code. |
| `luceneReference` | The Apache Lucene Core 10.5.0 file and lines that define the correct behaviour (required for `kind: "portability"`, per `CLAUDE.md` §14.5). |
| `foundBy` | The test that exposed it. |
| `gitCommit` | Commit that fixed it. |
| `gitDate` | ISO date of `gitCommit`. |

A `Defect` reaches the code through `IMPLEMENTED_IN` (→ `File`, where the fix
landed), records where it landed through `COMMITTED_IN` (→ `Commit`), and is
pinned down by the regression test that points at it with `TESTS`. That test is
normally a file under `tests/`; when it is a `#[cfg(test)]` unit test inside the
module itself, the `src/` file is the `TESTS` origin instead.

### `Feature`
A high-level functional capability, used to link packages/types to what they implement.

| Property | Purpose |
|----------|---------|
| `name` | Feature name. |
| `description` | Short description. |

---

## Edge types

| Edge | Meaning |
|------|---------|
| `CONTAINS` | `Project` → `Module`, `Module` → `Package`, `Package` → `Package`, `Package` → `Class`/`Interface`/`Enum`/`Exception`/`Annotation`, `Class` → `Method`. |
| `DECLARES` | `File` → `Class`/`Interface`/`Enum`/`Exception`/`Annotation`/`Method`, and, for local crate files, `File` → `RustStruct`/`RustTrait`/`RustEnum`. |
| `NESTED_IN` | `Class` (inner type) → `Class` (enclosing top-level type). |
| `DEPENDS_ON` | `Package` → `Package`, `Class` → `Class`, `Module` → `Module`, and Rucene type → Rucene type (`RustStruct`/`RustTrait`/`RustEnum`/`Component`/legacy `Trait`/legacy `Interface`), optionally carrying a `note` that says what the dependency is. Also `Task` → `Task`, mirroring the dependency `rmp` records, and `Decision` → `Task`, which a `kind: "gap"` decision uses to name the task that will close it. |
| `EXTENDS` | `Class` → `Class` / `Class` → `Interface`. |
| `IMPLEMENTS` | `Class` → `Interface`, and Rucene type → `RustTrait` (the Rust type implements that trait). |
| `EXPORTS` | `Feature` (`module-info`) → `Package` (JPMS exported package). |
| `OPENS` | `Feature` (`module-info`) → `Package` (JPMS opened package). |
| `REQUIRES` | `Feature` (`module-info`) → `Feature` (required module). |
| `USES` | `Feature` (`module-info`) → `Class` (SPI service interface). |
| `PROVIDES` | `Feature` (`module-info`) → `Class` (SPI service interface). |
| `PROVIDED_BY` | `Class` (SPI interface) → `Class` (implementation). |
| `TESTS` | `File` / `Class` → `Feature` / `Class` / `RustStruct`/`RustTrait`/`RustEnum`/`Component`/`Defect`. A portability test file points at the harness `Feature` it belongs to and at the Rucene types whose behaviour it pins down; where it is also the regression test for a fixed bug, it points at the `Defect` too. The origin is normally a file under `tests/`; a `src/` file is the origin when the regression test for a `Defect` is a `#[cfg(test)]` unit test in the module itself. |
| `SPECIFIED_IN` | `Feature` → `File` (specification document). |
| `REFERENCES` | `Project` → `Project` (Rucene references Apache Lucene Core 10.5.0). |
| `PORTS` | Rucene type (`RustStruct`/`RustTrait`/`RustEnum`/`Component`) → Lucene `Class`/`Interface`/`Enum`. The Rust type is the port of that Lucene type. Optional `note` property records that the port is partial, a placeholder, or a deliberate adaptation, and says what is missing or what was changed and why. |
| `IMPLEMENTED_IN` | `Component`/`Task`/`Decision`/`Defect` → `File`/`Module`/`Commit` (where the thing lives, landed, or was fixed). |
| `IMPLEMENTS` | Also used as `Feature` → `File`/`Class`: the file or type that realises the feature. This is the current direction; a few early syncs (including `b0e1a75`) wrote it the other way round, as `File` → `Feature`, and those edges are still present. |
| `COMMITTED_IN` | `File`/`Feature`/`Component`/`Decision`/`Defect` → `Commit`. |
| `CLOSES_TASK` | `Commit` → `Task`. |
| `DELIVERS` | `Task` → `Feature`: the capability the task delivered. Reintroduced by `b0e1a75` and used by new work alongside `Task` → `File` `IMPLEMENTED_IN`. |
| `TESTED_BY` / `MODIFIES` / `FULFILLS` | Legacy provenance edges from the first syncs; not used by new work. |

---

## Provenance convention

Every node and edge carries `gitCommit` (full 40-char hash) and `gitDate` (`YYYY-MM-DD`) stamping when the fact was last confirmed. For nodes describing the external Apache Lucene source, the provenance records the **local Rucene commit** at the time of discovery/registration.

---

## Materialization status

| Label / Edge | Status |
|--------------|--------|
| `Project` | populated (Rucene, Apache Lucene Core 10.5.0) |
| `Module` | populated (`lucene/core`) |
| `Package` | populated (39 packages under `org.apache.lucene` in `lucene/core`) |
| `Class`/`Interface`/`Enum`/`Exception`/`Annotation` | populated (1,196 top-level types from `lucene/core`, including `src/java` and `src/java21` sources); inner classes are not yet modelled |
| `Method` | target — populated selectively for key APIs |
| `File` | populated (1,232 source files from `lucene/core` — `src/java`, `src/java21` and `module-info.java` — plus local project files) |
| `Feature` | populated (36 nodes): JPMS module descriptors, Lucene-side capability groupings, and Rucene features created per synced commit |
| `CONTAINS` | populated (project→module, module→package, package→file) |
| `DECLARES` | populated (file→top-level type, Java and Rust) |
| `DEPENDS_ON` | populated (package→package dependencies derived from imports; type→type added per synced commit) |
| `EXTENDS` / `IMPLEMENTS` | populated (type→type relationships) |
| `REFERENCES` | populated (Rucene → Apache Lucene Core 10.5.0) |
| `TESTS` / `SPECIFIED_IN` | populated for the portability harness and the components it validates; extended per synced commit |
| `RustStruct` / `RustTrait` / `RustEnum` | populated incrementally, one sync per commit that ports types |
| `Task` / `Commit` | populated for the commits that have been synced; not a complete history |
| `Decision` | populated per synced commit, for decisions that constrain the code, including `kind: "gap"` declared gaps |
| `Defect` | populated per synced commit, for non-obvious bugs found and fixed (4 nodes, from `fd36286` and `0dfc12d`) |
| `PORTS` | populated for every ported Rucene type whose Lucene counterpart is already modelled |
| `IMPLEMENTED_IN` / `IMPLEMENTS` / `COMMITTED_IN` / `CLOSES_TASK` | populated per synced commit |
