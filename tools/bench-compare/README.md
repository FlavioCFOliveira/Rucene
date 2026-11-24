# bench-compare

Runs the same Lucene byTask algorithm on the same dataset with **Lucene 10.5.0
(Java)** and **Rucene** (`crates/rucene-benchmark`, bin `benchmark`), and prints
their `RepSumByName` / `RepSumByPrefRound` reports side by side (rmp #673).

The algorithm is `conf/micro-standard.alg`, the default of Lucene's
`gradle run` (`lucene/benchmark/build.gradle`). The dataset is Reuters-21578,
obtained as Lucene's `getReuters` task does.

## Usage

```sh
tools/bench-compare/run.sh            # all: fetch (if missing), lucene, rucene, compare
tools/bench-compare/run.sh fetch      # download + extract the dataset
tools/bench-compare/run.sh lucene     # run Lucene 10.5.0
tools/bench-compare/run.sh rucene     # build (release) and run Rucene
tools/bench-compare/run.sh compare    # print the table of the last two runs
```

`compare` exits with status 2 when the two runs' reports differ in shape
(operations, rounds, run counts, records per run); such rows are marked `(!)`.

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
* **lucene** — `java -Xmx1g -cp <jars> org.apache.lucene.benchmark.byTask.Benchmark <alg>`
  (`gradle run` uses `maxHeapSize` 1G and no other JVM flags).
* **rucene** — `rucene-cargo build --release -j 6 -p rucene-benchmark --bin benchmark`,
  then runs the binary on the same `.alg`.
* Both engines run with the working directory `$BENCH_HOME` and the default
  `work.dir=work`, so `docs.dir=reuters-out` and the index (`work/index`,
  removed before each run) resolve to the same paths. They run one after the
  other, each in a memory-capped scope with a `timeout` kill cap
  (`KILL_CAP`, default 900 s).
* The `.alg` is `crates/rucene-benchmark/tests/conf/micro-standard.alg`; the
  script fails if it differs from Lucene's copy.

## Locations (outside git)

| Variable | Default |
|---|---|
| `BENCH_HOME` | `/data/dev/github.com/FlavioCFOliveira/.rucene-perf/bench-compare` |
| `CARGO_TARGET_DIR` | `/data/dev/github.com/FlavioCFOliveira/.rucene-dev-target` |
| `TMPDIR_BENCH` | `/data/dev/github.com/FlavioCFOliveira/.rucene-test-tmp/bench-compare` |

Run logs: `$BENCH_HOME/runs/latest/{lucene,rucene}.out`, table:
`$BENCH_HOME/runs/latest/compare.txt`.
