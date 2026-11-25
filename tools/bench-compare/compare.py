"""Side-by-side table of the byTask reports of Lucene and Rucene Benchmark runs.

Usage: python3 -I compare.py [--tsv FILE] <lucene.out> <rucene.out>
       python3 -I compare.py [--tsv FILE] <lucene.out>... -- <rucene.out>...

With several runs per engine, every rec/s and elapsedSec cell (and the
process wall-clock total read from "<out>.wall", when present) is the median
over that engine's runs. --tsv also writes one machine-readable line per row.

Parses every "------------> Report ..." block (RepSumByName,
RepSumByPrefRound, RepSumByNameRound, RepSelectByPref, RepSumByPref, ...) of
every output, whatever its columns (round-varying properties included). Report rows are fixed-width: every
column but "Operation" is right-aligned under its header, and Lucene's
`Report` pads alternate rows with " -" filler, so each value is the last
token of the slice that ends where its header ends (after realigning rows
whose operation name overflows the Operation column, and joining the lines
of an operation name that holds newlines).

Exit status 2 when the reports differ in shape between any two runs (report
titles, column headers, operations, rounds, run counts or records per run):
the timing columns are then not comparable where marked "(!)".
"""

import re
import statistics
import sys

REPORT_MARK = "------------> Report "
NUMERIC = re.compile(r"-?[0-9][0-9,]*(\.[0-9]+)?")


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
            prefix = []
            while i < len(lines) and lines[i].strip():
                line = lines[i].rstrip()
                # An operation name may span several lines (an analyzer spec
                # holding newlines): every line but the last of a row lacks
                # the trailing numeric avgTotalMem value.
                if not NUMERIC.fullmatch(line.split()[-1]):
                    prefix.append(line.strip())
                    i += 1
                    continue
                # An operation name longer than the header's Operation column
                # shifts the whole row right (Lucene's Report pads, never
                # truncates): realign on the row's right edge.
                shift = len(line) - len(header.rstrip())
                cut = [e + shift for e in ends]
                # The name may hold spaces ("[...] Name"): it is everything
                # before the second column's value, less the " -" filler of
                # alternate rows.
                toks = line[:cut[1]].split()[:-1] if len(names) > 1 else line.split()
                while toks and toks[-1] == "-":
                    toks.pop()
                row = {names[0]: " ".join(prefix + toks)}
                prefix = []
                for k in range(1, len(names)):
                    cell = line[cut[k - 1]:cut[k]].split()
                    row[names[k]] = cell[-1] if cell else ""
                rows.append(row)
                i += 1
            reports.append((title, names, rows))
        else:
            i += 1
    return reports


SHAPE = ("Operation", "round", "runCnt", "recsPerRun")


def num(s):
    return float(s.replace(",", ""))


def median(cells):
    vals = [num(c) for c in cells if c not in ("", "-")]
    return statistics.median(vals) if vals else None


def fmt(v, digits):
    return "-" if v is None else f"{v:,.{digits}f}"


def ratio(r, l):
    return f"{r / l:.2f}" if r is not None and l else "-"


def shape(row):
    return [row.get(c, "") for c in SHAPE]


def wall(path):
    try:
        with open(path + ".wall", encoding="utf-8") as f:
            return float(f.read().strip())
    except (OSError, ValueError):
        return None


def main():
    args = sys.argv[1:]
    tsv = None
    if args[:1] == ["--tsv"]:
        if len(args) < 2:
            sys.exit("--tsv needs a file")
        tsv, args = args[1], args[2:]
    if "--" in args:
        k = args.index("--")
        lpaths, rpaths = args[:k], args[k + 1:]
    elif len(args) == 2:
        lpaths, rpaths = args[:1], args[1:]
    else:
        lpaths = rpaths = []
    if not lpaths or not rpaths:
        sys.exit("usage: compare.py [--tsv FILE] <lucene.out>... -- <rucene.out>...")

    runs = [(p, parse_reports(p)) for p in lpaths + rpaths]
    for p, reps in runs:
        if not reps:
            sys.exit(f"no report found in {p}")
    nl = len(lpaths)
    ref_path, ref = runs[0]
    diverged = []
    ref_titles = [t for t, _, _ in ref]
    for p, reps in runs[1:]:
        titles = [t for t, _, _ in reps]
        if titles != ref_titles:
            diverged.append(f"report titles differ: {ref_path}={ref_titles} {p}={titles}")
        for (title, names, rows), (_, rnames, _) in zip(ref, reps):
            if names != rnames:
                diverged.append(f"{title}: columns differ: {ref_path}={names} {p}={rnames}")

    cols = ("Operation", "round", "runCnt", "recsPerRun", "L rec/s", "R rec/s",
            "R/L rec/s", "L elapsed", "R elapsed", "R/L elapsed")
    out_tsv = []
    for b, (title, _, ref_rows) in enumerate(ref):
        blocks = [reps[b][2] if b < len(reps) else [] for _, reps in runs]
        print(f"== {title}")
        table = [cols]
        nrows = max(len(rows) for rows in blocks)
        for k in range(nrows):
            cells = [rows[k] if k < len(rows) else None for rows in blocks]
            missing = [runs[i][0] for i, c in enumerate(cells) if c is None]
            if missing:
                diverged.append(f"{title}: row {k} missing in {missing}")
                continue
            first = shape(cells[0])
            odd = [(runs[i][0], shape(c)) for i, c in enumerate(cells) if shape(c) != first]
            for p, sh in odd:
                diverged.append(f"{title}: row {k}: {runs[0][0]} {first} vs {p} {sh}")
            mark = " (!)" if odd else ""
            lc, rc = cells[:nl], cells[nl:]

            def joined(col):
                vals = sorted({c.get(col, "") for c in cells})
                return vals[0] if len(vals) == 1 else "|".join(vals)

            lrate, rrate = median([c.get("rec/s", "") for c in lc]), median([c.get("rec/s", "") for c in rc])
            lel, rel = median([c.get("elapsedSec", "") for c in lc]), median([c.get("elapsedSec", "") for c in rc])
            op = cells[0].get("Operation", "")
            table.append((
                op + mark, joined("round"), joined("runCnt"), joined("recsPerRun"),
                fmt(lrate, 2), fmt(rrate, 2), ratio(rrate, lrate),
                fmt(lel, 2), fmt(rel, 2), ratio(rel, lel),
            ))
            out_tsv.append("\t".join((
                title, op, joined("round"), joined("runCnt"), joined("recsPerRun"),
                fmt(lrate, 2), fmt(rrate, 2), fmt(lel, 2), fmt(rel, 2), ratio(rel, lel),
                "1" if odd else "0",
            )))
        widths = [max(len(r[c]) for r in table) for c in range(len(cols))]
        for r in table:
            print("  ".join(
                r[c].ljust(widths[c]) if c == 0 else r[c].rjust(widths[c])
                for c in range(len(cols))
            ))
        print()
    lw = [w for w in (wall(p) for p in lpaths) if w is not None]
    rw = [w for w in (wall(p) for p in rpaths) if w is not None]
    if lw and rw:
        lm, rm = statistics.median(lw), statistics.median(rw)
        print(f"Total (process wall-clock, s): L {lm:.2f}  R {rm:.2f}  R/L {rm / lm:.2f}")
        out_tsv.append("\t".join(("TOTAL", "wall", "", "", "", "", "", f"{lm:.2f}",
                                   f"{rm:.2f}", f"{rm / lm:.2f}", "0")))
    if nl > 1 or len(rpaths) > 1:
        print(f"Medians over {nl} Lucene and {len(rpaths)} Rucene runs.")
    print("L = Lucene 10.5.0, R = Rucene; R/L rec/s > 1 and R/L elapsed < 1 mean Rucene is faster.")
    if tsv:
        with open(tsv, "w", encoding="utf-8") as f:
            f.write("report\toperation\tround\trunCnt\trecsPerRun\tL_rec_s\tR_rec_s"
                    "\tL_elapsed\tR_elapsed\tRL_elapsed\tshape_diverges\n")
            f.write("".join(line + "\n" for line in out_tsv))
    if diverged:
        print("\nREPORT SHAPE DIVERGES (timings not comparable where marked (!)):")
        for d in diverged:
            print(f"  {d}")
        sys.exit(2)


if __name__ == "__main__":
    main()
