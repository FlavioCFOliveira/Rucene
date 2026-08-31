#!/usr/bin/env python3
"""Shared `rmp graph` I/O for the KG tools.

Three things every loader here needs and must not get wrong:

* **Cypher map literals, not JSON.** The engine wants unquoted map keys, so
  `json.dumps` cannot be used to build an `UNWIND` payload.
* **Retry on a busy store.** A concurrent writer -- an `rmp web` session is
  enough -- makes a write fail. Dying mid-load leaves the graph half-applied,
  which a later audit cannot tell apart from real drift.
* **Batching.** One `rmp` invocation per statement costs ~19,000 process
  launches for the Lucene member surface alone.
"""
import json
import subprocess
import sys
import time


def esc(v):
    """A Cypher literal for `v`."""
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return str(v)
    return "'" + str(v).replace("\\", "\\\\").replace("'", "\\'") + "'"


def rmp(mode, roadmap, query, attempts=6):
    delay = 1.0
    result = None
    for attempt in range(1, attempts + 1):
        result = subprocess.run(
            ["rmp", "graph", mode, "-r", roadmap],
            input=query, text=True, capture_output=True,
        )
        if result.returncode == 0 or "store is busy" not in result.stderr:
            break
        if attempt < attempts:
            print(f"  store busy, retrying in {delay:.0f}s", file=sys.stderr)
            time.sleep(delay)
            delay = min(delay * 2, 20)
    if result.returncode != 0:
        raise SystemExit(f"{mode} failed: {result.stderr[:500]}\n{query[:400]}")
    return result.stdout


def read(roadmap, query):
    """Rows of a read query, as a list of lists."""
    return json.loads(rmp("query", roadmap, query))["rows"]


def read_dicts(roadmap, query):
    doc = json.loads(rmp("query", roadmap, query))
    return [dict(zip(doc["columns"], row)) for row in doc["rows"]]


def unwind(roadmap, mode, rows, body, label, batch=300):
    """`UNWIND [<rows>] AS row <body>`, in batches."""
    if not rows:
        print(f"  {label}: 0", file=sys.stderr)
        return
    for i in range(0, len(rows), batch):
        maps = ", ".join(
            "{" + ", ".join(f"{k}:{esc(v)}" for k, v in r.items()) + "}"
            for r in rows[i : i + batch]
        )
        rmp(mode, roadmap, f"UNWIND [{maps}] AS row\n{body}")
    print(f"  {label}: {len(rows)}", file=sys.stderr)
