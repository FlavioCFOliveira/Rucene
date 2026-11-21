# Groadmap graph engine: defect report

Defects of the `rmp graph` Cypher engine observed while maintaining Rucene's
knowledge graph, each with a minimal reproduction on an empty roadmap. Written
for the Groadmap maintainers (Rucene rmp task #507).

* **Version:** `rmp --version` → `Groadmap version 1.21.0 (commit f5526d7)`.
* **Platform:** Linux; graph served with `rmp graph serve -r <roadmap>`,
  statements sent with `rmp graph client -r <roadmap> -q <cypher>`.
* **Reproduced:** 2026-11-15, on throwaway roadmaps that the script creates and
  removes.

| # | Observation | Status |
|---|---|---|
| D1 | `AllNodesScan` misses checkpointed nodes after nodes are created since the last checkpoint | **Defect**, reproduced |
| D2 | A chain joined by `WITH 1 AS _` stops after a `MATCH` that finds nothing | **Not a defect**: conformant Cypher semantics |
| D3 | `CREATE` that reuses a variable name after a `WITH` that drops it creates no node | **Defect**, reproduced |

## D1. `AllNodesScan` misses checkpointed nodes

**Reproduction** (the D1 part of the script below):

```cypher
UNWIND range(1, 50000) AS i CREATE (:Fn {id: i})
-- checkpoint: stop the server with SIGTERM, then `rmp graph serve` again
UNWIND range(1, 1000) AS i CREATE (:Class {id: i})
MATCH (n:Fn) RETURN count(n)            -- 50000
MATCH (n) WHERE n:Fn RETURN count(n)    -- 44646
```

* **Expected:** both counts are 50000; the two statements differ only in how
  the label is matched.
* **Observed:** the labelled scan returns 50000; the statement planned as
  `Project, CountRows, ColumnarFilter, AllNodesScan` returns 44646. After
  another checkpoint it returns 50000. No error is reported.
* **Conditions (measured):** the nodes missed are checkpointed ones; nodes
  created since the checkpoint are seen. The loss needs a large enough store
  and enough nodes created since the checkpoint; a delete is not needed.

  | Checkpointed `:Fn` (other nodes) | Created after the checkpoint | `MATCH (n) WHERE n:Fn` |
  |---|---|---|
  | 1,000 | 1,000 | 1,000 (no loss) |
  | 10,000 | 1,000 | 10,000 (no loss) |
  | 50,000 | 1 | 50,000 (no loss) |
  | 50,000 | 1,000 | 44,646 |
  | 15,000 (30,000 other) | 1,500 | 14,698 |
  | 30,000 (60,000 other) | 3,000 | 15,361 |

* **First seen:** during a Rucene sync, `MATCH (n) WHERE n:Class` returned 2557
  against 2816 for `MATCH (n:Class)`, and every pattern opening on an
  unlabelled node lost the same rows.
* **Workaround in `tools/kg`:** `sync_commit.sh` restarts the server
  (`checkpoint()`: SIGTERM, then `serve`) after each of the 17 graph-writing
  steps (`run`), about 108 s of every sync; `kgio.item_labels` /
  `kgio.unwind_items` route every Rust-item pattern to one labelled statement
  per label instead of an unlabelled start. The engine offers no checkpoint
  procedure that a client could call instead.

## D2. `WITH 1 AS _` chains stop after a `MATCH` that finds nothing

**Reproduction:**

```cypher
UNWIND range(1, 4) AS i CREATE (:K {id: i})
MATCH (a:K {id: 1}) SET a.v = 1 WITH 1 AS _
MATCH (b:K {id: 99}) SET b.v = 1 WITH 1 AS _
MATCH (c:K {id: 3}) SET c.v = 1                 -- propertiesWritten: 1
MATCH (a:K {id: 2}) SET a.w = 1 WITH count(*) AS _
MATCH (b:K {id: 99}) SET b.w = 1 WITH count(*) AS _
MATCH (c:K {id: 4}) SET c.w = 1                 -- propertiesWritten: 2
```

* **Observed:** with `WITH 1 AS _`, only `id 1` is set; with
  `WITH count(*) AS _`, ids 2 and 4 are set. No error either way.
* **Assessment:** this is Cypher's semantics, not a defect. A `MATCH` that finds
  nothing produces no rows, `WITH 1 AS _` projects one row per input row (none),
  and every later clause runs zero times; `count(*)` aggregates zero rows into
  one row, so the chain goes on. It is listed here because Rucene's tooling
  first recorded it as an engine defect.
* **In `tools/kg`:** `kgio` joins chained clauses with `WITH count(*) AS _`
  (`kgio.py`, the comment above `CHAIN_CLAUSES`). That is the correct idiom and
  needs no change when the engine changes.

## D3. `CREATE` reusing a variable name after a `WITH` creates no node

**Reproduction:**

```cypher
CREATE (t:V {id: 1}) WITH count(*) AS _ CREATE (t:V {id: 2}) WITH count(*) AS _ CREATE (t:V {id: 3})
MATCH (n:V) RETURN n.id ORDER BY n.id     -- [[1]]
CREATE (t:U {id: 1}) WITH 1 AS _ CREATE (t:U {id: 2}) WITH 1 AS _ CREATE (t:U {id: 3})
MATCH (n:U) RETURN n.id ORDER BY n.id     -- [[1]]
CREATE (:C {id: 1}) WITH count(*) AS _ CREATE (:C {id: 2}) WITH count(*) AS _ CREATE (:C {id: 3})
MATCH (n:C) RETURN n.id ORDER BY n.id     -- [[1], [2], [3]]
```

* **Expected:** three nodes in each case. `WITH count(*) AS _` and
  `WITH 1 AS _` project only `_`, so `t` is out of scope after them and
  `CREATE (t:V {id: 2})` declares a new variable and creates a new node.
* **Observed:** `nodesCreated: 1` and one node for the first two statements,
  with no error; the anonymous-node form creates three.
* **Workaround in `tools/kg`:** `kgio.chainable` refuses to chain any body
  holding a `CREATE` clause (`_CREATE_CLAUSE`), so node creation always runs as
  `UNWIND … CREATE` or one comma-separated `CREATE` pattern, giving up the
  index-seeking chain for those statements.

## Reproduction script

Run with `rmp` on `PATH` and no server running for the roadmap it names
(default `groadmap-defect-repro`). It creates that roadmap, serves its graph,
runs the statements above, and removes the roadmap on exit.

```bash
#!/bin/bash
# Minimal reproductions of the rmp graph-engine observations (Rucene rmp #507).
# Runs on a throwaway roadmap, created here and removed on exit.
set -u
R=${1:-groadmap-defect-repro}
LOG=$(mktemp)
PID=
trap '[ -n "$PID" ] && kill -TERM "$PID" 2>/dev/null; sleep 0.5; rmp roadmap remove "$R" >/dev/null 2>&1; rm -f "$LOG"' EXIT
rmp --version
rmp roadmap create "$R" >/dev/null || exit 2
start() {   # serve the roadmap's graph; returns once the socket is bound
  : > "$LOG"
  setsid nohup rmp graph serve -r "$R" > "$LOG" 2>&1 < /dev/null &
  PID=$!
  for _ in $(seq 150); do grep -q '"socket"' "$LOG" && return 0; sleep 0.2; done
  echo "server did not start"; cat "$LOG"; exit 3
}
checkpoint() {   # SIGTERM checkpoints the store; then serve again
  kill -TERM "$PID"; while kill -0 "$PID" 2>/dev/null; do sleep 0.2; done; start
}
q() {
  echo "> $1"
  rmp graph client -r "$R" -q "$1" | python3 -c 'import json,sys
d=json.load(sys.stdin)
print("  rows:", d.get("rows"), "| counters:", d.get("counters"))'
}
plan() {
  echo "> EXPLAIN $1"
  rmp graph client -r "$R" -q "EXPLAIN $1" | python3 -c 'import json,sys
def ops(p):
    yield p.get("operatorType") or p.get("operator") or p.get("name")
    for c in p.get("children", []): yield from ops(c)
print("  operators:", list(ops(json.load(sys.stdin)["plan"])))'
}

start
echo "=== D1. AllNodesScan misses checkpointed nodes once nodes were created after the checkpoint"
q "UNWIND range(1, 50000) AS i CREATE (:Fn {id: i})"
checkpoint
q "MATCH (n) WHERE n:Fn RETURN count(n)"
q "UNWIND range(1, 1000) AS i CREATE (:Class {id: i})"
q "MATCH (n:Fn) RETURN count(n)"
q "MATCH (n) WHERE n:Fn RETURN count(n)"
plan "MATCH (n) WHERE n:Fn RETURN count(n)"
checkpoint
q "MATCH (n) WHERE n:Fn RETURN count(n)"

echo "=== D2. A chain stops after a MATCH that finds nothing when clauses are joined by WITH 1 AS _"
q "UNWIND range(1, 4) AS i CREATE (:K {id: i})"
q "MATCH (a:K {id: 1}) SET a.v = 1 WITH 1 AS _ MATCH (b:K {id: 99}) SET b.v = 1 WITH 1 AS _ MATCH (c:K {id: 3}) SET c.v = 1"
q "MATCH (k:K) RETURN k.id, k.v ORDER BY k.id"
q "MATCH (a:K {id: 2}) SET a.w = 1 WITH count(*) AS _ MATCH (b:K {id: 99}) SET b.w = 1 WITH count(*) AS _ MATCH (c:K {id: 4}) SET c.w = 1"
q "MATCH (k:K) RETURN k.id, k.w ORDER BY k.id"

echo "=== D3. CREATE reusing a variable name after WITH count(*) AS _ creates one node"
q "CREATE (t:V {id: 1}) WITH count(*) AS _ CREATE (t:V {id: 2}) WITH count(*) AS _ CREATE (t:V {id: 3})"
q "MATCH (n:V) RETURN n.id ORDER BY n.id"
q "CREATE (t:U {id: 1}) WITH 1 AS _ CREATE (t:U {id: 2}) WITH 1 AS _ CREATE (t:U {id: 3})"
q "MATCH (n:U) RETURN n.id ORDER BY n.id"
q "CREATE (:C {id: 1}) WITH count(*) AS _ CREATE (:C {id: 2}) WITH count(*) AS _ CREATE (:C {id: 3})"
q "MATCH (n:C) RETURN n.id ORDER BY n.id"
```

### Output of the run (2026-11-15)

```text
Groadmap version 1.21.0 (commit f5526d7)
=== D1. AllNodesScan misses checkpointed nodes once nodes were created after the checkpoint
> UNWIND range(1, 50000) AS i CREATE (:Fn {id: i})
  rows: None | counters: {'nodesCreated': 50000, 'propertiesWritten': 50000, 'labelsAdded': 50000}
> MATCH (n) WHERE n:Fn RETURN count(n)
  rows: [[50000]] | counters: None
> UNWIND range(1, 1000) AS i CREATE (:Class {id: i})
  rows: None | counters: {'nodesCreated': 1000, 'propertiesWritten': 1000, 'labelsAdded': 1000}
> MATCH (n:Fn) RETURN count(n)
  rows: [[50000]] | counters: None
> MATCH (n) WHERE n:Fn RETURN count(n)
  rows: [[44646]] | counters: None
> EXPLAIN MATCH (n) WHERE n:Fn RETURN count(n)
  operators: ['Project', 'CountRows', 'ColumnarFilter', 'AllNodesScan']
> MATCH (n) WHERE n:Fn RETURN count(n)
  rows: [[50000]] | counters: None
=== D2. A chain stops after a MATCH that finds nothing when clauses are joined by WITH 1 AS _
> UNWIND range(1, 4) AS i CREATE (:K {id: i})
  rows: None | counters: {'nodesCreated': 4, 'propertiesWritten': 4, 'labelsAdded': 4}
> MATCH (a:K {id: 1}) SET a.v = 1 WITH 1 AS _ MATCH (b:K {id: 99}) SET b.v = 1 WITH 1 AS _ MATCH (c:K {id: 3}) SET c.v = 1
  rows: None | counters: {'propertiesWritten': 1}
> MATCH (k:K) RETURN k.id, k.v ORDER BY k.id
  rows: [[1, 1], [2, None], [3, None], [4, None]] | counters: None
> MATCH (a:K {id: 2}) SET a.w = 1 WITH count(*) AS _ MATCH (b:K {id: 99}) SET b.w = 1 WITH count(*) AS _ MATCH (c:K {id: 4}) SET c.w = 1
  rows: None | counters: {'propertiesWritten': 2}
> MATCH (k:K) RETURN k.id, k.w ORDER BY k.id
  rows: [[1, None], [2, 1], [3, None], [4, 1]] | counters: None
=== D3. CREATE reusing a variable name after WITH count(*) AS _ creates one node
> CREATE (t:V {id: 1}) WITH count(*) AS _ CREATE (t:V {id: 2}) WITH count(*) AS _ CREATE (t:V {id: 3})
  rows: None | counters: {'nodesCreated': 1, 'propertiesWritten': 1, 'labelsAdded': 1}
> MATCH (n:V) RETURN n.id ORDER BY n.id
  rows: [[1]] | counters: None
> CREATE (t:U {id: 1}) WITH 1 AS _ CREATE (t:U {id: 2}) WITH 1 AS _ CREATE (t:U {id: 3})
  rows: None | counters: {'nodesCreated': 1, 'propertiesWritten': 1, 'labelsAdded': 1}
> MATCH (n:U) RETURN n.id ORDER BY n.id
  rows: [[1]] | counters: None
> CREATE (:C {id: 1}) WITH count(*) AS _ CREATE (:C {id: 2}) WITH count(*) AS _ CREATE (:C {id: 3})
  rows: None | counters: {'nodesCreated': 3, 'propertiesWritten': 3, 'labelsAdded': 3}
> MATCH (n:C) RETURN n.id ORDER BY n.id
  rows: [[1], [2], [3]] | counters: None
```
