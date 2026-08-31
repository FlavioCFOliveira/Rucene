#!/usr/bin/env python3
"""Executa um ficheiro Cypher em batches via rmp graph create/update."""
import subprocess
import time
import sys
import argparse


def run_batch(cmd, query, idx, total, use_stdin, attempts=6):
    # A concurrent writer (an `rmp web` session is enough) makes the store reject
    # a write. Dying here leaves the load half-applied, which later reads as real
    # drift, so retry the transient case with backoff.
    delay = 1.0
    for attempt in range(1, attempts + 1):
        if use_stdin:
            result = subprocess.run(cmd, input=query, text=True, capture_output=True)
        else:
            result = subprocess.run(cmd + ['-q', query], text=True, capture_output=True)
        if result.returncode == 0 or 'store is busy' not in result.stderr:
            break
        if attempt < attempts:
            time.sleep(delay)
            delay = min(delay * 2, 20)
    if result.returncode != 0:
        print(f"\nBATCH {idx}/{total} FAILED (exit {result.returncode})", file=sys.stderr)
        print(result.stderr, file=sys.stderr)
        print("QUERY:\n" + query[:500], file=sys.stderr)
        sys.exit(1)
    out = result.stdout.strip()
    if out != '{"ok":true}':
        print(f"BATCH {idx}/{total}: {out[:200]}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('mode', choices=['create', 'update'])
    parser.add_argument('roadmap')
    parser.add_argument('file')
    parser.add_argument('--batch-size', type=int, default=80)
    args = parser.parse_args()

    with open(args.file, 'r', encoding='utf-8') as f:
        lines = [line.strip() for line in f if line.strip() and not line.strip().startswith('//')]

    use_stdin = args.mode == 'create'
    subcmd = 'create' if args.mode == 'create' else 'update'
    base_cmd = ['rmp', 'graph', subcmd, '-r', args.roadmap]

    # rmp graph create/update only accept one Cypher statement per invocation.
    args.batch_size = 1

    batches = [lines[i:i + args.batch_size] for i in range(0, len(lines), args.batch_size)]
    total = len(batches)
    print(f"Running {total} batches of up to {args.batch_size} lines ({len(lines)} total)", file=sys.stderr)
    for idx, batch in enumerate(batches, 1):
        query = '\n'.join(batch)
        run_batch(base_cmd, query, idx, total, use_stdin)
        if idx % 50 == 0 or idx == total:
            print(f"  {idx}/{total} done", file=sys.stderr)
    print(f"All {args.mode} batches completed.", file=sys.stderr)


if __name__ == '__main__':
    main()
