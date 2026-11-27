# kgsync — fast differential sync of the Rucene knowledge graph

`kgsync` keeps the `rucene` roadmap's knowledge graph (`rmp graph`) equal to the
sources. The graph answers three questions:

1. **What the reference library is made of.** Every module of Apache Lucene Core 10.5.0
   and of the other reference projects. For each module: its packages, files, types and
   members, from the **javac AST**. The test surface is included where the graph has one.
2. **What the `rucene` crate is made of.** Its crates, files, module paths and items,
   from the **`syn` AST**, following the compiler's module tree.
3. **How the two correspond.** Each Rust item and the Lucene type or member it ports
   (`PORTS`, `PORTS_MEMBER`), the skeletons, the declared divergences, the commits and
   the `rmp` tasks.

## Usage

```bash
# once: build (standalone crate, not a workspace member)
CARGO_TARGET_DIR=../../../.rucene-kg-target/kgsync cargo build --release --offline   # from tools/kgsync

# after each commit, from the repository root (the graph server must be running:
# rmp graph serve -r rucene)
../.rucene-kg-target/kgsync/release/kgsync
```

| Option | Effect |
|---|---|
| `--layers rust,java,map,git` | The layers to run (default: all four). |
| `--full` | Re-derive and verify every layer, even when its inputs are unchanged. |
| `--dry-run` | Compute and report the difference; write nothing. |
| `--schema` | Declare a `UNIQUE` constraint on every label's identity and drop the plain indexes it supersedes. |
| `--allow-dirty` | Sync an uncommitted working tree (by default the tree must be clean, so the graph equals a commit). |
| `--verbose` | Report every slice and the time of each phase. |
| `KGSYNC_SAMPLE=n` | Print the first *n* creates, updates and deletes of each slice. |

The run ends with one line: the slices it reconciled, `+created ~updated -deleted`,
the statements it sent, and the total time.

## Design

**Desired state versus current state, and nothing else.** Each layer computes from the
primary sources the *desired* state of the graph slices it owns. A slice is the nodes of
one label, or the edges of one (source label, type, target label), together with the
properties the layer owns. The reconciler reads each slice's *current* state in one
labelled statement and writes only the difference. Nothing is rebuilt, and an unchanged
tree writes nothing. The sync keeps no state of its own, so every run is also a full
verification: a slice that drifted is repaired by the next run.

**Ownership.** A layer only touches what it owns. It owns a label's nodes through a
filter (for example `n.language = 'Rust'`), the properties it lists, and the edges of a
type through a filter (for example `e.evidence = 'doc-comment'`). Curated nodes, curated
edges (other `evidence` values) and properties written by hand or by analyses are never
written or deleted.

**Every core.**

* The sources are parsed in parallel: `syn` with rayon, and javac in a pool sized to the
  machine.
* The current state of every slice is read **while** the sources are parsed. A
  specification pass declares the slices first; the server serves concurrent readers.
* The differences are planned in parallel.
* Writes are sequential, because the store has a single writer.

**Skip what did not change.** Each layer's inputs are fingerprinted (path, size and
mtime of every source file, plus the configuration and the extractor version). The
fingerprints are stored on `Project {name: 'Rucene'}` as `kgsyncRust`, `kgsyncJava`,
`kgsyncMap` and `kgsyncGit`. A layer whose fingerprint is unchanged is skipped. The
Lucene sources never change, so the Java layer runs only on `--full` or when its inputs
change.

| Layer | Source | Owns |
|---|---|---|
| `rust` | `syn` AST of every `.rs` file reached from a Cargo target | Rust `File`, `RustStruct`/`RustTrait`/`RustEnum`/`RustAlias`/`RustFn` (`origin: "source"`), their `DECLARES`, `IMPLEMENTS` (crate traits), `DEPENDS_ON` (`use`), crate `Module` → `File` |
| `java` | javac AST (`java/JavaAst.java`) of every module listed as a `Module {kind: 'maven-module'}`, roots in `references.json` | `Package`, Java `File`, `Class`, `Method` (methods, constructors, fields, enum constants), `TestClass`/`TestMethod` (from `src/test`), `CONTAINS`, `DECLARES`, `NESTED_IN`, `EXTENDS`, `IMPLEMENTS`, `DEPENDS_ON` |
| `map` | the crate's doc comments, joined with both ASTs | `PORTS`/`SKELETON_OF` (`evidence: "doc-comment"`), `PLANS_PORT` (`"skeleton-doc-comment"`), `Divergence` with `DECLARED_IN`/`CONCERNS`/`DIVERGES_FROM`, `PORTS_MEMBER` (`"name-mapping"`) |
| `git` | `git log`, and the roadmap store read-only (`rmp task list` caps at 100) | `Commit`, `Task`, `COMMITTED_IN`, `CLOSES_TASK`, `REFERENCES_TASK` |

Every node and edge a run creates or changes is stamped with `gitCommit`/`gitDate`. An
unchanged element keeps its stamp. `Project.syncedCommit` records the commit at which
the whole graph was last verified.

## Engine measurements (rmp 1.21.0, 2026-11-24)

The write shapes are chosen from these measurements (`PROFILE`/`EXPLAIN` and wall time
on the live graph); re-measure after an engine upgrade.

| Operation | Shape used | Why |
|---|---|---|
| Lookup by identity | `UNIQUE` constraint per identity | The constraint's index serves `NodeByIndexSeek`, so plain indexes are redundant. |
| Node update | Literal `MATCH (n:L {k: v}) SET n.p = …`, 200 per statement, chained with `WITH count(*) AS _` | 200 updates in 145 ms. **`SET n += {…}` plans a label scan per clause** (50 clauses: 2.7 s). |
| Node create | `UNWIND rows CREATE (n:L) SET n += r` | 2000 nodes in 87 ms. |
| Node delete | Chain `SET n:KgTomb`, then `MATCH (n:KgTomb) … DETACH DELETE` with `LIMIT` | `DELETE` never uses an index: 200 indexed deletes exceed the 5 s budget. |
| Edge create | Literal chain (inline property map) for ≤ 600 edges; `UNWIND … MATCH … MATCH … CREATE` above that | 200 in 108 ms (chain); 3000 in 837 ms (`UNWIND`). |
| Edge delete | Chain `SET e.kgTomb, a:KgDirty`, then `MATCH (a:KgDirty)-[e]->() WHERE e.kgTomb DELETE e` with `LIMIT` | A chained `DELETE` exceeds the budget; the tombstone pass takes about 55 ms. |
| Edge reads | Grouped by source: `RETURN a.k, collect(b.k)` | 25–42 % less to serialise than one row per edge. |
| Large reads | Labels above 20k nodes read in 4 `id(n) % 4` partitions | 68k rows: 410 → 183 ms alone; about 0.1 s per sync under load. |
| Reads | One labelled statement per slice, all concurrent | 4 parallel readers are 2.3× faster than serial. Cost grows with the columns returned (68k rows: 131 ms for the key alone, 409 ms with 9 properties). |

`rust-analyzer scip` is the only resolver for the crate's `REFERENCES`. It was measured
at 6 min 31 s wall time, 107 % CPU (single-threaded) and 10.8 GB RSS. It has no
incremental mode, so it is not part of this sync, and `REFERENCES` edges are left as
they are.

## Where the time goes, and what was measured

Two on-disk caches keep work the inputs do not require off the critical path. Both are
under `.rucene-kg-target/kgsync-cache/` and both are keyed by their inputs:

* **Per-file Rust extraction.** A file whose size and mtime are unchanged is not
  parsed again. Rust extraction goes from 1.5 s to 0.3 s. The cache name carries
  `FINGERPRINT_VERSION`: **bump it in `src/main.rs` whenever an extractor's output
  changes.**
* **The Lucene index of the mapping layer.** It is cached under the java layer's
  fingerprint, so it is read from the graph once per Lucene change. While the sources
  are parsed it costs nothing.

A commit that changes Rust sources now runs:

* extraction, done at about 0.5 s;
* desired state, done at about 1.0 s;
* the graph reads, done at about 1.1 s;
* planning, 0.2 s, using zero-copy key views.

The reads and the desired state run concurrently and finish together.

Measured and rejected:

* **A snapshot of the last applied state** on disk, validated per slice by `count`
  and a generation token. It replaces the graph reads, but loading about 60 MB of
  JSON plus the counts is slower than reading the graph (3.4–3.8 s against
  1.7–2.4 s), and it weakens the per-run verification.
* **A per-file digest** that reads only the changed files' slices. Its ceiling is
  bounded by the same reads, and it loses the full verification.

## Measured performance (16 cores, 2026-11-24)

| Case | Time | Writes |
|---|---|---|
| Nothing changed | 0.15–0.22 s | none (every layer skipped) |
| A commit that changes Rust sources (`rust` + `map`) | about 1.8 s (3.5–4 s under heavy machine load) | only the difference |
| A commit with no Rust or Java change (`git`) | 0.3–0.9 s | the new commit and tasks |
| `--full` (all four layers verified) | 4–7 s | only the difference |
| First run over the previous graph (one-off migration, about 147k changes) | 56 s | — |
