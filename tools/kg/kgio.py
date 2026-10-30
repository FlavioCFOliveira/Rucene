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
PER_ROW_SCAN_BATCH = 20

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
        result = subprocess.run(
            ["rmp", "graph", "client", "-r", roadmap],
            input=query, text=True, capture_output=True, check=False,
        )
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


def unwind(roadmap, mode, rows, body, label, batch=300):
    """`UNWIND [<rows>] AS row <body>`, in batches.

    Returns the engine's write counters summed over the batches. The line it
    prints gives the rows *sent* and what the engine reports it changed: the
    row count is the input, never a result.
    """
    total = {}
    if not rows:
        print(f"  {label}: 0 rows sent", file=sys.stderr)
        return total
    for i in range(0, len(rows), batch):
        chunk = rows[i : i + batch]
        maps = ", ".join(
            "{" + ", ".join(f"{k}:{esc(v)}" for k, v in r.items()) + "}"
            for r in chunk
        )
        out = client(roadmap, f"UNWIND [{maps}] AS row\n{body}",
                     label=f"{label} ({mode}, rows {i + 1}-{i + len(chunk)})")
        for k, v in counters_of(out).items():
            total[k] = total.get(k, 0) + v
    print(f"  {label}: {len(rows)} rows sent; {format_counters(total)}", file=sys.stderr)
    return total
