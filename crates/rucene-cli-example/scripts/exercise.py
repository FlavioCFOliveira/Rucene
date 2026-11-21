#!/usr/bin/env python3
"""End-to-end exercise of rucene-cli against an independent Python model.

Usage::

    python3 -I exercise.py --bin PATH --data CSV --workdir DIR [--seed N]

The script builds an index from the USGS CSV through every write path of the
CLI (``create``; ``add`` by ``--set`` and by CSV; ``update`` by ``--set`` and
by CSV; ``delete`` of one id and of many ids in one call). Every write is
preceded and followed by a probe battery, and the run ends with a query
battery over every field kind, combined boolean queries and multi-key sorts.

Every answer is checked against a model of the expected index content that is
computed from the CSV in Python, never from CLI output:

- ``# total hits`` must equal the model count exactly: every search passes
  ``--exact-total`` and the line must carry no ``(lower bound)`` suffix;
- the full id set is compared when the model count is at most 2,000;
- sorted queries request 500 hits (2,000 when the whole result fits) and the
  per-position sort keys must equal the model's (ties may be in any order);
- relevance-ordered queries must have non-increasing scores;
- after each write, every stored column of sampled ids is compared.

The run is deterministic for a given ``--seed``. It stops with exit status 1
and a precise message at the first failed check, and ends with
``ALL CHECKS PASSED: N assertions`` otherwise.

Only the Python standard library is used.
"""

from __future__ import annotations

import argparse
import csv
import math
import os
import random
import re
import shutil
import subprocess
import sys
import time
import unicodedata
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from decimal import Decimal
from pathlib import Path
from typing import Callable, Iterable, Sequence

KEYWORD = {"id", "magType", "net", "type", "status", "locationSource", "magSource"}
TEXT = {"place"}
DATE = {"time", "updated"}
DOUBLE = {
    "latitude",
    "longitude",
    "depth",
    "mag",
    "gap",
    "dmin",
    "rms",
    "horizontalError",
    "depthError",
    "magError",
}
INT = {"nst", "magNst"}
EARTH_R = 6371008.7714  # org.apache.lucene.geo.GeoUtils.EARTH_MEAN_RADIUS_METERS
EPOCH = datetime(1970, 1, 1, tzinfo=timezone.utc)
MARKER = ".rucene-exercise"
FULL_SET_MAX = 2000  # id sets are compared up to this many hits
SORTED_TOP = 500  # sorted queries check this many leading positions
GEO_MARGIN_M = 1.0  # no point may lie this close to a radius
GEO_MARGIN_DEG = 1e-5  # ~1.1 m: no point may lie this close to a box edge
# A whitespace-delimited chunk of a place tokenises identically under Python's
# \w+ and StandardAnalyzer (UAX #29) when it holds only letters and digits
# (no combining marks), separators that always break words (including the
# '?' USGS writes for unknown characters), a period that ends a word, or a
# period inside a number (it only changes the number's own token). Apostrophes
# (U+0027, U+2018, U+2019) are not safe: UAX #29 joins letters across them.
# UAX #29 never joins across whitespace, so the check is per chunk.
SAFE_CHUNK = re.compile(r"^(?:[^\W_]|[,()\[\]?\-/]|\.(?=,|$)|(?<=\d)\.(?=\d))*$")
WORD = re.compile(r"\w+")

GNU_TIME = "/usr/bin/time" if os.access("/usr/bin/time", os.X_OK) else None
Row = list  # typed column values in HEADER order (None for an empty value)


class CheckFailed(Exception):
    """A check did not hold; the message says what and where."""


def to_ms(iso: str) -> int:
    """ISO 8601 (with Z or offset, or none = UTC) to epoch milliseconds, exactly."""
    dt = datetime.fromisoformat(iso.replace("Z", "+00:00"))
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return (dt - EPOCH) // timedelta(milliseconds=1)


def parse_value(field_name: str, raw: str) -> object:
    """Typed model value of one CSV cell, or None when it is empty."""
    if raw == "":
        return None
    if field_name in DATE:
        return to_ms(raw)
    if field_name in DOUBLE:
        return float(raw)
    if field_name in INT:
        return int(raw)
    return raw


def tokens(text: str) -> list[str]:
    return WORD.findall(text.lower())


def haversine(lat1: float, lon1: float, lat2: float, lon2: float) -> float:
    p1, p2 = math.radians(lat1), math.radians(lat2)
    dp, dl = p2 - p1, math.radians(lon2 - lon1)
    a = math.sin(dp / 2) ** 2 + math.cos(p1) * math.cos(p2) * math.sin(dl / 2) ** 2
    return 2 * EARTH_R * math.asin(math.sqrt(a))


@dataclass
class StepStats:
    """Wall time and peak RSS of the CLI processes of one step."""

    name: str
    index_s: float = 0.0
    index_rss_kb: int = 0
    search_s: float = 0.0
    search_n: int = 0
    search_rss_kb: int = 0
    checks: int = 0


@dataclass
class Exercise:
    binary: Path
    workdir: Path
    header: list[str]
    model: dict[str, Row]
    rng: random.Random
    checks: int = 0
    steps: list[StepStats] = field(default_factory=list)
    token_cache: dict[str, tuple[str, ...]] = field(default_factory=dict)

    def __post_init__(self) -> None:
        self.fi = {name: i for i, name in enumerate(self.header)}
        self.idx = self.workdir / "idx"
        self.out_path = self.workdir / "stdout.txt"
        self.err_path = self.workdir / "stderr.txt"
        self.rss_path = self.workdir / "rss.txt"

    # ----------------------------------------------------------------- basics

    def check(self, cond: bool, what: str) -> None:
        self.checks += 1
        self.steps[-1].checks += 1
        if not cond:
            raise CheckFailed(what)

    def val(self, row: Row, name: str) -> object:
        return row[self.fi[name]]

    def place_tokens(self, row: Row) -> tuple[str, ...]:
        place = row[self.fi["place"]] or ""
        cached = self.token_cache.get(place)
        if cached is None:
            cached = tuple(tokens(place))
            self.token_cache[place] = cached
        return cached

    def step(self, name: str) -> None:
        self.steps.append(StepStats(name))
        print(f"== {name}", flush=True)

    def run(self, args: Sequence[str], kind: str) -> str:
        """Run the CLI; kind is 'index' or 'search'. Returns stdout.

        Peak RSS is read from GNU time (``/usr/bin/time -f %M``) when it is
        installed: the rusage that os.wait4 reports for a child spawned from
        this process also counts this process's own memory, inherited at
        fork/exec time, so it cannot measure the CLI alone.
        """
        cmd = [str(self.binary), *args]
        full = [GNU_TIME, "-f", "%M", "-o", str(self.rss_path), *cmd] if GNU_TIME else cmd
        with self.out_path.open("wb") as so, self.err_path.open("wb") as se:
            t0 = time.perf_counter()
            proc = subprocess.run(full, stdout=so, stderr=se, check=False)
            elapsed = time.perf_counter() - t0
        rss_kb = 0
        if GNU_TIME:
            last = self.rss_path.read_text(encoding="utf-8").strip().splitlines()[-1:]
            rss_kb = int(last[0]) if last and last[0].isdigit() else 0
        st = self.steps[-1]
        if kind == "index":
            st.index_s += elapsed
            st.index_rss_kb = max(st.index_rss_kb, rss_kb)
        else:
            st.search_s += elapsed
            st.search_n += 1
            st.search_rss_kb = max(st.search_rss_kb, rss_kb)
        out = self.out_path.read_text(encoding="utf-8")
        if proc.returncode != 0:
            err = self.err_path.read_text(encoding="utf-8")
            raise CheckFailed(
                f"exit status {proc.returncode}: {shown(cmd)}\nstderr: {err.strip()}"
            )
        return out

    def search(
        self, args: Sequence[str], limit: int, fields: str = "id"
    ) -> tuple[int, list[dict[str, str]], list[str]]:
        cmd = ["search", str(self.idx), *args, "--limit", str(limit), "--fields", fields,
               "--exact-total"]
        out = self.run(cmd, "search")
        lines = out.rstrip("\n").split("\n")
        m = re.fullmatch(r"# total hits: (\d+)(.*)", lines[0])
        self.check(m is not None, f"{shown(cmd)}: unexpected first line {lines[0]!r}")
        assert m is not None
        self.check(m.group(2) == "", f"{shown(cmd)}: total is not exact: {lines[0]!r}")
        head = lines[1].split("\t")
        hits = [dict(zip(head, line.split("\t"))) for line in lines[2:]]
        self.check(
            all(len(h) == len(head) for h in hits), f"{shown(cmd)}: ragged TSV rows"
        )
        return int(m.group(1)), hits, cmd

    # ----------------------------------------------------------- expectations

    def sorted_ids(self, ids: list[str], sort: Sequence[tuple[str, bool]]) -> list[str]:
        """Model order: stable passes from the last key to the first; for each
        key, present values ascending or descending, missing values last."""
        out = list(ids)
        for name, desc in reversed(sort):
            i = self.fi[name]
            present = [x for x in out if self.model[x][i] is not None]
            missing = [x for x in out if self.model[x][i] is None]
            present.sort(key=lambda x: self.model[x][i], reverse=desc)
            out = present + missing
        return out

    def expect(
        self,
        desc: str,
        args: Sequence[str],
        pred: Callable[[Row], bool],
        sort: Sequence[tuple[str, bool]] | None = None,
    ) -> int:
        """Run a search and compare it with the model rows that satisfy pred."""
        exp = [i for i, r in self.model.items() if pred(r)]
        n = len(exp)
        full = n <= FULL_SET_MAX
        limit = FULL_SET_MAX if full else SORTED_TOP
        sort_args: list[str] = []
        if sort:
            sort_args = ["--sort", ",".join(f"{f}:{'desc' if d else 'asc'}" for f, d in sort)]
        total, hits, cmd = self.search([*args, *sort_args], limit)
        where = f"{desc}\n  command: {shown(cmd)}"
        self.check(total == n, f"{where}\n  total hits {total} != model {n}")
        self.check(
            len(hits) == min(n, limit),
            f"{where}\n  {len(hits)} rows returned, expected {min(n, limit)}",
        )
        got = [h["id"] for h in hits]
        self.check(len(set(got)) == len(got), f"{where}\n  duplicate ids in the hits")
        if full:
            missing = sorted(set(exp) - set(got))[:5]
            extra = sorted(set(got) - set(exp))[:5]
            self.check(
                not missing and not extra,
                f"{where}\n  id set differs: missing {missing}, unexpected {extra}",
            )
        else:
            unknown = [i for i in got if i not in self.model or not pred(self.model[i])][:5]
            self.check(not unknown, f"{where}\n  hits outside the model result: {unknown}")
        if sort:
            order = self.sorted_ids(exp, sort)[: len(got)]
            exp_keys = [tuple(self.val(self.model[i], f) for f, _ in sort) for i in order]
            got_keys = [tuple(self.val(self.model[i], f) for f, _ in sort) for i in got]
            for pos, (g, e) in enumerate(zip(got_keys, exp_keys)):
                self.check(
                    g == e,
                    f"{where}\n  sort key at position {pos} ({got[pos]}): {g} != model {e}",
                )
        else:
            scores = [float(h["score"]) for h in hits]
            for pos in range(1, len(scores)):
                self.check(
                    scores[pos] <= scores[pos - 1],
                    f"{where}\n  score increases at position {pos}: "
                    f"{scores[pos - 1]} then {scores[pos]}",
                )
        print(f"  ok  {desc}: {total} hits", flush=True)
        return total

    def expect_distance_sort(self, lat: float, lon: float, km: float) -> None:
        pred = self.near(lat, lon, km)
        exp = sorted(
            (haversine(lat, lon, self.val(r, "latitude"), self.val(r, "longitude")), i)
            for i, r in self.model.items()
            if pred(r)
        )
        n = len(exp)
        limit = FULL_SET_MAX if n <= FULL_SET_MAX else SORTED_TOP
        args = ["--distance", f"{lat},{lon},{km}", "--sort", "_distance"]
        total, hits, cmd = self.search(args, limit)
        where = f"distance sort {lat},{lon},{km}km\n  command: {shown(cmd)}"
        self.check(total == n, f"{where}\n  total hits {total} != model {n}")
        self.check(len(hits) == min(n, limit), f"{where}\n  {len(hits)} rows returned")
        col = "sort:_distance(m):asc"
        dists = [float(h[col]) for h in hits]
        for pos, h in enumerate(hits):
            r = self.model[h["id"]]
            mine = haversine(lat, lon, self.val(r, "latitude"), self.val(r, "longitude"))
            self.check(
                abs(dists[pos] - exp[pos][0]) < GEO_MARGIN_M,
                f"{where}\n  position {pos}: {dists[pos]} m, model {exp[pos][0]} m",
            )
            self.check(
                abs(dists[pos] - mine) < GEO_MARGIN_M,
                f"{where}\n  {h['id']}: printed {dists[pos]} m, model {mine} m",
            )
            if pos:
                self.check(dists[pos] >= dists[pos - 1], f"{where}\n  not ascending at {pos}")
        if n <= FULL_SET_MAX:
            self.check(
                {h["id"] for h in hits} == {i for _, i in exp}, f"{where}\n  id set differs"
            )
        print(f"  ok  distance sort {lat},{lon},{km}km: {total} hits", flush=True)

    def verify_stored(self, ids: Iterable[str]) -> None:
        for i in ids:
            total, hits, cmd = self.search(["--id", i], 10, fields="all")
            self.check(total == 1, f"stored fields of {i}: {total} hits\n  command: {shown(cmd)}")
            h, r = hits[0], self.model[i]
            for name in self.header:
                e, g = self.val(r, name), parse_value(name, h.get(name, ""))
                self.check(
                    g == e, f"stored {i}.{name}: CLI {h.get(name, '')!r} != model {e!r}"
                )

    def verify_absent(self, ids: Iterable[str]) -> None:
        for i in ids:
            total, _, cmd = self.search(["--id", i], 10)
            self.check(total == 0, f"deleted {i} still found\n  command: {shown(cmd)}")

    def sample_ids(self, k: int) -> list[str]:
        keys = sorted(self.model)
        return self.rng.sample(keys, min(k, len(keys)))

    # -------------------------------------------------------------- predicates

    def has_words(self, words: str) -> Callable[[Row], bool]:
        want = set(tokens(words))
        self.assert_text_safe(want)
        return lambda r: want <= set(self.place_tokens(r))

    def has_phrase(self, words: str) -> Callable[[Row], bool]:
        want = tuple(tokens(words))
        self.assert_text_safe(set(want))
        k = len(want)

        def pred(r: Row) -> bool:
            t = self.place_tokens(r)
            return any(t[j : j + k] == want for j in range(len(t) - k + 1))

        return pred

    def assert_text_safe(self, words: set[str]) -> None:
        """Every whitespace-delimited chunk of an indexed place that contains a
        query word as a substring must tokenise the same way under \\w+ and
        StandardAnalyzer, so the model's token match is exact."""
        places = {self.val(r, "place") or "" for r in self.model.values()}
        for p in places:
            for chunk in p.split():
                low = chunk.lower()
                if not any(w in low for w in words):
                    continue
                safe = SAFE_CHUNK.match(chunk) and all(
                    len(c.lower()) == 1 and not unicodedata.category(c).startswith("M")
                    for c in chunk
                )
                self.check(
                    bool(safe),
                    f"query words {sorted(words)}: chunk {chunk!r} of place {p!r} is not text-safe",
                )

    def rng_pred(self, name: str, lo: object = None, hi: object = None) -> Callable[[Row], bool]:
        i = self.fi[name]

        def pred(r: Row) -> bool:
            v = r[i]
            return v is not None and (lo is None or v >= lo) and (hi is None or v <= hi)

        return pred

    def term(self, name: str, *values: object) -> Callable[[Row], bool]:
        i, s = self.fi[name], set(values)
        return lambda r: r[i] in s

    def box(self, minlat: float, maxlat: float, minlon: float, maxlon: float) -> Callable[[Row], bool]:
        """LatLonPoint.newBoxQuery semantics; minlon > maxlon crosses the date line."""
        la, lo, m = self.fi["latitude"], self.fi["longitude"], GEO_MARGIN_DEG
        crosses = minlon > maxlon

        def lon_in(lon: float, margin: float) -> bool:
            if crosses:
                return lon >= minlon - margin or lon <= maxlon + margin
            return minlon - margin <= lon <= maxlon + margin

        for r in self.model.values():
            lat, lon = r[la], r[lo]
            if lat is None or lon is None:
                continue
            near_lat = abs(lat - minlat) < m or abs(lat - maxlat) < m
            near_lon = abs(lon - minlon) < m or abs(lon - maxlon) < m
            self.check(
                not ((near_lat and lon_in(lon, m)) or (near_lon and minlat - m <= lat <= maxlat + m)),
                f"box {minlat},{maxlat},{minlon},{maxlon}: point {lat},{lon} "
                f"lies within {m} degrees of an edge",
            )
        return lambda r: (
            r[la] is not None
            and r[lo] is not None
            and minlat <= r[la] <= maxlat
            and lon_in(r[lo], 0.0)
        )

    def near(self, lat: float, lon: float, km: float) -> Callable[[Row], bool]:
        la, lo, radius = self.fi["latitude"], self.fi["longitude"], km * 1000.0
        dist: dict[int, float] = {}
        for r in self.model.values():
            if r[la] is None or r[lo] is None:
                continue
            d = haversine(lat, lon, r[la], r[lo])
            dist[id(r)] = d
            self.check(
                abs(d - radius) >= GEO_MARGIN_M,
                f"distance {lat},{lon},{km}km: point {r[la]},{r[lo]} lies "
                f"{abs(d - radius):.3f} m from the radius",
            )
        return lambda r: id(r) in dist and dist[id(r)] <= radius

    # ------------------------------------------------------------------ probes

    def probes(self, tag: str) -> None:
        """The battery run before and after every write."""
        t0, t1 = to_ms("2019-07-06T03:00"), to_ms("2019-07-06T04:00")
        self.expect(f"[{tag}] match-all", [], lambda r: True)
        self.expect(f"[{tag}] term net=ak", ["--term", "net=ak"], self.term("net", "ak"))
        self.expect(
            f"[{tag}] range mag=4.5.. sort mag:desc",
            ["--range", "mag=4.5.."],
            self.rng_pred("mag", 4.5),
            sort=[("mag", True)],
        )
        self.expect(f"[{tag}] text nevada", ["--text", "nevada"], self.has_words("nevada"))
        self.expect(
            f"[{tag}] range time (1 h, Ridgecrest) sort time:asc",
            ["--range", "time=2019-07-06T03:00..2019-07-06T04:00"],
            self.rng_pred("time", t0, t1),
            sort=[("time", False)],
        )


# ---------------------------------------------------------------------- helpers


def shown(cmd: Sequence[str]) -> str:
    text = " ".join(f"'{a}'" if (" " in a or a == "") else a for a in cmd)
    return text if len(text) <= 600 else text[:600] + f" ... ({len(cmd)} arguments)"


def write_csv(path: Path, header: list[str], rows: Iterable[list[str]]) -> None:
    with path.open("w", encoding="utf-8", newline="") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow(header)
        w.writerows(rows)


def set_args(header: list[str], raw: list[str]) -> list[str]:
    args: list[str] = []
    for name, value in zip(header, raw):
        if value != "":
            args += ["--set", f"{name}={value}"]
    return args


def prepare_workdir(workdir: Path) -> None:
    """The script owns DIR: it must be new, empty, or a previous exercise dir."""
    if workdir.exists():
        if any(workdir.iterdir()) and not (workdir / MARKER).exists():
            sys.exit(f"error: {workdir} is not empty and is not an exercise directory")
        shutil.rmtree(workdir)
    workdir.mkdir(parents=True)
    (workdir / MARKER).write_text("rucene-cli exercise work directory\n", encoding="utf-8")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--bin", required=True, type=Path, help="the rucene-cli binary")
    ap.add_argument("--data", required=True, type=Path, help="the USGS CSV")
    ap.add_argument("--workdir", required=True, type=Path, help="scratch directory (replaced)")
    ap.add_argument("--seed", type=int, default=20190101, help="sampling seed")
    a = ap.parse_args()

    started = time.perf_counter()
    prepare_workdir(a.workdir)
    csvdir = a.workdir / "csv"
    csvdir.mkdir()

    # Pass 1: the typed model of every row, in file order.
    t = time.perf_counter()
    with a.data.open(encoding="utf-8", newline="") as f:
        reader = csv.reader(f)
        header = next(reader)
        typed: list[Row] = []
        intern_cols = {i for i, h in enumerate(header) if h in KEYWORD | TEXT and h != "id"}
        for lineno, raw in enumerate(reader, start=2):
            if len(raw) != len(header):
                sys.exit(f"error: {a.data}:{lineno}: {len(raw)} fields")
            typed.append(
                [
                    sys.intern(v) if i in intern_cols and v else parse_value(h, v)
                    for i, (h, v) in enumerate(zip(header, raw))
                ]
            )
    n = len(typed)
    i_id = header.index("id")
    ids = [r[i_id] for r in typed]
    if len(set(ids)) != n:
        sys.exit("error: the CSV has duplicate ids; run scripts/fetch_dataset.sh")
    print(f"loaded {n} rows from {a.data} in {time.perf_counter() - t:.1f} s", flush=True)

    # The write plan, by row position.
    half = n // 2
    rest = list(range(half + 1, n))
    rest_batches = [rest[k::4] for k in range(4)]  # 4 batches, interleaved order kept
    rest_batches = [sorted(b) for b in rest_batches]
    rng = random.Random(a.seed)
    upd_one = ids[rng.randrange(1, n)]
    n_upd = min(5000, n // 20)  # 5,000 on the full dataset
    n_del = min(200, n // 10)  # 200 on the full dataset
    upd_many = rng.sample(sorted(set(ids) - {upd_one, ids[0], ids[half]}), n_upd)
    del_one = ids[rng.randrange(1, n)]
    while del_one == upd_one or del_one in set(upd_many):
        del_one = ids[rng.randrange(1, n)]
    upd_set = set(upd_many) | {upd_one, del_one}
    del_many = (
        [upd_one]
        + upd_many[: n_del // 4 - 1]
        + rng.sample(sorted(set(ids) - upd_set), n_del - n_del // 4)
    )
    needed_raw = {upd_one, del_one, *upd_many, ids[0]}

    # Pass 2: write the add batches with the original CSV text.
    raw_of: dict[str, list[str]] = {}
    files = {
        "big": csvdir / "add-big.csv",
        "one": csvdir / "add-one.csv",
        **{f"rest{k}": csvdir / f"add-rest-{k}.csv" for k in range(4)},
    }
    where: dict[int, str] = {p: "big" for p in range(1, half)}
    where[half] = "one"
    for k, b in enumerate(rest_batches):
        for p in b:
            where[p] = f"rest{k}"
    handles = {k: p.open("w", encoding="utf-8", newline="") for k, p in files.items()}
    writers = {k: csv.writer(h, lineterminator="\n") for k, h in handles.items()}
    counts = dict.fromkeys(files, 0)
    with a.data.open(encoding="utf-8", newline="") as f:
        reader = csv.reader(f)
        next(reader)
        for w in writers.values():
            w.writerow(header)
        for pos, raw in enumerate(reader):
            if raw[i_id] in needed_raw:
                raw_of[raw[i_id]] = raw
            dest = where.get(pos)
            if dest is not None:
                writers[dest].writerow(raw)
                counts[dest] += 1
    for h in handles.values():
        h.close()
    print(f"batches: {counts} (+1 --set document)", flush=True)

    ex = Exercise(a.bin, a.workdir, header, {}, random.Random(a.seed + 1))
    idx = str(ex.idx)
    model = ex.model

    def add_positions(positions: Iterable[int]) -> list[str]:
        out = []
        for p in positions:
            model[ids[p]] = typed[p]
            out.append(ids[p])
        return out

    try:
        ex.step("1. create")
        ex.run(["create", idx], "index")
        ex.probes("after create")

        ex.step("2. add one document (--set)")
        ex.probes("before")
        ex.run(["add", idx, *set_args(header, raw_of[ids[0]])], "index")
        add_positions([0])
        ex.probes("after")
        ex.verify_stored([ids[0]])

        ex.step(f"3. add {counts['big']} documents (CSV)")
        ex.probes("before")
        ex.run(["add", idx, "--file", str(files["big"])], "index")
        add_positions(range(1, half))
        ex.probes("after")
        ex.verify_stored(ex.sample_ids(20))

        ex.step("4. add one document (1-row CSV)")
        ex.probes("before")
        ex.run(["add", idx, "--file", str(files["one"])], "index")
        add_positions([half])
        ex.probes("after")
        ex.verify_stored([ids[half]])

        for k, b in enumerate(rest_batches):
            ex.step(f"5.{k + 1} add {len(b)} documents (CSV batch {k + 1} of 4)")
            ex.probes("before")
            ex.run(["add", idx, "--file", str(files[f"rest{k}"])], "index")
            added = add_positions(b)
            ex.probes("after")
            ex.verify_stored(ex.rng.sample(added, min(10, len(added))) + ex.sample_ids(10))
        ex.check(len(model) == n, f"model holds {len(model)} of {n} rows")

        ex.step("6. update one document (--set)")
        u = list(raw_of[upd_one])
        u[header.index("place")] = "Exercise Zyxwplace, Testland"
        u[header.index("mag")] = "9.87"
        word = ex.has_words("zyxwplace")
        ex.probes("before")
        ex.expect("[before] text zyxwplace", ["--text", "zyxwplace"], word)
        ex.run(["update", idx, *set_args(header, u)], "index")
        model[upd_one] = [parse_value(h, v) for h, v in zip(header, u)]
        ex.probes("after")
        ex.expect("[after] text zyxwplace", ["--text", "zyxwplace"], ex.has_words("zyxwplace"))
        ex.expect("[after] phrase 'zyxwplace testland'", ["--phrase", "zyxwplace testland"],
                  ex.has_phrase("zyxwplace testland"))
        ex.expect("[after] range mag=9.5..", ["--range", "mag=9.5.."], ex.rng_pred("mag", 9.5))
        ex.verify_stored([upd_one])

        ex.step(f"7. update {len(upd_many)} documents (CSV: mag+1, status=revised)")
        i_mag, i_status = header.index("mag"), header.index("status")
        new_rows = []
        for i in upd_many:
            r = list(raw_of[i])
            r[i_mag] = str(Decimal(r[i_mag]) + 1) if r[i_mag] else "1"
            r[i_status] = "revised"
            new_rows.append(r)
        upath = csvdir / "update-many.csv"
        write_csv(upath, header, new_rows)
        revised = ex.term("status", "revised")
        ex.probes("before")
        ex.expect("[before] term status=revised", ["--term", "status=revised"], revised)
        ex.run(["update", idx, "--file", str(upath)], "index")
        for r in new_rows:
            model[r[i_id]] = [parse_value(h, v) for h, v in zip(header, r)]
        ex.probes("after")
        ex.expect("[after] term status=revised", ["--term", "status=revised"], revised)
        ex.expect(
            "[after] term status=revised sort mag:desc,id:asc",
            ["--term", "status=revised"],
            revised,
            sort=[("mag", True), ("id", False)],
        )
        ex.verify_stored(ex.rng.sample(upd_many, min(20, len(upd_many))))

        ex.step("8. delete one document")
        ex.probes("before")
        ex.expect(f"[before] id {del_one}", ["--id", del_one], ex.term("id", del_one))
        ex.run(["delete", idx, "--id", del_one], "index")
        del model[del_one]
        ex.probes("after")
        ex.expect(f"[after] id {del_one}", ["--id", del_one], ex.term("id", del_one))
        ex.verify_absent([del_one])
        ex.verify_stored(ex.sample_ids(10))

        ex.step(f"9. delete {len(del_many)} documents (one call, incl. updated ones)")
        in_del = ex.term("id", *del_many)
        terms_arg = ["--terms", "id=" + ",".join(del_many)]
        ex.probes("before")
        ex.expect("[before] terms id=<deleted set>", terms_arg, in_del)
        ex.expect("[before] term status=revised", ["--term", "status=revised"], revised)
        ex.run(["delete", idx, *sum((["--id", i] for i in del_many), [])], "index")
        for i in del_many:
            del model[i]
        ex.probes("after")
        ex.expect("[after] terms id=<deleted set>", terms_arg, in_del)
        ex.expect("[after] term status=revised", ["--term", "status=revised"], revised)
        ex.expect("[after] text zyxwplace", ["--text", "zyxwplace"], ex.has_words("zyxwplace"))
        ex.verify_absent(del_many[:20])
        ex.verify_stored(ex.sample_ids(20))

        ex.step("10. final query battery")
        final_battery(ex)
    except CheckFailed as e:
        print(f"\nFAIL in step '{ex.steps[-1].name}': {e}", file=sys.stderr)
        print(f"({ex.checks} checks passed before the failure)", file=sys.stderr)
        return 1

    if not GNU_TIME:
        print("\n(/usr/bin/time not found: peak RSS columns are 0)")
    print("\nstep | index s | index peak RSS MiB | searches | search s | search peak RSS MiB | checks")
    for s in ex.steps:
        print(
            f"{s.name} | {s.index_s:.2f} | {s.index_rss_kb / 1024:.0f} | {s.search_n} | "
            f"{s.search_s:.2f} | {s.search_rss_kb / 1024:.0f} | {s.checks}"
        )
    tot_i = sum(s.index_s for s in ex.steps)
    tot_s = sum(s.search_s for s in ex.steps)
    print(
        f"\nindex wall {tot_i:.1f} s; {sum(s.search_n for s in ex.steps)} searches, "
        f"wall {tot_s:.1f} s; total run {time.perf_counter() - started:.1f} s; "
        f"final index holds {len(model)} documents"
    )
    print(f"ALL CHECKS PASSED: {ex.checks} assertions")
    return 0


def final_battery(ex: Exercise) -> None:
    rp, term = ex.rng_pred, ex.term
    t0, t1 = to_ms("2024-01-01T07:00"), to_ms("2024-01-01T09:00")
    y0, y1 = to_ms("2021-01-01"), to_ms("2021-12-31T23:59:59.999")

    # Every field kind alone (relevance order).
    ex.expect("text 'puerto rico'", ["--text", "puerto rico"], ex.has_words("puerto rico"))
    ex.expect("text 'papua new guinea'", ["--text", "papua new guinea"], ex.has_words("papua new guinea"))
    ex.expect("phrase 'puerto rico'", ["--phrase", "puerto rico"], ex.has_phrase("puerto rico"))
    ex.expect("phrase 'new guinea'", ["--phrase", "new guinea"], ex.has_phrase("new guinea"))
    ex.expect("phrase 'the geysers'", ["--phrase", "the geysers"], ex.has_phrase("the geysers"))
    ex.expect("term magType=mww", ["--term", "magType=mww"], term("magType", "mww"))
    ex.expect("term type='quarry blast'", ["--term", "type=quarry blast"], term("type", "quarry blast"))
    ex.expect("term mag=5.5 (double)", ["--term", "mag=5.5"], term("mag", 5.5))
    ex.expect("term nst=100 (int)", ["--term", "nst=100"], term("nst", 100))
    ex.expect("terms net=pr,tx,ok", ["--terms", "net=pr,tx,ok"], term("net", "pr", "tx", "ok"))
    ex.expect("terms magType=mwr,mh,mb_lg", ["--terms", "magType=mwr,mh,mb_lg"],
              term("magType", "mwr", "mh", "mb_lg"))
    ex.expect("range mag=6.5..", ["--range", "mag=6.5.."], rp("mag", 6.5))
    ex.expect("range depth=..-2", ["--range", "depth=..-2"], rp("depth", hi=-2.0))
    ex.expect("range nst=150..200", ["--range", "nst=150..200"], rp("nst", 150, 200))
    ex.expect("range time 2024-01-01T07:00..09:00", ["--range", "time=2024-01-01T07:00..2024-01-01T09:00"],
              rp("time", t0, t1))
    ex.expect("range time year 2021", ["--range", "time=2021-01-01..2021-12-31T23:59:59.999"], rp("time", y0, y1))
    ex.expect("box California 32,42,-125,-114", ["--box", "32.123,42.123,-125.123,-114.123"],
              ex.box(32.123, 42.123, -125.123, -114.123))
    ex.expect("box small (Noto) 37.0,37.6,136.6,137.4", ["--box", "37.0123,37.6123,136.6123,137.4123"],
              ex.box(37.0123, 37.6123, 136.6123, 137.4123))
    ex.expect("box across the date line -25,-10,175,-170", ["--box", "-25.0123,-10.0123,175.0123,-170.0123"],
              ex.box(-25.0123, -10.0123, 175.0123, -170.0123))
    ex.expect("distance Ridgecrest 35.77,-117.6,20.725km", ["--distance", "35.77,-117.6,20.725"],
              ex.near(35.77, -117.6, 20.725))
    ex.expect("distance Anchorage 61.2,-149.9,7km", ["--distance", "61.2,-149.9,7"],
              ex.near(61.2, -149.9, 7))

    # Combined queries.
    ex.expect(
        "MUST text alaska + range mag=3.. + NOT status=automatic",
        ["--text", "alaska", "--range", "mag=3..", "--not", "--term", "status=automatic"],
        lambda r, w=ex.has_words("alaska"), m=rp("mag", 3.0): w(r) and m(r) and r[ex.fi["status"]] != "automatic",
    )
    ex.expect(
        "MUST net=ci + box + date range + NOT type='quarry blast'",
        ["--term", "net=ci", "--box", "33.01371,36.01371,-119.01371,-115.01371",
         "--range", "time=2021-01-01..2021-12-31T23:59:59.999", "--not", "--term", "type=quarry blast"],
        lambda r, b=ex.box(33.01371, 36.01371, -119.01371, -115.01371), y=rp("time", y0, y1):
            r[ex.fi["net"]] == "ci" and b(r) and y(r) and r[ex.fi["type"]] != "quarry blast",
    )
    ex.expect(
        "MUST phrase 'puerto rico' + range depth=10..50 + NOT magType=md",
        ["--phrase", "puerto rico", "--range", "depth=10..50", "--not", "--term", "magType=md"],
        lambda r, p=ex.has_phrase("puerto rico"), d=rp("depth", 10.0, 50.0): p(r) and d(r) and r[ex.fi["magType"]] != "md",
    )
    ex.expect(
        "MUST distance + terms type + date range",
        ["--distance", "35.77,-117.6,20.725", "--terms", "type=earthquake,explosion",
         "--range", "time=2019-07-06..2019-07-07"],
        lambda r, d=ex.near(35.77, -117.6, 20.725), y=rp("time", to_ms("2019-07-06"), to_ms("2019-07-07")):
            d(r) and r[ex.fi["type"]] in {"earthquake", "explosion"} and y(r),
    )
    ex.expect(
        "MUST net=hv + SHOULD text kilauea (SHOULD only scores)",
        ["--term", "net=hv", "--should", "--text", "kilauea"],
        term("net", "hv"),
    )
    ex.expect(
        "SHOULD-only: text kermadec | net=pr | mag>=7",
        ["--should", "--text", "kermadec", "--should", "--term", "net=pr", "--should", "--range", "mag=7.."],
        lambda r, w=ex.has_words("kermadec"), m=rp("mag", 7.0): w(r) or r[ex.fi["net"]] == "pr" or m(r),
    )
    ex.expect(
        "NOT-only: NOT net=ak, NOT depth<=5",
        ["--not", "--term", "net=ak", "--not", "--range", "depth=..5"],
        lambda r, d=rp("depth", hi=5.0): r[ex.fi["net"]] != "ak" and not d(r),
    )
    ex.expect(
        "NOT-only: NOT text alaska",
        ["--not", "--text", "alaska"],
        lambda r, w=ex.has_words("alaska"): not w(r),
    )

    # Sorts by one key.
    ex.expect("all sort mag:desc", [], lambda r: True, sort=[("mag", True)])
    ex.expect("all sort mag:asc (missing last)", [], lambda r: True, sort=[("mag", False)])
    ex.expect("all sort time:asc", [], lambda r: True, sort=[("time", False)])
    ex.expect("all sort updated:desc", [], lambda r: True, sort=[("updated", True)])
    ex.expect("all sort depth:desc", [], lambda r: True, sort=[("depth", True)])
    ex.expect("all sort gap:asc (missing last)", [], lambda r: True, sort=[("gap", False)])
    ex.expect("all sort gap:desc (missing last)", [], lambda r: True, sort=[("gap", True)])
    ex.expect("all sort magType:asc (keyword)", [], lambda r: True, sort=[("magType", False)])
    ex.expect("all sort magType:desc (keyword, missing last)", [], lambda r: True, sort=[("magType", True)])
    ex.expect("all sort place:asc (place_exact)", [], lambda r: True, sort=[("place", False)])
    ex.expect("all sort nst:desc (int)", [], lambda r: True, sort=[("nst", True)])
    ex.expect("range mag=6.. sort time:desc", ["--range", "mag=6.."], rp("mag", 6.0), sort=[("time", True)])

    # Sorts by two and three keys.
    ex.expect("terms net=us,ak sort net:asc,mag:desc", ["--terms", "net=us,ak"], term("net", "us", "ak"),
              sort=[("net", False), ("mag", True)])
    ex.expect("range mag=2.5.. + depth<=100 sort magType:desc,time:asc",
              ["--range", "mag=2.5..", "--range", "depth=..100"],
              lambda r, m=rp("mag", 2.5), d=rp("depth", hi=100.0): m(r) and d(r),
              sort=[("magType", True), ("time", False)])
    ex.expect("all sort status:asc,depth:asc,id:desc", [], lambda r: True,
              sort=[("status", False), ("depth", False), ("id", True)])
    ex.expect("all sort type:desc,dmin:asc,updated:desc", [], lambda r: True,
              sort=[("type", True), ("dmin", False), ("updated", True)])
    ex.expect("text nevada sort magError:desc,nst:asc", ["--text", "nevada"], ex.has_words("nevada"),
              sort=[("magError", True), ("nst", False)])
    ex.expect("box small sort mag:desc,time:asc,id:asc", ["--box", "37.0123,37.6123,136.6123,137.4123"],
              ex.box(37.0123, 37.6123, 136.6123, 137.4123),
              sort=[("mag", True), ("time", False), ("id", False)])

    # Distance sort.
    ex.expect_distance_sort(35.77, -117.6, 20.725)
    ex.expect_distance_sort(61.2, -149.9, 7)
    ex.expect_distance_sort(37.5, 137.2, 300)


if __name__ == "__main__":
    sys.exit(main())
