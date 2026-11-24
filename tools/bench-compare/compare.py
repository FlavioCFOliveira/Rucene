"""Side-by-side table of the byTask reports of two Benchmark runs.

Usage: python3 -I compare.py <lucene.out> <rucene.out>

Parses every "------------> Report ..." block (RepSumByName,
RepSumByPrefRound, ...) of both outputs. Report rows are fixed-width: every
column but "Operation" is right-aligned under its header, and Lucene's
`Report` pads alternate rows with " -" filler, so each value is the last
token of the slice that ends where its header ends.

Exit status 2 when the two runs' reports differ in shape (report titles,
operations, rounds, run counts or records per run): the timing columns are
then not comparable.
"""

import sys

REPORT_MARK = "------------> Report "


def parse_reports(path):
    """Return [(title, header_names, [row_dict, ...]), ...]."""
    with open(path, encoding="utf-8", errors="replace") as f:
        lines = f.read().splitlines()
    reports = []
    i = 0
    while i < len(lines):
        if lines[i].startswith(REPORT_MARK) and i + 1 < len(lines):
            title = lines[i][len(REPORT_MARK):].strip()
            header = lines[i + 1]
            names = header.split()
            ends, pos = [], 0
            for name in names:
                pos = header.index(name, pos) + len(name)
                ends.append(pos)
            rows = []
            i += 2
            while i < len(lines) and lines[i].strip():
                line = lines[i]
                row = {names[0]: line.split()[0]}
                for k in range(1, len(names)):
                    cell = line[ends[k - 1]:ends[k]].split()
                    row[names[k]] = cell[-1] if cell else ""
                rows.append(row)
                i += 1
            reports.append((title, names, rows))
        else:
            i += 1
    return reports


def num(s):
    return float(s.replace(",", ""))


def ratio(r, l):
    lv, rv = num(l), num(r)
    return f"{rv / lv:.2f}" if lv else "-"


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: compare.py <lucene.out> <rucene.out>")
    lucene, rucene = parse_reports(sys.argv[1]), parse_reports(sys.argv[2])
    diverged = []
    if not lucene:
        sys.exit(f"no report found in {sys.argv[1]}")
    if [t for t, _, _ in lucene] != [t for t, _, _ in rucene]:
        diverged.append(
            f"report titles differ: lucene={[t for t, _, _ in lucene]} "
            f"rucene={[t for t, _, _ in rucene]}"
        )

    cols = ("Operation", "round", "runCnt", "recsPerRun", "L rec/s", "R rec/s",
            "R/L rec/s", "L elapsed", "R elapsed", "R/L elapsed")
    for (title, _, lrows), (_, _, rrows) in zip(lucene, rucene):
        print(f"== {title}")
        table = [cols]
        for k in range(max(len(lrows), len(rrows))):
            lr = lrows[k] if k < len(lrows) else None
            rr = rrows[k] if k < len(rrows) else None
            if lr is None or rr is None:
                diverged.append(f"{title}: row {k} only in {'rucene' if lr is None else 'lucene'}")
                continue
            same = all(lr[c] == rr[c] for c in ("Operation", "round", "runCnt", "recsPerRun"))
            if not same:
                diverged.append(
                    f"{title}: row {k}: lucene "
                    f"{[lr[c] for c in ('Operation', 'round', 'runCnt', 'recsPerRun')]} "
                    f"rucene {[rr[c] for c in ('Operation', 'round', 'runCnt', 'recsPerRun')]}"
                )
            mark = "" if same else " (!)"
            recs = lr["recsPerRun"] if lr["recsPerRun"] == rr["recsPerRun"] else (
                f"{lr['recsPerRun']}|{rr['recsPerRun']}")
            runs = lr["runCnt"] if lr["runCnt"] == rr["runCnt"] else f"{lr['runCnt']}|{rr['runCnt']}"
            table.append((
                lr["Operation"] + mark, lr["round"], runs, recs,
                lr["rec/s"], rr["rec/s"], ratio(rr["rec/s"], lr["rec/s"]),
                lr["elapsedSec"], rr["elapsedSec"], ratio(rr["elapsedSec"], lr["elapsedSec"]),
            ))
        widths = [max(len(r[c]) for r in table) for c in range(len(cols))]
        for r in table:
            print("  ".join(
                r[c].ljust(widths[c]) if c == 0 else r[c].rjust(widths[c])
                for c in range(len(cols))
            ))
        print()
    print("L = Lucene 10.5.0, R = Rucene; R/L rec/s > 1 and R/L elapsed < 1 mean Rucene is faster.")
    if diverged:
        print("\nREPORT SHAPE DIVERGES (timings not comparable where marked (!)):")
        for d in diverged:
            print(f"  {d}")
        sys.exit(2)


if __name__ == "__main__":
    main()
