# KG extraction tools

Scripts that populate the `rmp` Knowledge Graph for the `rucene` roadmap. Together
they rebuild the whole graph from an empty store: the reference Apache Lucene Core
10.5.0 structure, the Rucene crate structure, and the port-coverage layer that says
what is ported, what is missing, and what to do next.

They are survey tooling, not part of the Rucene crate. They need only Python 3 and
the `rmp` binary on `PATH`.

## The scripts

| Script | What it does |
|---|---|
| `extract_lucene_kg.py` | Regex extractor for the Lucene side: packages, source files, top-level types, imports, `extends` and `implements`. Emits Cypher files. |
| `enrich_lucene_kg.py` | Second Lucene pass: inner types and members — methods, constructors, fields (initialised or not) and enum constants, the last under their own `kind`, `enumConstant`, as Lucene's own Javadoc categorises them. A member needs no modifier to be seen: an interface's abstract methods and every package-private member carry none, and requiring one hid 949 methods, 338 constructors and 438 fields — 10.1% of the denominator every depth and triage figure is quoted against. Emits Cypher files. |
| `run_kg_batches.py` | Feeds a generated Cypher file into `rmp graph create` / `rmp graph update`, one statement per invocation. |
| `load_members_unwind.py` | Superseded by `lucene_structure_kg.py`, which loads the same facts from the extractor's data instead of re-parsing its generated Cypher. Kept for replaying an old run. |
| `extract_rucene_kg.py` | Regex extractor for the Rucene side: every `.rs` file under `src/` and `tests/`, its structs, traits, enums, type aliases, functions and tests, its `impl` blocks, and the dependencies implied by its `use` declarations. Emits one JSON survey. |
| `load_rucene_kg.py` | Loads that survey into the graph in `UNWIND` batches, and keeps the Rucene side of the graph honest: collapses legacy labels, merges duplicate nodes, stamps provenance, and audits the result against the survey. |
| `port_coverage_kg.py` | The coverage layer: marks the in-scope Lucene surface, derives Lucene type→type dependencies, links open `rmp` tasks to the types they need, records `Component.status`, and writes the scope `Decision`. |
| `port_evidence_kg.py` | Reads the `Equivalent to `org.apache.lucene…`` claims the crate's own doc comments make (`CLAUDE.md` §14.1) and turns them into `PORTS` edges with `evidence: "doc-comment"`, then measures how much of each ported type is actually present. A claim naming a *member* (`…TwoPhaseCommitTool.execute`) resolves to the enclosing type when the graph confirms that type declares that member, and the edge records `claimLevel: "member"` and `declaredMember`; a nested type (`TotalHits.Relation`) goes on resolving as a type. It also **retires** a `doc-comment` edge no live claim declares any more, so a port cannot outlive its evidence. |
| `expanded_evidence_kg.py` | The port evidence that only exists after macro expansion: reads the doc comment the compiler prints above each macro-generated type (`#[doc = concat!(…)]` is only text once expanded) and turns its `Equivalent to \`org.apache.lucene…\`` claim into a `PORTS` edge carrying `evidence: "doc-comment"` and `declarationSource: "macro-expansion"`. |
| `lucene_modules_kg.py` | Registers all 35 modules of the Lucene distribution with their size and an explicit `inScope` flag, so port coverage is quoted against a stated denominator (`CLAUDE.md` §6.1). |
| `lucene_tests_kg.py` | Loads the Lucene test surface (`TestClass`, `TestMethod`) and its correspondence with the crate. |
| `lucene_structure_kg.py` | Loads the Lucene nested types and members in `UNWIND` batches, calling `enrich_lucene_kg.py`'s extractors directly. Replaces feeding its generated Cypher through `run_kg_batches.py`, which cost one process launch per statement — about 23,000 for the member surface. `--prune` removes nested types the reference tree no longer declares; the member phase never prunes. |
| `commits_kg.py` | Mirrors the full commit history and derives `CLOSES_TASK` from the message conventions the history actually uses. |
| `rust_surface.py` | Parses the crate's Rust surface the way the member triage needs it: every `impl` and `trait` block with the functions inside it, the doc comment above each one, the `#[derive]`s on each type declaration, and the `macro_rules!` bodies that `extract_rucene_kg.py` deliberately blanks. A scanner, not a compiler — brace depth over masked text. |
| `triage_dump.py` | Snapshots, once, the graph facts the triage reads, so the classification runs against a fixed graph state instead of a moving one. Writes one JSON file per query into the output directory, `supers_all.json` among them — the Lucene type hierarchy without the `portScope` filter, which `triage_members.py` loads separately for the `doc-comment` and `related` rules. It is a required input: without it the re-derivation aborts with `FileNotFoundError` before it classifies anything. |
| `triage_repro.py` | Reproduces `measure_depth` exactly and emits the `(Lucene type, Java method)` pairs it could not match. Fails if its own per-type counts differ from the `memberMatched` the graph holds — the guard that keeps the triage attached to the floor it classifies. |
| `triage_members.py` | Classifies every unmatched pair as `absent`, `renamed` or `na`, with the rule that decided it and the Rust item or the reason. Rules are tiered by how tightly the item is bound to the type (`own`, `super`, `file`) and run strongest first; anything no rule settles is `absent`. See *Where the remaining work is, by package* in `knowledge-model.md`. |
| `triage_load.py` | Writes the verdicts to the graph: `portTriage` / `portTriageRule` / `portTriageEvidence` on each `Method`, and `memberAbsent` / `memberRenamed` / `memberNa` on each measured `Class`. |
| `retirement_check.py` | Proof, off the store, that a port cannot outlive its declaration: a hand-built graph in which a file that declared a port has been deleted, asserting the edge is retired and the node and `File` pruned, that a curated or macro-expansion port is spared, that a live claim is untouched, and that a member-level claim resolves to its enclosing type while a nested type still resolves as a type. Run `python3 tools/kg/retirement_check.py`; `failures: 0` is the pass condition. |
| `member_extraction_check.py` | Proof, off the store and off the Lucene clone, that the member extractor sees every shape it has lost at one time or another: enum constants (implicitly `public static final`), fields carrying an initialiser up to an anonymous class spanning many lines, members with no modifier at all, an annotation written *before* the modifiers, code a mis-ordered lexer deletes, C-style array declarators — and that the traps opening the regex up creates stay shut, a continuation line and the blanked Javadoc above a member both being offered to it as declarations. A hand-built source tree modelled on `PreloadHint`, `ScoreMode`, `HnswGraph`, `Accountable`, `DocumentsWriterDeleteQueue`, `Version`, `Automaton`, `RegExp` and `SimpleWKTShapeParser`. Run `python3 tools/kg/member_extraction_check.py`; `failures: 0` is the pass condition. |
| `audit_kg.py` | The adversarial fidelity audit. Re-derives both sides from primary sources — the Lucene clone, parsed by a scanner written independently of the loaders (only the lexical pass, `javatext.strip_java`, is shared), and the crate as the compiler expands it — and reports every divergence. `problems: 0` is the pass condition. |
| `javatext.py` | The Java lexical pass both Lucene-side scanners need: `strip_java` blanks comments, strings and char literals in one left-to-right pass, preserving length. Order is not negotiable — stripping strings *before* line comments lets a `//` holding an odd number of `"` open a phantom string that swallows the code after it, which is what deleted `Automaton#toDot` and `RegExp#parseCharExp`. The loader and the audit share it so they cannot drift; the audit's independence rests on its scanner, not on disagreeing about what a token is. |
| `kgio.py` | Shared `rmp graph` I/O: Cypher map-literal serialisation (the engine rejects JSON's quoted keys), `UNWIND` batching, and retry-with-backoff when the store is busy. |

## Rebuilding the graph from scratch

Get the reference sources first (`CLAUDE.md` §16.1). Clone the reference sources first:

```bash
git clone --branch releases/lucene/10.5.0 --single-branch \
    https://github.com/apache/lucene.git /tmp/lucene1050
```

Every script now takes `--lucene-root` and defaults to `/tmp/lucene1050`, the
path `CLAUDE.md` §16.1 names. They used to hard-code `/tmp/lucene-10.5.0`, which
left the Lucene half of the pipeline unable to run at all against a clone at the
documented path.

Then, with `COMMIT` and `DATE` set to the Rucene commit being surveyed
(`git rev-parse HEAD` and `git show -s --format=%cs HEAD`):

```bash
COMMIT=$(git rev-parse HEAD)
DATE=$(git show -s --format=%cs HEAD)

# 0. Lucene: the module inventory that states the port's denominator
python3 tools/kg/lucene_modules_kg.py rucene --lucene-root /tmp/lucene1050 \
    --commit "$COMMIT" --date "$DATE"

# 1. Lucene: packages, files, top-level types, dependencies
python3 tools/kg/extract_lucene_kg.py \
    --source-root /tmp/lucene1050/lucene/core/src/java/org/apache/lucene \
    --output-dir /tmp/lucene_kg --commit "$COMMIT" --date "$DATE"
for f in /tmp/lucene_kg/nodes_*.cypher /tmp/lucene_kg/edges_*.cypher; do
    python3 tools/kg/run_kg_batches.py create rucene "$f"
done
python3 tools/kg/run_kg_batches.py update rucene /tmp/lucene_kg/update.cypher

# 2. Lucene: inner types, members and the JPMS descriptor
python3 tools/kg/lucene_structure_kg.py rucene --lucene-root /tmp/lucene1050 \
    --commit "$COMMIT" --date "$DATE" --phase all --prune

# 3. Rucene: the crate survey
python3 tools/kg/extract_rucene_kg.py --source-root . --expand \
    --commit "$COMMIT" --date "$DATE" --output /tmp/rucene_kg/survey.json
python3 tools/kg/load_rucene_kg.py rucene --survey /tmp/rucene_kg/survey.json

# 4. Port coverage, tasks and the scope decision
python3 tools/kg/port_coverage_kg.py rucene --survey /tmp/rucene_kg/survey.json \
    --lucene-root /tmp/lucene1050 --commit "$COMMIT" --date "$DATE"

# 5. The port evidence the crate states about itself
python3 tools/kg/port_evidence_kg.py rucene --source-root . --phase extract
python3 tools/kg/port_evidence_kg.py rucene --source-root . --phase load \
    --commit "$COMMIT" --date "$DATE"

# 5b. The same evidence for the types that exist only after macro expansion
python3 tools/kg/expanded_evidence_kg.py rucene --source-root . \
    --commit "$COMMIT" --date "$DATE"

# 5c. The depth of each port, now that every declared edge is in place
python3 tools/kg/port_evidence_kg.py rucene --source-root . --phase depth \
    --commit "$COMMIT" --date "$DATE"

# 6. The commit history
python3 tools/kg/commits_kg.py rucene

# 7. The Lucene test surface
python3 tools/kg/lucene_tests_kg.py rucene --lucene-root /tmp/lucene1050 \
    --repo-root . --commit "$COMMIT" --date "$DATE"

# 8. Prove it: the audit must end in `problems: 0`
python3 tools/kg/audit_kg.py rucene --survey /tmp/rucene_kg/survey.json
```

Step 5 must run after step 4: it supersedes `PORTS_CANDIDATE` edges with the
declared `PORTS` edges, and the depth measurement reads the `portScope` the
coverage phase assigns. Step 5b must run between the load and the depth
measurement, because `measure_depth` resolves a ported type's Rust functions
through the `(name, file)` of a `PORTS` edge: without the edges 5b writes, the
macro-generated types are never measured and their Lucene members stay outside
the denominator. Re-run `port_coverage_kg.py` afterwards so `portState` reflects the
edges steps 5 and 5b wrote — as **two** invocations, because `--phase` is a
single-choice argument and rejects a comma-separated list:

```bash
python3 tools/kg/port_coverage_kg.py rucene --survey /tmp/rucene_kg/survey.json \
    --lucene-root /tmp/lucene1050 --commit "$COMMIT" --date "$DATE" --phase scope
python3 tools/kg/port_coverage_kg.py rucene --survey /tmp/rucene_kg/survey.json \
    --lucene-root /tmp/lucene1050 --commit "$COMMIT" --date "$DATE" --phase candidates
```

### `--expand`: the macro-generated types

`extract_rucene_kg.py --expand` runs `cargo +nightly rustc --lib --
-Zunpretty=expanded` and records the types and inherent methods that only exist
after macro expansion. Without it the survey misses 118 real types — the 24
`BulkOperationPackedN`, the `internal::hppc` containers, the range fields and the
doc-values iterators — and `load_rucene_kg.py --phase audit` reports them as
stale nodes to delete, which would silently drop their `PORTS` edges. The flag
needs the nightly toolchain; `--expanded-file` reuses a captured dump instead.

`expanded_evidence_kg.py` reads the same dump for the second thing only the
compiler can show: the *doc comment* printed above each of those types, which is
where their `Equivalent to \`org.apache.lucene…\`` claim lives — a claim written
`#[doc = concat!(…)]` is not text until it is expanded. It takes the same
`--expanded-file`, so one captured dump serves the survey, the evidence pass and
`audit_kg.py` alike:

```bash
cargo +nightly rustc --lib -- -Zunpretty=expanded > /tmp/rucene_kg/expanded.rs
```

Step 4 must run after step 3: the candidate detection reads the crate survey, and
`portState` is derived from the `PORTS` edges the Rucene load has repaired.

## Surveying a commit that is not checked out

Never survey a dirty working tree — another task's half-finished work would enter
the graph as fact. Use a detached worktree:

```bash
git worktree add --detach /tmp/rucene-survey <commit>
python3 tools/kg/extract_rucene_kg.py --source-root /tmp/rucene-survey --expand \
    --commit <full-sha> --date <YYYY-MM-DD> --output /tmp/rucene_kg/survey.json
git worktree remove /tmp/rucene-survey
```

## `load_rucene_kg.py` phases

`--phase all` (the default) runs them in this order; each can also be run alone.

| Phase | What it does |
|---|---|
| `repair` | Hygiene, using the survey as ground truth: deletes unlabelled nodes and the Lucene stub duplicates (re-linking their `PORTS` to the canonical type), re-points a `PORTS` that aimed at a Java *file* to the type, lifts `TestSuite` edges to file level, and collapses `Struct`/`Trait`/`Enum`/`Interface`/`Test`/`Component` and Rust-side `Class` nodes onto `RustStruct`/`RustTrait`/`RustEnum`/`RustAlias`/`RustFn`. Nodes are matched by `id(n)`, the only unambiguous key while legacy labels coexist. |
| `nodes` | Creates the crate root, then the `File`, `RustStruct`, `RustTrait`, `RustEnum`, `RustAlias` and `RustFn` nodes. |
| `edges` | `CONTAINS`, `DECLARES`, `IMPLEMENTS`, `DEPENDS_ON`. |
| `reconcile` | Merges pre-existing nodes that the survey shows to be the same type as a canonical one (missing or wrong `file`, wrong label, or an outright duplicate), moving their edges first so nothing is lost. |
| `stamp` | Stamps `gitCommit`/`gitDate` on every edge shape this loader owns. It runs after `reconcile` because moving an edge creates a new one. It never touches the Lucene side. |
| `prune` | Deletes the `File` nodes and the type/function nodes the survey no longer confirms. Two safeguards: an empty survey is refused, so a failed extraction can never be read as "the crate has no types"; and a node is spared when its `PORTS` edge is one the crate survey is not the authority on — `evidence: "curated"` (asserted by hand, with no source site) or `declarationSource: "macro-expansion"` (declared by a type no source line contains, invisible to a survey run without `--expand`). It used to spare *every* node carrying `PORTS`, which — with nothing retiring a stale edge — left a deleted file's port in the graph for ever. |
| `audit` | Compares the graph against the survey: every `.rs` file has exactly one node, every type node matches the survey's `(name, file)` and label, no stale node, no duplicate. Exits with a report; `audit problems: 0` is the pass condition. |

## `port_coverage_kg.py` phases

| Phase | What it does |
|---|---|
| `scope` | Sets `portScope`/`portScopeRule` on every Lucene type and `portState` from the `PORTS` edges. |
| `deps` | Loads Lucene type→type `DEPENDS_ON`, derived from `import` declarations plus same-package references (Java needs no import inside a package). |
| `candidates` | Adds `PORTS_CANDIDATE` for a Lucene type with no `PORTS` whose simple name matches exactly one Rucene type, and moves it to `portState = 'candidate'`. |
| `tasks` | Mirrors every `rmp` task with its live status, links each open task to the Lucene types its statement names (`REQUIRES_PORT`), and mirrors the task dependencies. |
| `components` | Fills `Component.status`. |
| `decision` | Writes the scope rule as an auditable `Decision` node. |

## Gotchas these scripts encode

- `rmp graph create` rejects any `SET`, including inside `MERGE … ON CREATE SET`.
  Every upsert is therefore two steps: `create` with identity properties only, then
  `update` with the rest.
- `MERGE` matches the **whole** pattern, so only identity properties may appear in a
  `MERGE` map, and existing nodes are located with `MATCH` before being related.
- `MATCH (x), (y) WHERE id(x) = … AND id(y) = …` builds the Cartesian product of all
  nodes before filtering. Anchor each end separately: `MATCH (x) WHERE id(x) = … WITH
  x MATCH (y) WHERE id(y) = …`.
- Undirected relationship patterns resolve the wrong type on node pairs that carry
  edges both ways. Always traverse outgoing, and take the union of the two legs to
  cover both directions.
- `MERGE (a)-[:X]->(b)` matches an existing relationship of a **different** type
  between the same ordered pair and creates nothing, silently. Adding a second edge
  type between two already-connected nodes means deleting the first one.
- `Task.id` is stored as an **integer**. Writing it as a string creates a second,
  duplicate node for the same task.
- `rmp task list` returns at most 100 tasks; its date filters are only reliable in
  `YYYY-MM-DD` form, and `--created-until D` excludes day `D` itself. `fetch_tasks`
  therefore walks the project's date range one day at a time, with the window
  `[D, D+1)`.
