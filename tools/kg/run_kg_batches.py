#!/usr/bin/env python3
"""Executa um ficheiro Cypher em batches via `rmp graph client` (`kgio.client`)."""
import json
import sys
import argparse

from kgio import client


def run_batch(roadmap, query, idx, total):
    # A failure exits non-zero (`kgio.client`): dying here is recoverable, while
    # a skipped statement leaves the load half-applied, which later reads as
    # real drift.
    doc = json.loads(client(roadmap, query, label=f"BATCH {idx}/{total}"))
    # A write acknowledges with {"ok": true[, "counters": {...}]}; anything
    # else (a statement that returned rows) is worth showing.
    if doc.get("ok") is not True:
        print(f"BATCH {idx}/{total}: {json.dumps(doc, separators=(',', ':'))[:200]}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('mode', choices=['create', 'update'])
    parser.add_argument('roadmap')
    parser.add_argument('file')
    parser.add_argument('--batch-size', type=int, default=80)
    args = parser.parse_args()

    with open(args.file, 'r', encoding='utf-8') as f:
        lines = [line.strip() for line in f if line.strip() and not line.strip().startswith('//')]

    # `mode` named the subcommand of the old CLI (`rmp graph create|update`);
    # `rmp graph client` runs both kinds of statement, so it is kept for the
    # callers and the log line only.

    # rmp graph client accepts one Cypher statement per invocation.
    args.batch_size = 1

    batches = [lines[i:i + args.batch_size] for i in range(0, len(lines), args.batch_size)]
    total = len(batches)
    print(f"Running {total} batches of up to {args.batch_size} lines ({len(lines)} total)", file=sys.stderr)
    for idx, batch in enumerate(batches, 1):
        query = '\n'.join(batch)
        run_batch(args.roadmap, query, idx, total)
        if idx % 50 == 0 or idx == total:
            print(f"  {idx}/{total} done", file=sys.stderr)
    print(f"All {args.mode} batches completed.", file=sys.stderr)


if __name__ == '__main__':
    main()
