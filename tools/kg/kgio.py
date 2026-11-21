#!/usr/bin/env python3
"""Shared `rmp graph client` I/O for the KG tools.

Every statement the KG tools issue goes through `client`: one Cypher statement
per `rmp graph client` invocation, sent to the graph server the operator keeps
running (`rmp graph serve -r <roadmap>`). The tools never start or stop a server.

Four things every loader here needs and must not get wrong:

* **Cypher map literals, not JSON.** The engine wants unquoted map keys, so
  `json.dumps` cannot be used to build an `UNWIND` payload.
* **Classified failures.** `rmp graph client` exits 1 for every failure and
  prints one published `Error:` line, which is not always the first line of
  stderr. Only the write-conflict line is retried: it means every attempt of
  rmp's own retry policy (2.5 s) lost a serialisation conflict to another
  writer, that nothing was written, and that the same statement should simply be
  run again. Every other failure stops the tool with a non-zero exit, because
  dying mid-load is recoverable and a silently skipped statement is not: it
  leaves the graph half-applied, which a later audit cannot tell apart from real
  drift.
* **Batching.** One `rmp` invocation per statement costs ~19,000 process
  launches for the Lucene member surface alone, so loaders send `UNWIND` batches.
* **The 5-second statement budget.** The server cancels a statement that runs
  longer than 5 s, and a batch is one statement, so each batch must fit the
  budget whole. See `PER_ROW_SCAN_BATCH`.
"""
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

# Where every KG tool writes by default: its JSON surveys, generated Cypher, the
# cargo target of `--expand`, and its reports. `/tmp` is tmpfs (RAM) on the
# development machine, and the large outputs of a sync written there have filled
# swap, so no tool here defaults to `/tmp` or to a `tempfile` directory.
#
# The default is `.rucene-kg-target`, a sibling of the repository checkout
# (beside `.rucene-dev-target`): `/data/dev/github.com/FlavioCFOliveira/
# .rucene-kg-target` for the canonical clone. `RUCENE_KG_TARGET` overrides it,
# and each tool's own output flag overrides both.
KG_TARGET_ENV = "RUCENE_KG_TARGET"
REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_KG_TARGET = REPO_ROOT.parent / ".rucene-kg-target"


def kg_target() -> Path:
    """The directory KG tool outputs default to (`RUCENE_KG_TARGET` overrides)."""
    env = os.environ.get(KG_TARGET_ENV)
    return Path(env) if env else DEFAULT_KG_TARGET

# The largest `UNWIND` batch for a statement the engine runs once per row over a
# whole label or the whole graph, instead of one scan plus a hash join. In rmp
# 1.18.0 (GoGraph v0.15.0) these are:
#
# * a node `MERGE (n:L {k: row.k})`, which walks the posting list of `L` for
#   every row (`cypher/exec/merge_search.go`);
# * a `MATCH` that binds an unlabelled node by property, `(t {name: row.name})`,
#   or by `id(n) = row.id`, which scans every node for every row.
#
# Size it on the WRITE path. A read runs on the lock-free snapshot path, a write
# statement inside the writer transaction, and the same per-row scan costs about
# ten times more there: the `PORTS_CANDIDATE` delete of `port_evidence_kg.py`
# reads at 4.7 ms per row and writes at 50 ms per row. Sized on its read side, a
# 200-row batch of it exceeded the budget in a real sync.
#
# Measured on 2026-10-23 by replaying the sync chain, statement for statement,
# against a copy of the `rucene` store (52,324 nodes) opened with the engine rmp
# 1.18.0 ships: an unlabelled match runs at 25-56 ms per row on the write path,
# so at 20 rows the slowest statement took 1.07 s; a node `MERGE` walks its
# label at 2.2 ms per row on `TestMethod` (5,746 nodes). A labelled
# `MATCH (n:L {k: row.k})` is hash-joined -- one label scan per statement, at
# most 0.04 s on the write path at 200 rows -- and needs no cap.
#
# Since 2026-11-03 every label carries a hash index on its identity property
# (`knowledge-model.md`, *Indexes and constraints*). A node `MERGE` on an
# indexed property is then an index seek -- measured on `TestMethod`: a 20-row
# batch in 0.12 ms in all, against 2.2 ms per row before -- and needs no cap
# either. The cap still governs the unlabelled match and the `id(n)` match,
# which no index can serve; the Rust item pattern `(t {name:row.name,
# file:row.file})` is routed through `unwind_items` instead, which binds it
# with its label.
PER_ROW_SCAN_BATCH = 20

# The labels whose nodes a Rust item pattern `(t {name:row.name, file:row.file})`
# can bind: the four type labels, `RustFn` and `Component` are the only ones that
# carry both `name` and a crate `file`.
RUST_ITEM_LABELS = ("RustStruct", "RustTrait", "RustEnum", "RustAlias", "RustFn", "Component")
ITEM_PATTERN = "(t {name:row.name, file:row.file})"

_ERROR_LINE = re.compile(r"^Error:.*$", re.MULTILINE)

# Fixed prefixes of the lines `rmp graph client` publishes (Groadmap v1.18.0,
# `internal/commands/graph_socket.go`; `SPEC/COMMANDS.md` makes the whole line
# the contract). Only these prefixes are matched, never the engine diagnostic
# that may follow them.
_NO_SERVER = (
    "Error: graph server error: no graph server is listening on ",
    "Error: graph server error: graph server unreachable at ",
)
_WRITE_CONFLICT = "Error: graph engine error: graph write conflict: "
_BUDGET = "Error: graph engine error: graph query exceeded the "


def esc(v):
    """A Cypher literal for `v`."""
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return str(v)
    return "'" + str(v).replace("\\", "\\\\").replace("'", "\\'") + "'"


def error_line(stderr: str) -> str:
    """The `Error:` line of a failed `rmp` invocation, or all of stderr."""
    m = _ERROR_LINE.search(stderr)
    return m.group(0).strip() if m else stderr.strip()


def _failure(roadmap: str, query: str, label: str, code: int, error: str) -> str:
    lines = [f"{label} FAILED (rmp graph client exit {code})", error]
    if error.startswith(_NO_SERVER):
        lines.append(
            f"No graph server is reachable for roadmap '{roadmap}'. Start one with "
            f"`rmp graph serve -r {roadmap}`, leave it running, and run this tool again.")
    elif error.startswith(_BUDGET):
        lines.append(
            "The statement exceeded the server's 5-second budget and wrote nothing. "
            "A batched statement needs a smaller batch (kgio.PER_ROW_SCAN_BATCH).")
    lines.append("QUERY:\n" + query[:800])
    return "\n".join(lines)


# `KG_TRACE=<file>` appends one tab-separated line per statement -- the caller
# script, its label, the seconds the statement took, its exit code and its
# length -- so a slow sync is attributed from measurement, not guessed.
_TRACE = os.environ.get("KG_TRACE")
_SCRIPT = Path(sys.argv[0]).name if sys.argv and sys.argv[0] else "?"


def _trace(label: str, query: str, seconds: float, code: int) -> None:
    if not _TRACE:
        return
    with open(_TRACE, "a", encoding="utf-8") as f:
        f.write(f"{_SCRIPT}\t{label}\t{seconds:.4f}\t{code}\t{len(query)}\n")


def client(roadmap: str, query: str, label: str = "statement", attempts: int = 6) -> str:
    """Run one statement on the roadmap's graph server; return stdout (JSON).

    A read prints `{"columns": [...], "rows": [...]}`; a write prints
    `{"ok": true}`, with a `counters` map when it changed the graph. The
    statement goes through stdin, which has no argv length limit.

    A write conflict is re-sent up to `attempts` times in all, waiting 1, 2, 4,
    8 and 16 s between attempts. Any other failure -- no server, a parse or
    execution error, the 5-second budget, a lost connection whose outcome is
    unknown -- raises `SystemExit` at once with the `Error:` line, so the tool
    exits non-zero and nothing is skipped silently.
    """
    delay = 1.0
    attempt = 1
    while True:
        started = time.monotonic()
        result = subprocess.run(
            ["rmp", "graph", "client", "-r", roadmap],
            input=query, text=True, capture_output=True, check=False,
        )
        _trace(label, query, time.monotonic() - started, result.returncode)
        if result.returncode == 0:
            return result.stdout
        error = error_line(result.stderr)
        if attempt >= attempts or not error.startswith(_WRITE_CONFLICT):
            raise SystemExit(_failure(roadmap, query, label, result.returncode, error))
        print(f"  {label}: write conflict, retrying in {delay:.0f}s "
              f"({attempt}/{attempts - 1})", file=sys.stderr)
        time.sleep(delay)
        delay = min(delay * 2, 20.0)
        attempt += 1


def rmp(mode: str, roadmap: str, query: str, attempts: int = 6) -> str:
    """`client`, labelled with the `rmp graph <mode>` name callers still pass.

    `mode` (`create`, `update`, `query`, `delete`) selected a subcommand of the
    old CLI; `rmp graph client` runs reads and writes alike, so it is now only
    the label a failure is reported under.
    """
    return client(roadmap, query, label=mode, attempts=attempts)


def read(roadmap, query):
    """Rows of a read query, as a list of lists."""
    return json.loads(client(roadmap, query, label="read"))["rows"]


def read_dicts(roadmap, query):
    doc = json.loads(client(roadmap, query, label="read"))
    return [dict(zip(doc["columns"], row)) for row in doc["rows"]]


def counters_of(stdout: str) -> dict:
    """The write counters of one `rmp graph client` result, `{}` when none.

    A statement that changed the graph carries a `counters` map
    (`relationshipsCreated`, `relationshipsDeleted`, `propertiesWritten`, ...;
    Groadmap v1.19.0 `internal/graphjson/counters.go`), each key left out when
    zero; one that changed nothing carries none.
    """
    try:
        doc = json.loads(stdout)
    except (json.JSONDecodeError, TypeError):
        return {}
    return dict(doc.get("counters") or {}) if isinstance(doc, dict) else {}


def format_counters(counters: dict) -> str:
    return ", ".join(f"{k}={v}" for k, v in sorted(counters.items())) or "no change"


# Bulk statements. Measured on rmp 1.21.0 (2026-11-11): inside `UNWIND` the
# planner serves an identity lookup by one labelled scan hash-joined with the
# rows, never by the hash index, so a statement costs a fixed ~140 ms (36,550
# `RustFn`) whatever its row count: 300 rows took 0.14 s and 6,000 rows 0.31 s.
# The fewest statements is therefore the fastest sync. A statement is capped by
# rows and by bytes (the server refuses one over 1 MiB, exit 6); when one still
# exceeds the 5-second budget -- a `MERGE` that misses scans its label per row
# -- the engine has written nothing, so the chunk is halved and re-sent.
BULK_ROWS = 5000
BULK_BYTES = 512 * 1024


def serialize(row: dict) -> str:
    """A row as a Cypher map literal."""
    return "{" + ", ".join(f"{k}:{esc(v)}" for k, v in row.items()) + "}"


def unwind_maps(roadmap, mode, maps, body, label, cap=None):
    """`UNWIND [<maps>] AS row <body>` over pre-serialised map literals, in bulk.

    `cap` keeps a caller's small row limit (a statement known to scan per row);
    otherwise a chunk holds up to `BULK_ROWS` rows and `BULK_BYTES` bytes. A
    chunk that exceeds the statement budget is halved and re-sent, and the
    smaller size is kept for the rest of the call. Returns the summed counters.
    """
    total = {}
    rows_cap = cap if cap is not None and cap > 0 else BULK_ROWS
    i, n = 0, len(maps)
    while i < n:
        j, size = i, 0
        while j < n and j - i < rows_cap and (j == i or size + len(maps[j]) + 2 <= BULK_BYTES):
            size += len(maps[j]) + 2
            j += 1
        statement = f"UNWIND [{', '.join(maps[i:j])}] AS row\n{body}"
        try:
            out = client(roadmap, statement, label=f"{label} ({mode}, rows {i + 1}-{j})")
        except SystemExit as e:
            if _BUDGET in str(e) and j - i > 1:
                rows_cap = max(1, (j - i) // 2)
                print(f"  {label}: budget exceeded at {j - i} rows, halving to "
                      f"{rows_cap}", file=sys.stderr)
                continue
            raise
        for k, v in counters_of(out).items():
            total[k] = total.get(k, 0) + v
        i = j
    return total


# Index-seeking chains. Measured on rmp 1.21.0 (2026-11-12): a literal
# identity lookup `(x:L {key: <literal>})` plans `NodeByIndexSeek`, while the
# same lookup through `UNWIND ... row.key` never does (label scan + hash
# join). Over 36,550 RustFn, the same writes took: 50 rows 0.022 s chained vs
# 0.382 s UNWIND; 300 rows 0.080 s vs 0.380 s; 1,000 rows 0.131 s vs 0.429 s;
# 3,000 rows 0.268 s vs 0.295 s; only at 6,000 rows did UNWIND win (0.278 s vs
# 0.730 s). A chain costs super-linearly beyond ~300 clauses (2,000 clauses:
# 2.2 s), so chains hold CHAIN_CLAUSES rows each.
#
# Clauses are separated by `WITH count(*) AS _`, which always yields exactly
# one row: `WITH 1 AS _` silently stops the chain at the first clause whose
# MATCH finds nothing (measured: 50 of 100 writes, no error). A `CREATE` inside
# such a chain creates one node only (engine defect, measured), so bodies with
# a CREATE clause stay on UNWIND -- creation needs no lookup, and UNWIND or one
# comma-separated CREATE pattern create 300 nodes in ~0.04 s.
CHAIN_CLAUSES = 200
CHAIN_MAX_ROWS = 3000
_ROW_REF = re.compile(r"\brow\.(\w+)")
_ROW_IN_WITH = re.compile(r",\s*row\b(?!\.)")
_CREATE_CLAUSE = re.compile(r"(?<!ON )\bCREATE\b")


# A chain only pays when every per-row lookup hits a hash index on a label much
# larger than the batch: without an index each clause scans its label (a 300-row
# chain on an unindexed 300-node label took 0.15 s against 0.02 s for UNWIND,
# measured), and on a small label UNWIND's single scan is cheap. RustFn (36,550
# nodes) favoured chains up to ~3,000 rows, i.e. a label/rows ratio of ~12.
CHAIN_LABEL_RATIO = 12
_LOOKUP = re.compile(r"\(\w*:(\w+)\s*\{([^}]*\brow\.[^}]*)\}\)")
_PROP = re.compile(r"(\w+)\s*:\s*row\.")
_index_cache = {}
_count_cache = {}


def _indexed(roadmap):
    if roadmap not in _index_cache:
        rows = json.loads(client(roadmap, "SHOW INDEXES", label="indexes"))["rows"]
        _index_cache[roadmap] = {(r[4][0], p) for r in rows if r[3] == "NODE" for p in r[5]}
    return _index_cache[roadmap]


def _label_count(roadmap, label):
    key = (roadmap, label)
    if key not in _count_cache:
        _count_cache[key] = read(roadmap, f"MATCH (n:{label}) RETURN count(n)")[0][0]
    return _count_cache[key]


def chainable(body: str, roadmap: str = None, nrows: int = 0, sample: dict = None) -> bool:
    """Whether `body` should run as an index-seeking chain (see above): no
    RETURN, no CREATE, and every `(x:Label {prop: row.k ...})` lookup has an
    indexed property on a label at least CHAIN_LABEL_RATIO times the rows,
    and (given a `sample` row) the plan of one bound clause seeks an index and
    scans no label (`_plan_seeks_only`)."""
    if "RETURN" in body or _CREATE_CLAUSE.search(body) or "row" not in body:
        return False
    lookups = _LOOKUP.findall(body)
    if not lookups or roadmap is None:
        return False
    idx = _indexed(roadmap)
    for label, props in lookups:
        if not any((label, p) in idx for p in _PROP.findall(props)):
            return False
        if _label_count(roadmap, label) < CHAIN_LABEL_RATIO * max(nrows, 1):
            return False
    return sample is None or _plan_seeks_only(roadmap, body, sample)


# The plan decides, not the text: a keyed lookup can still be planned from an
# unkeyed node of the same pattern. `(:Class)-[e:DECLARES]->(m:Method {qn: v})`
# started each chained clause with a scan of every Class and its DECLARES edges
# (57 chains averaged 3.06 s; the UNWIND form had cost one scan per statement --
# measured 2026-11-12). So one clause bound to a sample row is EXPLAINed per body
# shape, and a chain is used only if the plan seeks an index and scans nothing.
_plan_cache = {}
_SCANS = ("NodeByLabelScan", "AllNodesScan")


def _plan_operators(node, out):
    out.append(node.get("operator", ""))
    for child in node.get("children", []):
        _plan_operators(child, out)
    return out


def _plan_seeks_only(roadmap, body, sample):
    key = (roadmap, body)
    if key not in _plan_cache:
        doc = json.loads(client(roadmap, "EXPLAIN " + _bind(body, sample), label="explain"))
        ops = _plan_operators(doc.get("plan") or {}, [])
        _plan_cache[key] = (any("IndexSeek" in o for o in ops)
                            and not any(o in _SCANS for o in ops))
    return _plan_cache[key]


def _bind(body: str, row: dict) -> str:
    """`body` for one row: `row.k` -> literal, `, row` dropped from WITH lists."""
    template = _ROW_IN_WITH.sub("", body)
    # Checked on the template, never on the bound text: a literal value may
    # itself contain the word "row" (observed in no_caller evidence text).
    if re.search(r"\brow\b", _ROW_REF.sub("", template)):
        raise ValueError(f"cannot bind every use of `row` in: {body[:200]}")
    return _ROW_REF.sub(lambda m: esc(row[m.group(1)]), template)


def chain(roadmap, mode, rows, body, label, cap=None):
    """Run `body` once per row as literal index-seeking clauses, chained.

    Returns the summed write counters. A chain over the 5 s budget wrote
    nothing and is halved and re-sent, as `unwind_maps` does.
    """
    total = {}
    size = cap if cap is not None and cap > 0 else CHAIN_CLAUSES
    clauses = [_bind(body, r) for r in rows]
    i, n = 0, len(clauses)
    while i < n:
        j, nbytes = i, 0
        while j < n and j - i < size and (j == i or nbytes + len(clauses[j]) <= BULK_BYTES):
            nbytes += len(clauses[j]) + 20
            j += 1
        statement = " WITH count(*) AS _ ".join(clauses[i:j])
        try:
            out = client(roadmap, statement, label=f"{label} ({mode}, chain rows {i + 1}-{j})")
        except SystemExit as e:
            if _BUDGET in str(e) and j - i > 1:
                size = max(1, (j - i) // 2)
                print(f"  {label}: budget exceeded at {j - i} clauses, halving to {size}",
                      file=sys.stderr)
                continue
            raise
        for k, v in counters_of(out).items():
            total[k] = total.get(k, 0) + v
        i = j
    return total


def unwind(roadmap, mode, rows, body, label, batch=None):
    """Run `body` once per row in bulk, by the fastest measured shape.

    Up to `CHAIN_MAX_ROWS` rows of a chainable body (no RETURN, no CREATE) run
    as index-seeking literal chains (`chain`); otherwise, or for creation,
    `UNWIND [<rows>] AS row <body>` in bulk (`unwind_maps`). `batch` is
    honoured only at or below `PER_ROW_SCAN_BATCH`, the cap of a statement that
    scans per row. Returns the engine's write counters summed. The line it
    prints gives the rows *sent* and what the engine reports it changed.
    """
    if not rows:
        print(f"  {label}: 0 rows sent", file=sys.stderr)
        return {}
    cap = batch if batch is not None and batch <= PER_ROW_SCAN_BATCH else None
    if len(rows) <= CHAIN_MAX_ROWS and chainable(body, roadmap, len(rows), rows[0]):
        total = chain(roadmap, mode, rows, body, label, cap)
    else:
        total = unwind_maps(roadmap, mode, [serialize(r) for r in rows], body, label, cap)
    print(f"  {label}: {len(rows)} rows sent; {format_counters(total)}", file=sys.stderr)
    return total


def item_labels(roadmap, labels=RUST_ITEM_LABELS):
    """`{(name, file): {label, ...}}` over every node of `labels`.

    The routing table of `unwind_items`: one read per label, instead of a scan of
    the whole graph for every row a batch sends.
    """
    out = {}
    for label in labels:
        for name, file in read(roadmap, f"MATCH (n:{label}) RETURN n.name, n.file"):
            out.setdefault((name, file), set()).add(label)
    return out


def route_items(rows, index):
    """`{label: [row, ...]}`: each row under every label its `(name, file)` carries.

    A row whose `(name, file)` names no node of the index's labels is dropped:
    the unlabelled pattern matched nothing for it either.
    """
    out = {}
    for r in rows:
        for label in sorted(index.get((r["name"], r["file"]), ())):
            out.setdefault(label, []).append(r)
    return out


def unwind_items(roadmap, mode, rows, body, label, index, batch=None):
    """`unwind` for a body that binds the Rust item as `ITEM_PATTERN`.

    Each row is sent once per label its `(name, file)` carries in `index`
    (`item_labels`), with the pattern rewritten to `(t:<Label> {...})`. That binds
    exactly the nodes the unlabelled pattern bound among the index's labels, and
    a labelled match is hash-joined or index-served instead of scanning every
    node of the graph for every row. Returns the summed write counters.
    """
    if ITEM_PATTERN not in body:
        raise ValueError(f"unwind_items: body does not bind {ITEM_PATTERN}")
    total = {}
    routed = route_items(rows, index)
    if not routed:
        print(f"  {label}: 0 rows routed ({len(rows)} given)", file=sys.stderr)
        return total
    for lab, sel in sorted(routed.items()):
        typed = body.replace(ITEM_PATTERN, f"(t:{lab} {{name:row.name, file:row.file}})")
        for k, v in unwind(roadmap, mode, sel, typed, f"{label} [{lab}]", batch).items():
            total[k] = total.get(k, 0) + v
    return total
