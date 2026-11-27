# bench-compare

Runs the same Lucene byTask algorithm on the same dataset with **Lucene 10.5.0
(Java)** and **Rucene** (`crates/rucene-benchmark`, bin `benchmark`), and prints
every report block of both runs side by side (rmp #673, #677).

The algorithm is any `.alg` of `crates/rucene-benchmark/tests/conf/`; the
default is `micro-standard.alg`, the default of Lucene's `gradle run`
(`lucene/benchmark/build.gradle`). The dataset is Reuters-21578, obtained as
Lucene's `getReuters` task does.

## Usage

```sh
tools/bench-compare/run.sh            # all: fetch (if missing), lucene, rucene, compare
tools/bench-compare/run.sh fetch      # download + extract the dataset
tools/bench-compare/run.sh addindexes-input   # build the input index of addIndexes.alg (Lucene)
tools/bench-compare/run.sh lucene     # run Lucene 10.5.0
tools/bench-compare/run.sh rucene     # build (release) and run Rucene
tools/bench-compare/run.sh compare    # print the table of the last two runs
```

### Algorithm parameter

The algorithm is the second argument, or the `ALG` environment variable when
the argument is absent; the `.alg` suffix is optional:

```sh
tools/bench-compare/run.sh all standard            # conf/standard.alg
ALG=facets.alg tools/bench-compare/run.sh lucene   # conf/facets.alg
```

Other environment variables:

| Variable | Default | Effect |
|---|---|---|
| `ALG` | `micro-standard.alg` | algorithm when no second argument is given |
| `OUT_DIR` | `$BENCH_HOME/runs/latest` | where `lucene.out`, `rucene.out` and `compare.txt` go |
| `RUCENE_BUILD` | `1` | `0` skips the release build and runs the existing binary |
| `KILL_CAP` | `900` | wall-clock kill cap per engine run, in seconds |

Each engine run also writes its process wall-clock seconds to `<engine>.out.wall`.

### compare.py

`compare.py` parses every `------------> Report ...` block (`RepSumByName`,
`RepSumByPrefRound`, `RepSumByNameRound`, `RepSelectByPref`, `RepSumByPref`,
...), whatever its columns, including operation names that hold spaces,
overflow the Operation column or span several lines. It accepts one run per engine
(`compare.py lucene.out rucene.out`) or several
(`compare.py L1 L2 L3 -- R1 R2 R3`); with several, every `rec/s` and
`elapsedSec` cell and the wall-clock total are medians over each engine's
runs. `--tsv FILE` also writes one tab-separated line per row.

It exits with status 2 when the reports differ in shape between any two runs
(report titles, column headers, operations, rounds, run counts, records per
run); such rows are marked `(!)`.

## Requirements

* JDK 25 (`java` on `PATH`, or `JAVA=...`).
* The Lucene 10.5.0 clone at `LUCENE_SRC` (default `/tmp/lucene1050`, tag
  `releases/lucene/10.5.0`): it provides `ExtractReuters.java` and the
  reference `micro-standard.alg`.
* The Lucene 10.5.0 jars and their runtime dependencies in `~/.m2/repository`
  (listed in `run.sh`, as in `tests/sprint17_oracle/mod.rs`).
* `~/.local/bin/rucene-cargo` and the `rucene-tests.slice` systemd user slice.

## What it does

* **fetch** — downloads `reuters21578.tar.gz` (SHA-256 pinned in `run.sh`),
  untars it excluding `*.txt`, and runs Lucene's own
  `build-tools/.../datasets/ExtractReuters.java` unchanged (JDK source
  launcher) into `$BENCH_HOME/work/reuters-out`; checks 21578 files.
* **addindexes-input** — `addIndexes.alg` adds the index at
  `addindexes.input.dir=input/index` (relative to the cwd), which Lucene does
  not ship. This command builds `$BENCH_HOME/input/index` once with Lucene
  10.5.0 from the 21578 Reuters documents (StandardAnalyzer, FSDirectory,
  default codec, `{ AddDoc } : 21578`, CloseIndex); both engines then read the
  same input. `lucene`/`rucene` refuse to run such an algorithm while the input
  is missing.
* **lucene** — `java -Xmx1g -cp <jars> org.apache.lucene.benchmark.byTask.Benchmark <alg>`
  (`gradle run` uses `maxHeapSize` 1G and no other JVM flags).
* **rucene** — `rucene-cargo build --release -j 6 -p rucene-benchmark --bin benchmark`,
  then runs the binary on the same `.alg`.
* Both engines run with the working directory `$BENCH_HOME`, so `work.dir`
  (default `work`), `docs.dir` and the index resolve to the same paths. The
  index and taxonomy directories (`<work.dir>/index`, `<work.dir>/taxo`) are
  removed before each run.
* Algorithms with `docs.dir=reuters21578` (they set `work.dir=data`) read
  `$BENCH_HOME/data/reuters21578`, a directory of hard links to the
  `work/reuters-out` files. A symlink to the directory does not work:
  `ContentSource.collectFiles` walks with `Files.walkFileTree`, which does not
  follow links, so both engines fail with "Is a directory". They run one after the
  other, each in a memory-capped scope with a `timeout` kill cap
  (`KILL_CAP`, default 900 s).
* The script fails if the `.alg` differs from Lucene's
  `lucene/benchmark/conf/<alg>`.

## Locations (outside git)

| Variable | Default |
|---|---|
| `BENCH_HOME` | `/data/dev/github.com/FlavioCFOliveira/.rucene-perf/bench-compare` |
| `CARGO_TARGET_DIR` | `/data/dev/github.com/FlavioCFOliveira/.rucene-dev-target` |
| `TMPDIR_BENCH` | `/data/dev/github.com/FlavioCFOliveira/.rucene-test-tmp/bench-compare` |

Run logs: `$OUT_DIR/{lucene,rucene}.out` (default `$BENCH_HOME/runs/latest`),
table: `$OUT_DIR/compare.txt`.
