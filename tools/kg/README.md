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
| `enrich_lucene_kg.py` | Second Lucene pass: inner types and members (methods, constructors, fields). Emits Cypher files. |
| `run_kg_batches.py` | Feeds a generated Cypher file into `rmp graph create` / `rmp graph update`, one statement per invocation. |
| `load_members_unwind.py` | Loads the member files with far fewer `rmp` invocations, batching rows into `UNWIND` statements. |
| `extract_rucene_kg.py` | Regex extractor for the Rucene side: every `.rs` file under `src/` and `tests/`, its structs, traits, enums, type aliases, functions and tests, its `impl` blocks, and the dependencies implied by its `use` declarations. Emits one JSON survey. |
| `load_rucene_kg.py` | Loads that survey into the graph in `UNWIND` batches, and keeps the Rucene side of the graph honest: collapses legacy labels, merges duplicate nodes, stamps provenance, and audits the result against the survey. |
| `port_coverage_kg.py` | The coverage layer: marks the in-scope Lucene surface, derives Lucene type→type dependencies, links open `rmp` tasks to the types they need, records `Component.status`, and writes the scope `Decision`. |

## Rebuilding the graph from scratch

Get the reference sources first (`CLAUDE.md` §16.1). The two Lucene extractors were
written for a clone at `/tmp/lucene-10.5.0` and hard-code that path (and, in
`enrich_lucene_kg.py`, the commit stamp of the original survey), so clone there — or
symlink — when replaying steps 1 and 2:

```bash
git clone --branch releases/lucene/10.5.0 --single-branch \
    https://github.com/apache/lucene.git /tmp/lucene-10.5.0
ln -sfn /tmp/lucene-10.5.0 /tmp/lucene1050   # the path CLAUDE.md names
```

Then, with `COMMIT` and `DATE` set to the Rucene commit being surveyed
(`git rev-parse HEAD` and `git show -s --format=%cs HEAD`):

```bash
COMMIT=$(git rev-parse HEAD)
DATE=$(git show -s --format=%cs HEAD)

# 1. Lucene: packages, files, top-level types, dependencies
python3 tools/kg/extract_lucene_kg.py \
    --source-root /tmp/lucene-10.5.0/lucene/core/src/java/org/apache/lucene \
    --output-dir /tmp/lucene_kg --commit "$COMMIT" --date "$DATE"
for f in /tmp/lucene_kg/nodes_*.cypher /tmp/lucene_kg/edges_*.cypher; do
    python3 tools/kg/run_kg_batches.py create rucene "$f"
done
python3 tools/kg/run_kg_batches.py update rucene /tmp/lucene_kg/update.cypher

# 2. Lucene: inner types, members and the JPMS descriptor
python3 tools/kg/enrich_lucene_kg.py --output-dir /tmp/lucene_kg_enrich --run
python3 tools/kg/load_members_unwind.py rucene

# 3. Rucene: the crate survey
python3 tools/kg/extract_rucene_kg.py --source-root . \
    --commit "$COMMIT" --date "$DATE" --output /tmp/rucene_kg/survey.json
python3 tools/kg/load_rucene_kg.py rucene --survey /tmp/rucene_kg/survey.json

# 4. Port coverage, tasks and the scope decision
python3 tools/kg/port_coverage_kg.py rucene --survey /tmp/rucene_kg/survey.json \
    --lucene-root /tmp/lucene-10.5.0 --commit "$COMMIT" --date "$DATE"
```

Step 4 must run after step 3: the candidate detection reads the crate survey, and
`portState` is derived from the `PORTS` edges the Rucene load has repaired.

## Surveying a commit that is not checked out

Never survey a dirty working tree — another task's half-finished work would enter
the graph as fact. Use a detached worktree:

```bash
git worktree add --detach /tmp/rucene-survey <commit>
python3 tools/kg/extract_rucene_kg.py --source-root /tmp/rucene-survey \
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
