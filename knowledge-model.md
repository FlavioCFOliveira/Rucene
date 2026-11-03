# Knowledge Graph Model — rucene

The knowledge graph of the **Rucene** project (roadmap `rucene`, reached through
`rmp graph client`). It represents:

1. the **Rucene** crate: its files, types and functions, and the references between
   them;
2. the reference **Apache Lucene Core 10.5.0**: the 35 modules of the distribution,
   and `lucene/core` in depth — packages, files, types, members, dependencies,
   bytecode calls and its test surface;
3. **the port between them**: which Lucene type each Rust item ports and on what
   evidence, how much of each type exists, which members are absent, the declared
   divergences, the verdicts on items no production code references, and the `rmp`
   tasks and commits.

**This file holds the graph's form, never its content.** It defines the labels, their
properties and identity, the predicates, the constraints and indexes, and the queries
that answer the project's questions. It states no count and no finding: every such
fact is one statement away, and the entry names the statement instead.

Two rules make the graph an answer rather than an impression (`CLAUDE.md` §7, §8):

* **Every claim states its basis.** A `PORTS` edge carries `evidence`; a depth figure
  carries `memberEvidence`; a scope decision carries `portScopeRule`; a hand
  correction carries `portStateNote`; a member verdict carries its rule.
* **The graph is checked against the code, not against its own loaders.**
  `tools/kg/audit_kg.py` re-derives the census from the Lucene clone, the compiler's
  view of the crate, `git` and `rmp`; `problems: 0` is the pass condition.

---

## Using the graph during development

* **Keep it current.** After every commit, `tools/kg/sync_commit.sh <commit>` surveys
  the commit from a `git archive` export (the working tree may hold uncommitted
  work), runs the whole chain (*Reproducing the graph*), and ends with the audit. The
  log, `<kg-target>/kg_sync_<short>.log`, times every step.
* **Ask the graph first.** *Answering the standing questions* holds the statements
  for the questions the work asks: what is missing, what to do next, what a change
  touches, which divergences an edit must respect, what remains of task #269.
* **Read a property's maintainer before trusting it.** Each label below says which
  tool writes it. A label marked *curated* is written by hand when its fact arises; no
  sync step completes it, so its absence proves nothing.

---

## Node labels

Every node and edge carries the provenance stamp `gitCommit` (full 40-char hash) and
`gitDate` (`YYYY-MM-DD`): the commit at which the fact was last confirmed (see
*Provenance convention*). The tables below omit it.

### `Project`
A software project or library being modelled. Identity: `name` (string).

| Property | Purpose |
|---|---|
| `name` | `"Rucene"` or `"Apache Lucene Core 10.5.0"`. |
| `language` | Primary language: `"Rust"`, `"Java"`. |
| `version` | Version string, e.g. `"10.5.0"`. |
| `path` | Root path, for the local project. |
| `referenceUrl` | Authoritative source URL for an external project. |

### `Module`
A crate or Maven module: `rucene`, and every module of the Lucene 10.5.0 distribution
that ships Java sources, so the port's denominator is explicit (`CLAUDE.md` §6.1).
Identity: `name` (string). Written by `tools/kg/lucene_modules_kg.py`.

| Property | Purpose |
|---|---|
| `name` | `"rucene"`, `"lucene/core"`, `"lucene/analysis/common"`, … |
| `shortName` | The path under `lucene/`, e.g. `"core"`. |
| `kind` | `"crate"`, `"maven-module"`. |
| `path` | Root path in the source tree. |
| `project` | The owning `Project.name`. |
| `javaFiles` | Java source files the module declares at the surveyed tag. |
| `inScope` | `true` for a module the port covers. |
| `scopeRule` | The rule that set `inScope`: `"lucene-core-is-the-port-target"`, `"outside-lucene-core"`. |
| `role` | What the module is, in one line. |

### `Package`
A Java package. Identity: `name` (string).

| Property | Purpose |
|---|---|
| `name` | Fully qualified package name, e.g. `"org.apache.lucene.index"`. |
| `shortName` | Last segment. |
| `path` | Directory path. |

### `Class`
A Java type of `lucene/core` — class, interface, enum, record, exception or annotation;
`kind` tells them apart. (`Interface`, `Enum`, `Exception` and `Annotation` are reserved
labels that no loader writes.) Identity: `qualifiedName` (string); a nested type is
keyed `pkg.Outer$Inner`. Written by the Lucene survey (`extract_lucene_kg.py`,
`enrich_lucene_kg.py`, `lucene_structure_kg.py`); the port properties by the port
loaders named below.

| Property | Purpose |
|---|---|
| `name` | Simple type name. |
| `qualifiedName` | Fully qualified name (identity). |
| `kind` | `"class"`, `"interface"`, `"enum"`, `"record"`, `"exception"`, `"annotation"`. |
| `package` | Enclosing Java package. |
| `file` | Source file, relative to the Lucene repository root. |
| `extendsExternal` / `implementsExternal` | External (`java.*`, `javax.*`) super-types not modelled as nodes. |
| `portScope` | `"in"`, `"nested"`, `"out"` — see below. |
| `portScopeRule` | The mechanical rule that assigned `portScope`. |
| `portState` | Top-level: `"ported"`, `"candidate"`, `"unported"`. Nested: `"ported"`, `"absorbed"`, `"unported"`. See below. |
| `portStateEvidence` | On a nested type, the basis of its `portState`: `"doc-comment"`, `"curated"`, `"not-nameable"`, `"verified-absorption"`, `"verified-absent"`. |
| `portStateNote` | Why a hand verification overrode the mechanical `portState`. A type carrying it is skipped by the mechanical pass, so the verification survives the next sync. |
| `javaVisibility` | Top-level: `"public"` or `"package"`, from the class file's `ACC_PUBLIC` flag (`lucene_calls_kg.py`). Nested: the effective visibility. |
| `javaPublished` / `javaUsedAcrossFiles` | On a nested type: whether it and every enclosing type are `public`/`protected`; whether another `lucene/core` file names it as `Outer.Inner`. |
| `nestedHost` / `nestedHostFiles` | On a nested type: the nearest enclosing type that has a port, and the crate files that port it. |
| `portDestination` | On an `unported` nested type: the crate file its port belongs in. |
| `memberTotal` / `memberMatched` / `memberCoverage` | The depth floor: Java method names the type declares, those whose name maps to a function the port provides, and the ratio (`port_evidence_kg.py --phase depth`). |
| `memberEvidence` | Always `"name-mapping"`: the basis of the floor. |
| `memberAbsent` / `memberRenamed` / `memberNa` | The triage of the names the floor missed: really absent, present under another name, not applicable (`triage_load.py`). **`memberAbsent` is the number to plan from.** A measured type with nothing unmatched carries zeros, never null. |
| `memberTriage` | `"rule-ladder"` once the triage has classified the type. |

#### `portScope`

| Value | `portScopeRule` | Meaning |
|---|---|---|
| `in` | `lucene-core-top-level` | A top-level type declared by a file of `lucene/core` (`src/java`, `src/java21`). The port target. |
| `nested` | `nested-in-enclosing-type` | An inner type; not an independent port unit. Each carries its own measured `portState`. |
| `out` | `not-a-lucene-core-type` | Anything else carrying the label, such as Rucene's own Java test fixtures. |

The rule keys on the **declaring file**, not on the package name.

#### `portState`

| Value | Meaning |
|---|---|
| `ported` | A `PORTS` edge asserts that a Rucene item is the port of this type; its `evidence` says on what basis. |
| `candidate` | No assertion, but exactly one Rucene type carries the same simple name, recorded as `PORTS_CANDIDATE`. Probable, unverified. |
| `absorbed` | (nested only) No independent Rust item, and none is owed, on the evidence `portStateEvidence` names. |
| `unported` | Neither. Real work; a nested one carries `portDestination`. |

A nested type's `portStateEvidence`: `doc-comment` (the crate's own claim);
`curated` (a hand-verified edge in `tools/kg/nested_port_verdicts.json`, with file and
line); `not-nameable` (no client can name it — not `public`/`protected` through every
enclosing type, and no other file names it — so nothing depends on it except through
its host; its behaviour is measured by the host's depth, not per type);
`verified-absorption` (nameable, and the crate shows where its work went);
`verified-absent` (read and found missing, with destination).

#### Depth

`memberMatched` counts a Java method as present when its name maps (`fooBar` →
`foo_bar`; `getX`/`isX`/`hasX`/`setX` → `x`; a constructor → `new`) to a function of
the port's inherent `impl`s, of its file's free functions, or of a trait it
implements. It is a **floor**: a rename, a split over several items, a `#[derive]` or a
supertype's port reads as a miss. `tools/kg/triage_members.py` classifies every miss
with a rule ladder — tiers `own` (the porting type, its traits), `super` (the port of a
Java supertype), `file` (the porting files, the `X/` submodule, the sibling
`macros.rs`); within a tier `exact` → `accessor-prefix` → `separator` → `doc-comment`
→ `name-extension`; and `contract-trait` / `derive` for `equals`, `hashCode`,
`toString`, `clone`, `compareTo`, `iterator`, from the type's own impls only. Anything no
rule settles is `absent` — an upper bound on real absence. `na` has three grounds: a
`#[derive]` discharging an `Object` contract; a `private` Java method on a type whose
reachable surface has no absence; a member `tools/kg/port_gaps_exclusions.json` marks
`"triageClass": "na"`.

### `Method`
A member of a `lucene/core` type: method, constructor or field. Identity:
`qualifiedName` (string, `<class>#<signature>`).

| Property | Purpose |
|---|---|
| `name` | Member name. |
| `qualifiedName` | `<class>#<signature>` (identity). |
| `signature` | Return type, name and parameter list. |
| `kind` | `"method"`, `"constructor"`, `"field"`, `"enumConstant"`. |
| `modifiers` | Access and other modifiers. |
| `returnType` | Return or field type. |
| `parentQualifiedName` | The declaring class. |
| `overridesExternal` | `"<jdk type>#<name>(<params>)"` strings: JDK members it overrides, from the bytecode (`lucene_calls_kg.py`). |
| `portTriage` | The triage verdict of the member's name: `"absent"`, `"renamed"`, `"na"`. |
| `portTriageRule` / `portTriageEvidence` | The rule that decided it and its evidence. |
| `portTriageUnit` | `<class>#<method name>`: the triage's unit. Every overload of the name carries the name's verdict, so **sum member verdicts over distinct `portTriageUnit`**, which equals the class rollup. |

`triage_load.py` clears every member verdict before it writes the current run's, so
no older verdict survives.

### `File`
A source, build, documentation or configuration file of either side. Identity: `path`
(string, relative to its repository root).

| Property | Purpose |
|---|---|
| `path` | Path (identity). |
| `name` | File name. |
| `kind` | `"source"`, `"test"`, `"test-fixture"`, `"build"`, `"doc"`, `"config"`, `"module-descriptor"`. A `"test-fixture"` is a Java program under `tests/fixtures/` that drives Lucene 10.5.0 to emit reference data. |
| `language` | `"Rust"` for a crate `.rs` file; `"Java"` for a fixture; absent otherwise. |
| `modulePath` | For a crate file, its Rust module path or test-crate name. |
| `crate` | `"rucene"`, `"java-codec-harness"`, or the `[[test]]` crate name. |
| `loc` | Line count at the surveyed commit. |
| `package` | For a Java file, its package. |
| `role` / `moduleName` | For `module-info.java`, the JPMS module name. |

### `RustStruct`, `RustTrait`, `RustEnum`, `RustAlias`
A type declared by the crate; `RustAlias` is a module-level `type X = …;`. Identity:
`{name, file}` (strings), so same-named types of different modules stay distinct.
Written by `extract_rucene_kg.py` → `load_rucene_kg.py`, which also deletes the nodes a
survey no longer confirms.

| Property | Purpose |
|---|---|
| `name` | Type name (identity, with `file`). |
| `file` | Declaring file (identity, with `name`). |
| `qualifiedName` | `rucene::<module path>::<Name>`, or `<test crate>::…`. |
| `kind` | `"struct"`, `"trait"`, `"enum"`, `"union"`, `"alias"`. |
| `visibility` | `"pub"` (including `pub(crate)`) or `"private"`. |
| `scope` | `"crate"`, or `"test"` for an item gated by `#[cfg(test)]` on its module or itself. |
| `origin` | `"source"`, or `"macro"` for a type that exists only after macro expansion (`extract_rucene_kg.py --expand` reads `cargo +nightly rustc -- -Zunpretty=expanded`). Its `file` is the macro's invocation site. |
| `language` | `"Rust"`. |
| `noCallerVerdict` … | The #269 verdict, as on `RustFn` below. |

Types declared inside a function body, and `struct` templates inside a `macro_rules!`
body, are not modelled.

### `RustFn`
A function of the crate: a free function, a method of an inherent `impl`, a method
declared in a `trait`, or a test. The bodies of `impl Trait for Type` methods are not
modelled (the `IMPLEMENTS` edge and the trait's declaration cover them), nor are
functions nested in another function. Identity: `qualifiedName` (string).

| Property | Purpose |
|---|---|
| `qualifiedName` | `rucene::<module path>[::<Owner>]::<name>` (identity). |
| `name` | Function name. |
| `file` | Declaring file. |
| `owner` | The inherent `impl` type or the trait that declares it; absent for a free function. |
| `kind` | `"function"`, `"method"`, `"trait-method"`, `"test"`. |
| `visibility` | `"pub"` or `"private"`. |
| `scope` | `"crate"` or `"test"`. |
| `signature` | The declaration line, whitespace-normalised. |
| `origin` | `"source"` or `"macro"`. |
| `language` | `"Rust"`. |
| `noCallerVerdict` | Task #269's verdict on a `pub` item no production code references: `"W"` (a Lucene link was missing and is now wired), `"W="` (the link exists another way), `"W-blocked"` (the Java caller is not ported), `"P"` (public API kept), `"D"` (removed: no Lucene counterpart), `"T"` (test-only). Written by `no_caller_verdicts_kg.py` from `tools/kg/no_caller_verdicts/block*.md`. |
| `noCallerAction` / `noCallerEvidence` | What was done, and the Java evidence (file:line). |
| `noCallerTable` / `noCallerCommit` | The block table that holds the verdict, and the commit it was loaded at. |

### `Component`
A crate unit that declares no Rust type: a module of free functions porting one of
Lucene's static utility classes, or the port of a Lucene class with no members.
Identity: `name` (string). Written by `port_evidence_kg.py`: a module-level `//!`
claim naming `pkg.Foo`, in a file that declares no type, lands on that file's
`Component`.

| Property | Purpose |
|---|---|
| `name` | Unit name (identity). |
| `kind` | `"module"`. |
| `file` | The declaring file. |
| `status` | `"ported"` (carries a `PORTS` edge), `"present"` (no `PORTS` yet), `"stale"` (no file confirmed at the surveyed commit). |
| `language` | `"Rust"`. |

### `Divergence`
A divergence from Lucene 10.5.0 that the crate **declares** (`CLAUDE.md` §14.5): one
doc-comment paragraph led by `**Divergence…**` or `**Divergences…**`. Identity: `id`
(string, `"<file>:<line>"` of the lead line). Written whole by
`tools/kg/divergences_kg.py`, which deletes the nodes the code no longer declares.

| Property | Purpose |
|---|---|
| `id` | `"<file>:<line>"` (identity). |
| `file` / `line` | Where the paragraph is. |
| `title` | The bold lead, e.g. `"Divergence from Lucene 10.5.0"`. |
| `text` | The paragraph, whitespace-joined (truncated at 1500 characters). |
| `itemKind` | What the doc comment documents: `"fn"`, `"struct"`, `"enum"`, `"trait"`, `"type"`, `"impl"`, `"member"` (field or variant), `"const"`, `"static"`, `"mod"`, `"macro"`, `"module"` (a `//!` block), `"unknown"`. |
| `item` | The documented declaration's name; absent for a module. |
| `javaRefs` | The `File.java[:lines]` citations of the paragraph, `"; "`-separated. |

### `Task`
An `rmp` task: every task `rmp` knows, open or closed, and nothing else. Identity:
`id` (**integer**). Written by `port_coverage_kg.py` (the `tasks` phase), which also
deletes a node whose task `rmp` no longer has, unless a commit closes it.

| Property | Purpose |
|---|---|
| `id` | The `rmp` task number (identity, integer). |
| `name` | Task title. |
| `status` | `BACKLOG`, `SPRINT`, `DOING`, `TESTING`, `COMPLETED`. |
| `priority` | `rmp` priority (string, `"0"`–`"9"`). |
| `components` | Comma-separated Rust paths delivered by a closed task (legacy). |

### `Commit`
A commit of the local repository; every commit is mirrored. Identity: `hash` (string,
full 40-char). Written by `commits_kg.py`.

| Property | Purpose |
|---|---|
| `hash` | Full hash (identity). |
| `message` | Subject line. |
| `date` | Author date, ISO 8601 with offset. |
| `task_id` | The `rmp` task the commit closes, when there is one. |

### `Decision`
*Curated.* An engineering decision that constrains the code — a dependency, an
algorithm, a format, an adaptation, a project-wide principle, a declared gap — recorded
so it can be audited and revisited (`CLAUDE.md` §13.2). Identity: `name` (string).
A divergence the code declares is a `Divergence`, not a `Decision`.

| Property | Purpose |
|---|---|
| `name` | Short title (identity). |
| `kind` | `"dependency"`, `"algorithm"`, `"format"`, `"adaptation"`, `"principle"`, `"gap"`. |
| `summary` / `rationale` / `alternatives` / `evidence` | What was decided, why, what was rejected, and what settles it. |

A `"gap"` decision records a part of a task's acceptance criteria that could not be
verified end to end, and names the follow-up through `DEPENDS_ON` → `Task`.

### `Defect`
*Curated.* A non-obvious bug found and fixed — a divergence only a portability or
fuzz test could catch, or a hazard reachable from a corrupt file. Identity: `name`
(string).

| Property | Purpose |
|---|---|
| `name` | Short title (identity). |
| `kind` | `"portability"` or `"robustness"`. |
| `summary` / `cause` / `fix` | What was wrong, why, and what changed (`file:line`). |
| `luceneReference` | The Lucene 10.5.0 file and lines that define the correct behaviour (required for `"portability"`). |
| `foundBy` | The test that exposed it. |

### `Feature`
*Curated.* A functional capability, linking files and types to what they implement,
and the JPMS `module-info` descriptors. Identity: `name` (string). Properties: `name`,
`description`.

### `TestClass`
A top-level type of a Lucene 10.5.0 test tree — `lucene/test-framework/src/java` or
`lucene/core/src/test`. Identity: `qualifiedName` (string). Written by
`lucene_tests_kg.py`.

| Property | Purpose |
|---|---|
| `qualifiedName` | Fully qualified name (identity). |
| `name` / `kind` / `package` / `file` | As for `Class`. |
| `module` | `"lucene/test-framework"` or `"lucene/core"`. |
| `testKind` | `"framework"` or `"unit-test"`. |
| `role` | From name and path only: `unit-test`, `framework-util`, `base-test-case`, `mock`, `asserting`, `mock-filesystem`, `test-codec`, `test-case-base`, `test-helper`, `fault-injection`, `annotation`. |
| `isAbstract` | Whether the declaration is `abstract`. |
| `testMethodCount` | Test methods the file declares. |
| `ruceneCoverage` | `"covered"` (the subject is ported and its Rust file declares a `#[test]`), `"uncovered"` (ported, no test there), `"subject-unported"`, `"no-subject-resolved"` (the name follows no test-naming convention). A file-level correspondence: it never means the Lucene test's cases were ported. |

### `TestMethod`
A test method of a `TestClass`: `public void testXxx()` or a `void` method carrying
`@Test`, attributed to the first top-level type of its file. Identity: `qualifiedName`
(string, `<class>#<method>`). Properties: `qualifiedName`, `name`,
`parentQualifiedName`, `file`, `module`.

---

## Edge types

| Edge | Meaning |
|---|---|
| `CONTAINS` | `Project` → `Module`, `Module` → `Package` / `File`, `Package` → `Package` / `Class` / `File`. |
| `DECLARES` | `File` → `Class`; `Class` → `Method`; crate `File` → `RustStruct`/`RustTrait`/`RustEnum`/`RustAlias`/`RustFn`; Rust type → `RustFn`; `TestClass` → `TestMethod`. |
| `NESTED_IN` | Inner `Class` → its enclosing top-level `Class`. |
| `DEPENDS_ON` | Lucene `Package` → `Package` and `Class` → `Class` (imports plus same-package references); crate `File` → `File` (`use`); curated Rust type → type (`note`); `Task` → `Task` (as `rmp` records); `Decision` → `Task` (a gap's follow-up). |
| `EXTENDS` | `Class` → `Class`; `TestClass` → `TestClass` (how a test reaches `LuceneTestCase` or a `Base…TestCase`). |
| `IMPLEMENTS` | Lucene `Class` → interface `Class`; Rust type → `RustTrait` (`impl Trait for Type`, crate traits only); legacy `Feature` → `File`/`Class`, and the reverse in a few early edges. |
| `PORTS` | Rucene item (`RustStruct`/`RustTrait`/`RustEnum`/`RustAlias`/`RustFn`/`Component`) → Lucene `Class`. **`evidence` is mandatory**: `"doc-comment"` (the item's doc says `Equivalent to \`org.apache.lucene.X\`` or a lead-in of that family, or the module's `\| Rucene \| Apache Lucene Core 10.5.0 \|` table names it) or `"curated"`. `declaredAt` gives `file:line`, or the file alone with `declarationSource: "macro-expansion"`. `claimLevel: "member"` and `declaredMember` mark a claim naming a member. Optional `note`: a partial port or adaptation. |
| `PORTS_CANDIDATE` | Rust type → `Class`: exactly one Rust type has the same simple name; `evidence: "exact-name-match"`. Unverified; superseded by a `PORTS`. |
| `PORTS_MEMBER` | `RustFn` → `Method`: by name, the fn ports that member. `evidence: "name-mapping"`; `via`: `"declared"`, `"owner"`, `"inherited"`, `"free-fn"`. Written whole by `no_caller_kg.py --phase members`. |
| `REQUIRES_PORT` | `Task` → `Class`: the task's text names the type (an unambiguous simple name of at least four characters). |
| `CALLS` | Lucene `Method`/`Class` → `Method`, from the bytecode of `lucene-core-10.5.0.jar`. `kind`: `"invoke"`, `"field"`; `sites`; `resolution`: `"exact"`, `"synthetic"`, `"arity"`, `"name"`, `"ambiguous"`; `evidence: "bytecode"`. Written whole by `lucene_calls_kg.py`. |
| `OVERRIDES` | `Method` → `Method`, transitively: the source overrides or implements the target. `evidence: "bytecode"`. |
| `REFERENCES` (crate) | Rust item or `File` → Rust item: the source's code names the target. `evidence`: `"rust-analyzer-scip"` (resolved by rust-analyzer) or `"macro-body-lexical"` (a `macro_rules!` body names it; weaker). `prodRefs` / `testRefs`: occurrences at production and test sites. Written whole by `rust_refs_kg.py`. |
| `REFERENCES` | `Project` → `Project` / `Feature`; `Feature` → `Package`. Not a port relation. |
| `AWAITS_CALLER` | Rust item → Lucene `Class`: a `W-blocked` #269 item, and a Java class that calls its counterpart and is not ported. Written whole by `no_caller_verdicts_kg.py` from the commit's no-caller list. |
| `DECLARED_IN` | `Divergence` → `File` that declares it. |
| `CONCERNS` | `Divergence` → the Rust type or `RustFn` its doc comment documents, when the declaration resolves to exactly one node (a method of an `impl Trait for Type` block resolves to none). |
| `DIVERGES_FROM` | `Divergence` → a Lucene `Class` whose source file the paragraph cites, when one top-level type is declared by a file of that name; `lines` holds the cited lines. The Lucene type of a divergence is also reached through `CONCERNS` → `PORTS`. |
| `TESTS` | `File`/`Class` → `Feature`/`Class`/Rust type/`Component`/`Defect`: a test file and what it pins down; `TestClass` → `Class` with `evidence: "name-convention"` (`TestFoo` → `Foo`). |
| `COVERED_BY` | `TestClass` → `File`: the Rust file porting the test's subject, which declares a `#[test]`; `evidence: "ported-type-file-has-tests"`. |
| `EXPORTS` / `OPENS` / `REQUIRES` / `USES` / `PROVIDES` | `Feature` (`module-info`) → `Package`/`Feature`/`Class`. |
| `PROVIDED_BY` | SPI interface `Class` → implementation `Class`. |
| `SPECIFIED_IN` | `Feature` → specification `File`. |
| `IMPLEMENTED_IN` | `Component`/`Task`/`Decision`/`Defect` → `File`/`Commit`. |
| `COMMITTED_IN` | `File`/`Feature`/`Component`/`Decision`/`Defect` → `Commit`; for every file a commit changed (`commits_kg.py`). |
| `CLOSES_TASK` | `Commit` → `Task`: the commit that closed the task, as `rmp` decides (`rmp_closings.py` export); for a task the export does not list, a `Closes`/`Fixes` footer, the first body line's tag, a `Task:` trailer or a subject tag. |
| `REFERENCES_TASK` | `Commit` → `Task`: a `Refs #N` footer. One task edge per (`Commit`, `Task`) pair. |
| `DELIVERS` | `Task` → `Feature`. |
| `TESTED_BY` / `MODIFIES` / `FULFILLS` / `IMPLEMENTED_BY` | Legacy; not written by new work. |

---

## Answering the standing questions

### Is the graph current?

```cypher
MATCH (c:Commit) RETURN c.hash ORDER BY c.date DESC LIMIT 1
```

Compare with `git rev-parse HEAD`; if they differ, run `tools/kg/sync_commit.sh HEAD`.

### What is ported, and what is missing

```cypher
MATCH (c:Class) WHERE c.portScope IN ['in', 'nested']
RETURN c.portScope AS scope, c.portState AS state, count(c) AS types
ORDER BY scope, types DESC
```

What a port claim rests on, and the notes that record a partial port:

```cypher
MATCH (t)-[e:PORTS]->(c:Class {qualifiedName: $type})
RETURN labels(t)[0] AS label, t.name, t.file, e.evidence, e.declaredAt, e.note
```

### How much of each ported type exists — the member work

Plan from `memberAbsent`; audit with `memberMatched`. By package:

```cypher
MATCH (c:Class)
WHERE c.portScope = 'in' AND c.memberTriage IS NOT NULL
RETURN c.package AS package, count(c) AS types,
       sum(c.memberTotal) AS members,
       sum(c.memberTotal) - sum(c.memberMatched) AS unmatched,
       sum(c.memberAbsent) AS absent, sum(c.memberRenamed) AS renamed,
       sum(c.memberNa) AS notApplicable
ORDER BY absent DESC
```

The types to work on, and the members each lacks:

```cypher
MATCH (c:Class) WHERE c.portScope = 'in' AND c.memberAbsent > 0
OPTIONAL MATCH (c)-[:DECLARES]->(m:Method) WHERE m.portTriage = 'absent'
RETURN c.qualifiedName AS type, c.memberAbsent AS absent,
       collect(DISTINCT m.name) AS members
ORDER BY absent DESC, type
```

A member verdict and its rule (`portTriageRule`, `portTriageEvidence`); to sum member
verdicts, count distinct units:

```cypher
MATCH (m:Method) WHERE m.portTriage IS NOT NULL
RETURN m.portTriage AS verdict, count(DISTINCT m.portTriageUnit) AS units
```

### What is in scope

```cypher
MATCH (m:Module) WHERE m.project = 'Apache Lucene Core 10.5.0'
RETURN m.inScope AS inScope, count(m) AS modules, sum(m.javaFiles) AS javaFiles
```

### What to do next

An unported in-scope type, ranked by the tasks and types it blocks:

```cypher
MATCH (u:Class) WHERE u.portScope = 'in' AND u.portState <> 'ported'
OPTIONAL MATCH (t:Task)-[:REQUIRES_PORT]->(u) WHERE t.status <> 'COMPLETED'
OPTIONAL MATCH (d:Class)-[:DEPENDS_ON]->(u)
RETURN u.qualifiedName AS unported, count(DISTINCT t) AS blockedTasks,
       count(DISTINCT d) AS blockedTypes
ORDER BY blockedTasks + blockedTypes DESC LIMIT 15
```

Absent members, ranked by how many Lucene types depend on their type:

```cypher
MATCH (c:Class) WHERE c.portScope = 'in' AND c.memberAbsent > 0
OPTIONAL MATCH (d:Class)-[:DEPENDS_ON]->(c)
RETURN c.qualifiedName AS type, c.memberAbsent AS absent, count(DISTINCT d) AS dependants
ORDER BY dependants DESC, absent DESC LIMIT 15
```

Open tasks and what blocks them:

```cypher
MATCH (t:Task) WHERE t.status IN ['SPRINT', 'DOING', 'TESTING']
OPTIONAL MATCH (t)-[:DEPENDS_ON]->(b:Task) WHERE b.status <> 'COMPLETED'
RETURN t.id, t.status, t.name, collect(b.id) AS blockedBy ORDER BY t.id
```

### What a change touches

Where a Rust item lives, and what its file depends on:

```cypher
MATCH (f:File)-[:DECLARES]->(t) WHERE t.name = $name
RETURN f.path, labels(t)[0] AS label, t.qualifiedName
```

```cypher
MATCH (:File {path: $path})-[:DEPENDS_ON]->(b:File) RETURN b.path
```

Who references an item (production and test sites):

```cypher
MATCH (r)-[e:REFERENCES]->(x:RustFn {qualifiedName: $fn})
WHERE e.evidence = 'rust-analyzer-scip'
RETURN labels(r)[0] AS label, coalesce(r.qualifiedName, r.path) AS referrer,
       e.prodRefs, e.testRefs
```

The Lucene members a Rust fn ports, and their Java callers:

```cypher
MATCH (:RustFn {qualifiedName: $fn})-[:PORTS_MEMBER]->(m:Method)
OPTIONAL MATCH (m)<-[:CALLS]-(c)
RETURN m.qualifiedName, collect(DISTINCT coalesce(c.parentQualifiedName, c.qualifiedName)) AS callers
```

### Which divergences an edit must respect

Before editing a file, the divergences it declares (`CLAUDE.md` §14.5: existing
divergences are debt to re-evaluate):

```cypher
MATCH (d:Divergence)-[:DECLARED_IN]->(:File {path: $path})
RETURN d.line, d.itemKind, d.item, d.title, d.text ORDER BY d.line
```

The divergences of a Lucene type — on its port, or citing its source:

```cypher
MATCH (c:Class {qualifiedName: $type})
OPTIONAL MATCH (d1:Divergence)-[:CONCERNS]->()-[:PORTS]->(c)
OPTIONAL MATCH (d2:Divergence)-[:DIVERGES_FROM]->(c)
WITH collect(DISTINCT d1) + collect(DISTINCT d2) AS ds
UNWIND ds AS d
RETURN DISTINCT d.id, d.item, d.text
```

By subsystem (the crate's top-level module):

```cypher
MATCH (d:Divergence)
RETURN split(d.file, '/')[1] AS subsystem, count(d) AS divergences
ORDER BY divergences DESC
```

### Who reaches a ported item — `pub` items no production code references

Which `pub` items of the crate no production code references, what each one's Lucene
counterpart is, and who calls that counterpart in Lucene 10.5.0. It joins four edge
sets, each written whole by its tool: `REFERENCES` (crate), `CALLS` and `OVERRIDES`,
and `PORTS_MEMBER`.

* **`pub` item** — a `RustFn` or Rust type with `visibility = "pub"`, `scope =
  "crate"`, `file` under `src/`, not a test.
* **No production reference** — no incoming `REFERENCES` with `evidence =
  "rust-analyzer-scip"` and `prodRefs > 0`; for a type, a reference from a fn the
  type `DECLARES` does not count. A `macro-body-lexical` edge is reported
  (`macroBodyMentions`), never counted.
* **Counterpart** — for a fn, the `Method`s it `PORTS_MEMBER` plus the `Class` it
  `PORTS`; for a type, the `Class` it `PORTS`.
* **Lucene callers** — of a member, the sources of `CALLS` into it or into a
  declaration it `OVERRIDES`; of a type, every other `Class` that `DEPENDS_ON` it.
* **Triage class** — `W?`: the counterpart has Lucene callers. `P?`: no caller, and
  the counterpart is public API. `D?`: no counterpart, or one neither called nor API.
  A `W?` is *blocked* when none of its callers' classes is `ported` or `absorbed`.

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

Types — the same shape over the `PORTS` class and its `DEPENDS_ON` dependants; run it
once per label, with `{label}` replaced by `RustStruct`, `RustEnum`, `RustTrait` and
`RustAlias`:

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

By subsystem and class: replace the final `RETURN` of either query by `WITH` and append
`WITH subsystem, class, count(*) AS items RETURN subsystem, class, items ORDER BY
subsystem, class`. `no_caller_kg.py --phase report` runs exactly these statements and
writes every row to `<kg-target>/no_caller/list_<commit>.jsonl`.

### What remains of task #269

The verdicts recorded so far:

```cypher
MATCH (n) WHERE (n:RustFn OR n:RustStruct OR n:RustTrait OR n:RustEnum OR n:RustAlias)
  AND n.noCallerVerdict IS NOT NULL
RETURN split(n.qualifiedName, '::')[1] AS subsystem, n.noCallerVerdict AS verdict, count(*) AS items
ORDER BY subsystem, verdict
```

The items of the no-caller list still without a verdict: run the *Functions* query
above, replacing its `RETURN …` by

```cypher
WITH x WHERE x.noCallerVerdict IS NULL
RETURN split(x.qualifiedName, '::')[1] AS subsystem, count(*) AS unverified
ORDER BY subsystem
```

(and the same for each type label). The `W-blocked` items and the unported Java
callers they wait for:

```cypher
MATCH (n)-[:AWAITS_CALLER]->(c:Class)
RETURN c.qualifiedName AS javaCaller, c.portState AS state,
       collect(n.qualifiedName) AS waiting
ORDER BY size(waiting) DESC
```

---

## Indexes and constraints

Every index this engine can use (rmp 1.18.0) is a hash index on one node property, so
it serves **equality lookups only**. Each label carries one on its identity property,
because the sync tools and ad hoc queries bind nodes by identity in
`MATCH (n:L {k: …})` and `MERGE (n:L {k: …})`; without it a `MERGE` walks the whole
label for every row of a batch.

| Index | Label | Property |
|---|---|---|
| `idx_class_qualifiedname` | `Class` | `qualifiedName` |
| `idx_method_qualifiedname` | `Method` | `qualifiedName` |
| `idx_rustfn_qualifiedname` | `RustFn` | `qualifiedName` |
| `idx_testmethod_qualifiedname` | `TestMethod` | `qualifiedName` |
| `idx_testclass_qualifiedname` | `TestClass` | `qualifiedName` |
| `idx_file_path` | `File` | `path` |
| `idx_task_id` | `Task` | `id` |
| `idx_commit_hash` | `Commit` | `hash` |
| `idx_module_name` | `Module` | `name` |
| `idx_package_name` | `Package` | `name` |
| `idx_project_name` | `Project` | `name` |
| `idx_component_name` | `Component` | `name` |
| `idx_decision_name` | `Decision` | `name` |
| `idx_defect_name` | `Defect` | `name` |
| `idx_feature_name` | `Feature` | `name` |
| `idx_ruststruct_name` | `RustStruct` | `name` |
| `idx_rusttrait_name` | `RustTrait` | `name` |
| `idx_rustenum_name` | `RustEnum` | `name` |
| `idx_rustalias_name` | `RustAlias` | `name` |
| `idx_divergence_id` | `Divergence` | `id` |

The Rust type labels' identity is `{name, file}`; the engine rejects composite indexes,
so the index covers `name`, the selective half. `divergences_kg.py` creates
`idx_divergence_id` if it is missing. Create an index with `CREATE INDEX <name> IF NOT
EXISTS FOR (n:<Label>) ON (n.<property>)`; list them with `SHOW INDEXES`.

**Proof that an index is used** (re-run after an engine upgrade):
`PROFILE MATCH (c:Class {qualifiedName:'org.apache.lucene.index.IndexWriter'}) RETURN c`
must plan `NodeByIndexSeek`, not `NodeByLabelScan` + `Filter`; the integer `Task.id`
plans `NodeByIndexRangeScan`; a batched `UNWIND … MERGE (m:TestMethod {qualifiedName:
qn})` shows a `Merge` operator with no label scan beneath it.

**No index serves an unlabelled node.** `MATCH (t {name: row.name, file: row.file})`
and `MATCH (n) WHERE id(n) = row.id` scan every node for every row. The sync tools
route the Rust item pattern through `kgio.unwind_items`, which binds it with each
label its `(name, file)` carries.

**Constraints:** none are declared. Before declaring `IS UNIQUE` on an identity
property, this must return 0: `MATCH (n:<Label>) WITH n.<property> AS v, count(*) AS c
WHERE c > 1 RETURN count(v)`.

---

## Query pitfalls

**`NOT EXISTS { MATCH … WHERE … }` returns wrong answers on this engine** (observed:
it returned types a direct `MATCH` proves are required by an open task). Use
`OPTIONAL MATCH` and count:

```cypher
MATCH (u:Class) WHERE u.portScope = 'in' AND u.portState = 'unported'
OPTIONAL MATCH (t:Task)-[:REQUIRES_PORT]->(u) WHERE t.status <> 'COMPLETED'
RETURN u.name AS type, count(DISTINCT t) AS openTasks
```

**An alias that collides with a pattern variable silently returns the node id.**
`MATCH (t)-[:PORTS]->(c) RETURN c.memberTotal AS t` returns `t`'s id. Name aliases so
they cannot collide.

**`REQUIRES_PORT` misses ambiguous simple names.** A type whose simple name collides
across packages carries no edge even when a task names it; check the task texts.

**A pair may hold edges of several types.** On rmp 1.18.0 and later, `MERGE
(a)-[:X]->(b)` over a pair joined by an edge of another type creates the `X` edge; a
`MERGE` of a type the pair holds creates nothing. Where a pair is joined by more than
one edge, set or delete an edge's properties only after re-binding it with
`MATCH (a)-[e:X]->(b)`, never through a matched `MERGE`. Undirected patterns report
the forward edge's type for a reverse traversal, so write every read outgoing.

---

## Provenance convention

Every node and edge carries `gitCommit` (full 40-char hash) and `gitDate`
(`YYYY-MM-DD`): the commit at which the fact was last confirmed. For a node describing
the Lucene source, it is the local Rucene commit at which it was registered. A sync
stamps only what it re-derived: stamping a fact it did not re-verify would be a false
claim. The audit fails on a node without a stamp. Unstamped edges, by type:

```cypher
MATCH ()-[e]->() WHERE e.gitCommit IS NULL
RETURN type(e) AS edge, count(e) AS unstamped ORDER BY unstamped DESC
```

---

## Reproducing the graph

**One commit:** `tools/kg/sync_commit.sh <commit> [roadmap]`. The order is fixed by
what each step reads:

1. `extract_rucene_kg.py --expand` → `load_rucene_kg.py` — the crate survey, and the
   prune of nodes it no longer confirms.
2. `port_coverage_kg.py` — `portScope`, `portState`, Lucene `DEPENDS_ON`,
   `PORTS_CANDIDATE`, the `Task` mirror and its prune, `REQUIRES_PORT`.
3. `port_evidence_kg.py --phase extract`, `--phase load` — the declared `PORTS`;
   `expanded_evidence_kg.py` — the same for macro-generated items;
   `port_evidence_kg.py --phase depth` — the depth floor (needs both).
4. `nested_port_kg.py` — the nested types' `portState` (needs step 3).
5. `port_coverage_kg.py --phase scope`, then `--phase candidates` (separate
   invocations).
6. `triage_dump.py` → `triage_repro.py` (fails unless it reproduces the graph's
   `memberMatched`) → `triage_members.py` → `triage_load.py` — the member triage.
7. `rmp_closings.py` → `commits_kg.py --closings` — the commit history and task edges.
8. `rust_refs_kg.py` → `no_caller_kg.py` → `no_caller_verdicts_kg.py` — crate
   references, `PORTS_MEMBER`, the no-caller list and the #269 verdicts.
9. `divergences_kg.py` — the declared divergences.
10. `audit_kg.py --commit <commit>` — `problems: 0`.

**From an empty store:** the Lucene half first (`lucene_modules_kg.py`,
`extract_lucene_kg.py` + `run_kg_batches.py`, `enrich_lucene_kg.py` +
`lucene_structure_kg.py`, `lucene_calls_kg.py`, `lucene_tests_kg.py`), then the chain
above; `tools/kg/README.md` gives the arguments.

## What the audit checks

`tools/kg/audit_kg.py`, after every sync; `problems: 0` is the pass condition. Each
check compares the graph with a source it re-derives independently:

* **Lucene:** `lucene/core` Java files, top-level and nested types (a brace-depth
  scanner written independently of the extractor), registered modules.
* **Crate:** `.rs` files, types and functions (from the compiler's own expansion), and
  no type the compiler sees that the graph lacks.
* **Port state:** every `Class` has a `portScope`; every nested type a measured state
  and every unported one a destination; every `PORTS` edge has `evidence`; no type has
  both `PORTS` and `PORTS_CANDIDATE`; no nested `ported` without a `PORTS`; no `PORTS`
  from a node the survey does not confirm.
* **Provenance:** at least the surveyed commit's history is mirrored; every node is
  stamped.
* **Mirrors:** `Task` nodes equal `rmp`'s tasks in both directions; `Divergence` nodes
  equal the crate's declared paragraphs in both directions.
