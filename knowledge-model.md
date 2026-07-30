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

### `Method` *(target tier — populated selectively for key public/protected APIs)*
A method, constructor, or function.

| Property | Purpose |
|----------|---------|
| `name` | Method name. |
| `signature` | Method signature (simplified). |
| `kind` | `"method"`, `"constructor"`, `"static"`, `"function"`. |
| `file` | Source file path or URL. |
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
| `DEPENDS_ON` | `Package` → `Package`, `Class` → `Class`, `Module` → `Module`. |
| `EXTENDS` | `Class` → `Class` / `Class` → `Interface`. |
| `IMPLEMENTS` | `Class` → `Interface`. |
| `EXPORTS` | `Feature` (`module-info`) → `Package` (JPMS exported package). |
| `OPENS` | `Feature` (`module-info`) → `Package` (JPMS opened package). |
| `REQUIRES` | `Feature` (`module-info`) → `Feature` (required module). |
| `USES` | `Feature` (`module-info`) → `Class` (SPI service interface). |
| `PROVIDES` | `Feature` (`module-info`) → `Class` (SPI service interface). |
| `PROVIDED_BY` | `Class` (SPI interface) → `Class` (implementation). |
| `TESTS` | `File` / `Class` → `Feature` / `Class`. |
| `SPECIFIED_IN` | `Feature` → `File` (specification document). |
| `REFERENCES` | `Project` → `Project` (Rucene references Apache Lucene Core 10.5.0). |

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
| `TESTS` / `SPECIFIED_IN` | target — to be populated as specifications and tests are authored |
