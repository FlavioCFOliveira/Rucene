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
told apart from the Java ones by a `file` that starts with `src/`.

### `Component`
A named unit of the local crate registered before the `Rust*` labels existed
(mostly `src/util.rs` and `src/store.rs` items). Identity is `name`. New work
uses `RustStruct`/`RustTrait`/`RustEnum` for anything that declares a Rust type,
and keeps `Component` for a module of free functions that declares none — the
ports of Lucene's static utility classes, such as `ArrayUtil`, `BitUtil`,
`IOUtils`, `NumericUtils` and `VectorUtil`.

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
always use `hash`.

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
| `DECLARES` | `File` → `Class`/`Interface`/`Enum`/`Exception`/`Annotation`/`Method`. |
| `NESTED_IN` | `Class` (inner type) → `Class` (enclosing top-level type). |
| `DEPENDS_ON` | `Package` → `Package`, `Class` → `Class`, `Module` → `Module`. |
| `EXTENDS` | `Class` → `Class` / `Class` → `Interface`. |
| `IMPLEMENTS` | `Class` → `Interface`. |
| `EXPORTS` | `Feature` (`module-info`) → `Package` (JPMS exported package). |
| `OPENS` | `Feature` (`module-info`) → `Package` (JPMS opened package). |
| `REQUIRES` | `Feature` (`module-info`) → `Feature` (required module). |
| `USES` | `Feature` (`module-info`) → `Class` (SPI service interface). |
| `PROVIDES` | `Feature` (`module-info`) → `Class` (SPI service interface). |
| `PROVIDED_BY` | `Class` (SPI interface) → `Class` (implementation). |
| `TESTS` | `File` / `Class` → `Feature` / `Class` / `RustStruct`/`RustTrait`/`RustEnum`. A portability test file points at the harness `Feature` it belongs to and at the Rucene types whose behaviour it pins down. |
| `SPECIFIED_IN` | `Feature` → `File` (specification document). |
| `REFERENCES` | `Project` → `Project` (Rucene references Apache Lucene Core 10.5.0). |
| `PORTS` | Rucene type (`RustStruct`/`RustTrait`/`RustEnum`/`Component`) → Lucene `Class`/`Interface`/`Enum`. The Rust type is the port of that Lucene type. Optional `note` property records that the port is partial, a placeholder, or a deliberate adaptation, and says what is missing or what was changed and why. |
| `IMPLEMENTED_IN` | `Component`/`Task` → `File`/`Module`/`Commit` (where the thing lives or landed). |
| `IMPLEMENTS` | Also used as `Feature` → `File`/`Class`: the file or type that realises the feature. |
| `COMMITTED_IN` | `File`/`Feature`/`Component` → `Commit`. |
| `CLOSES_TASK` | `Commit` → `Task`. |
| `TESTED_BY` / `MODIFIES` / `FULFILLS` / `DELIVERS` | Legacy provenance edges from the first syncs; not used by new work. |

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
| `Feature` | target — to be created as needed for Rucene features |
| `CONTAINS` | populated (project→module, module→package, package→file) |
| `DECLARES` | populated (file→top-level type) |
| `DEPENDS_ON` | populated (package→package dependencies derived from imports) |
| `EXTENDS` / `IMPLEMENTS` | populated (type→type relationships) |
| `REFERENCES` | populated (Rucene → Apache Lucene Core 10.5.0) |
| `TESTS` / `SPECIFIED_IN` | populated for the portability harness and the components it validates; extended per synced commit |
| `RustStruct` / `RustTrait` / `RustEnum` | populated incrementally, one sync per commit that ports types |
| `Task` / `Commit` | populated for the commits that have been synced; not a complete history |
| `PORTS` | populated for every ported Rucene type whose Lucene counterpart is already modelled |
| `IMPLEMENTED_IN` / `IMPLEMENTS` / `COMMITTED_IN` / `CLOSES_TASK` | populated per synced commit |
