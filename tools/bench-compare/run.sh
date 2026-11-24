#!/usr/bin/env bash
# Comparative byTask benchmark: Lucene 10.5.0 (Java) vs Rucene on Reuters-21578.
#
# Runs Lucene's `gradle run` default algorithm (conf/micro-standard.alg) with
# both engines on the same dataset and prints the RepSumByName /
# RepSumByPrefRound reports side by side. See README.md.
#
# Usage: tools/bench-compare/run.sh [fetch|lucene|rucene|compare|all]   (default: all)
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TOOL="$REPO/tools/bench-compare"

BENCH_HOME="${BENCH_HOME:-/data/dev/github.com/FlavioCFOliveira/.rucene-perf/bench-compare}"
LUCENE_SRC="${LUCENE_SRC:-/tmp/lucene1050}"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/data/dev/github.com/FlavioCFOliveira/.rucene-dev-target}"
TMPDIR="${TMPDIR_BENCH:-/data/dev/github.com/FlavioCFOliveira/.rucene-test-tmp/bench-compare}"
RUCENE_CARGO="${RUCENE_CARGO:-$HOME/.local/bin/rucene-cargo}"
M2="${M2:-$HOME/.m2/repository}"
KILL_CAP="${KILL_CAP:-900}"
JAVA="${JAVA:-java}"
export CARGO_TARGET_DIR TMPDIR

ALG_NAME="micro-standard.alg"
ALG="$REPO/crates/rucene-benchmark/tests/conf/$ALG_NAME"
WORK="$BENCH_HOME/work"            # work.dir default "work", relative to the cwd
RUNS="$BENCH_HOME/runs"

# getReuters (gradle/datasets/external-datasets.gradle:137-166).
REUTERS_URL="https://kdd.ics.uci.edu/databases/reuters21578/reuters21578.tar.gz"
REUTERS_TGZ="$WORK/reuters21578.tar.gz"
# SHA-256 of the archive as downloaded for rmp #673 (2026-11-24). Lucene's
# task checks no digest; this pin only makes a changed upstream file visible.
REUTERS_SHA256="${REUTERS_SHA256:-3bae43c9b14e387f76a61b6d82bf98a4fb5d3ef99ef7e7075ff2ccbcf59f9d30}"
REUTERS_OUT="$WORK/reuters-out"
EXPECTED_DOCS=21578

# The runtime class path of `:lucene:benchmark:run` (lucene/benchmark/build.gradle):
# the module, its Lucene modules and third-party dependencies as the 10.5.0 POMs
# declare them (same jars as Module::Benchmark in tests/sprint17_oracle/mod.rs,
# without the test framework).
JARS=(
  org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar
  org/apache/lucene/lucene-benchmark/10.5.0/lucene-benchmark-10.5.0.jar
  org/apache/lucene/lucene-analysis-common/10.5.0/lucene-analysis-common-10.5.0.jar
  org/apache/lucene/lucene-facet/10.5.0/lucene-facet-10.5.0.jar
  org/apache/lucene/lucene-highlighter/10.5.0/lucene-highlighter-10.5.0.jar
  org/apache/lucene/lucene-memory/10.5.0/lucene-memory-10.5.0.jar
  org/apache/lucene/lucene-queries/10.5.0/lucene-queries-10.5.0.jar
  org/apache/lucene/lucene-queryparser/10.5.0/lucene-queryparser-10.5.0.jar
  org/apache/lucene/lucene-spatial-extras/10.5.0/lucene-spatial-extras-10.5.0.jar
  org/apache/lucene/lucene-spatial3d/10.5.0/lucene-spatial3d-10.5.0.jar
  org/locationtech/spatial4j/spatial4j/0.8/spatial4j-0.8.jar
  io/sgr/s2-geometry-library-java/1.0.0/s2-geometry-library-java-1.0.0.jar
  org/apache/commons/commons-compress/1.19/commons-compress-1.19.jar
  com/ibm/icu/icu4j/77.1/icu4j-77.1.jar
  org/apache/lucene/lucene-analysis-icu/10.5.0/lucene-analysis-icu-10.5.0.jar
  net/sourceforge/nekohtml/nekohtml/1.9.17/nekohtml-1.9.17.jar
  xerces/xercesImpl/2.12.2/xercesImpl-2.12.2.jar
)

die() { echo "bench-compare: $*" >&2; exit 1; }

# Memory-capped scope with a wall-clock kill cap.
capped() {
  systemd-run --user --scope -q --collect --slice=rucene-tests.slice \
    -p MemorySwapMax=0 -- timeout "$KILL_CAP" "$@"
}

check_lucene_src() {
  [[ -d "$LUCENE_SRC/.git" ]] || die "Lucene 10.5.0 clone not found at $LUCENE_SRC (git clone --branch releases/lucene/10.5.0 --single-branch https://github.com/apache/lucene.git $LUCENE_SRC)"
  local tag
  tag="$(git -C "$LUCENE_SRC" describe --tags --exact-match 2>/dev/null || true)"
  [[ "$tag" == "releases/lucene/10.5.0" ]] || die "$LUCENE_SRC is not at tag releases/lucene/10.5.0 (found: ${tag:-none})"
}

check_alg() {
  check_lucene_src
  cmp -s "$ALG" "$LUCENE_SRC/lucene/benchmark/conf/$ALG_NAME" \
    || die "$ALG differs from Lucene's lucene/benchmark/conf/$ALG_NAME"
}

cmd_fetch() {
  check_lucene_src
  mkdir -p "$WORK" "$TMPDIR"
  # Download (overwrite false: an existing archive is kept).
  if [[ ! -f "$REUTERS_TGZ" ]]; then
    echo "Downloading $REUTERS_URL ..."
    curl -fsSL --retry 3 -o "$REUTERS_TGZ.part" "$REUTERS_URL"
    mv "$REUTERS_TGZ.part" "$REUTERS_TGZ"
  fi
  local sha
  sha="$(sha256sum "$REUTERS_TGZ" | cut -d' ' -f1)"
  echo "reuters21578.tar.gz sha256 $sha"
  if [[ "$sha" != "$REUTERS_SHA256" ]]; then
    die "archive SHA-256 $sha differs from the pinned $REUTERS_SHA256 (override with REUTERS_SHA256=...)"
  fi

  # Decompress, excluding *.txt, into a fresh untar directory (project.sync).
  local untar="$TMPDIR/reuters-untar"
  rm -rf "$untar"
  mkdir -p "$untar"
  tar -xzf "$REUTERS_TGZ" -C "$untar" --exclude='*.txt' --no-same-owner

  # ext.dst.deleteDir(); buildinfra.extractReuters(untar, dst), which is
  # ExtractReuters.main({untar, dst}). Lucene's own source, run unchanged by
  # the JDK source launcher.
  rm -rf "$REUTERS_OUT" "$REUTERS_OUT-tmp"
  echo "Extracting into $REUTERS_OUT ..."
  capped "$JAVA" -Djava.io.tmpdir="$TMPDIR" \
    "$LUCENE_SRC/build-tools/build-infra/src/main/java/org/apache/lucene/gradle/datasets/ExtractReuters.java" \
    "$untar" "$REUTERS_OUT"
  rm -rf "$untar"

  local n
  n="$(find "$REUTERS_OUT" -maxdepth 1 -type f -name '*.txt' | wc -l)"
  echo "reuters-out: $n files"
  [[ "$n" -eq "$EXPECTED_DOCS" ]] || die "expected $EXPECTED_DOCS extracted documents, found $n"
}

check_dataset() {
  [[ -d "$REUTERS_OUT" ]] || die "dataset missing: run '$0 fetch' first"
}

run_dir() {
  local d="$RUNS/latest"
  mkdir -p "$d"
  echo "$d"
}

cmd_lucene() {
  check_alg
  check_dataset
  local cp="" jar
  for jar in "${JARS[@]}"; do
    [[ -f "$M2/$jar" ]] || die "missing jar $M2/$jar"
    cp="${cp:+$cp:}$M2/$jar"
  done
  local out
  out="$(run_dir)/lucene.out"
  rm -rf "$WORK/index"
  echo "Running Lucene 10.5.0 ($("$JAVA" -version 2>&1 | head -1)) -> $out"
  (cd "$BENCH_HOME" && capped "$JAVA" -Xmx1g -Djava.io.tmpdir="$TMPDIR" -cp "$cp" \
    org.apache.lucene.benchmark.byTask.Benchmark "$ALG") >"$out" 2>&1 \
    || { tail -20 "$out" >&2; die "Lucene run failed (log: $out)"; }
}

cmd_rucene() {
  check_alg
  check_dataset
  mkdir -p "$TMPDIR"
  echo "Building rucene-benchmark (release) ..."
  (cd "$REPO" && "$RUCENE_CARGO" build --release -j 6 -p rucene-benchmark --bin benchmark)
  local bin="$CARGO_TARGET_DIR/release/benchmark"
  local out
  out="$(run_dir)/rucene.out"
  rm -rf "$WORK/index"
  echo "Running Rucene ($bin) -> $out"
  (cd "$BENCH_HOME" && capped "$bin" "$ALG") >"$out" 2>&1 \
    || { tail -20 "$out" >&2; die "Rucene run failed (log: $out)"; }
}

cmd_compare() {
  local d="$RUNS/latest"
  [[ -f "$d/lucene.out" && -f "$d/rucene.out" ]] || die "run '$0 lucene' and '$0 rucene' first"
  python3 -I "$TOOL/compare.py" "$d/lucene.out" "$d/rucene.out" | tee "$d/compare.txt"
}

case "${1:-all}" in
  fetch) cmd_fetch ;;
  lucene) cmd_lucene ;;
  rucene) cmd_rucene ;;
  compare) cmd_compare ;;
  all)
    [[ -d "$REUTERS_OUT" ]] || cmd_fetch
    cmd_lucene
    cmd_rucene
    cmd_compare
    ;;
  *) die "usage: $0 [fetch|lucene|rucene|compare|all]" ;;
esac
