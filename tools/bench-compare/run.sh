#!/usr/bin/env bash
# Comparative byTask benchmark: Lucene 10.5.0 (Java) vs Rucene on Reuters-21578.
#
# Runs one byTask algorithm of conf/ (default micro-standard.alg, the default
# of Lucene's `gradle run`) with both engines on the same dataset and prints
# every report block side by side. See README.md.
#
# Usage: tools/bench-compare/run.sh [fetch|addindexes-input|lucene|rucene|compare|all] [ALG]
#   (default: all; ALG defaults to $ALG, then micro-standard.alg; the ".alg"
#   suffix is optional)
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

ALG_NAME="${2:-${ALG:-micro-standard.alg}}"
[[ "$ALG_NAME" == *.alg ]] || ALG_NAME="$ALG_NAME.alg"
ALG="$REPO/crates/rucene-benchmark/tests/conf/$ALG_NAME"
WORK="$BENCH_HOME/work"            # where the dataset is extracted
RUNS="$BENCH_HOME/runs"
OUT_DIR="${OUT_DIR:-$RUNS/latest}" # run logs and the comparison table
RUCENE_BUILD="${RUCENE_BUILD:-1}"  # 0: run the existing release binary

# Last assignment of a property in the .alg (byTask Config: a later line wins),
# or the given default.
alg_prop() {
  local v
  v="$(sed -n "s/^[[:space:]]*$1[[:space:]]*=[[:space:]]*//p" "$ALG" | tail -1 | sed 's/[[:space:]]*$//')"
  echo "${v:-$2}"
}
# work.dir (PerfRunData, default "work"), relative to the cwd $BENCH_HOME.
ALG_WORK="$BENCH_HOME/$(alg_prop 'work\.dir' work)"

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
  check_addindexes_input
  # Algorithms with docs.dir=reuters21578 (resolved against work.dir, as
  # ReutersContentSource does) read the same extracted reuters-out. A symlink
  # to the directory does not work: ContentSource.collectFiles walks with
  # Files.walkFileTree, which does not follow links, and then opens the link
  # itself ("Is a directory", both engines). The directory is therefore a real
  # one holding hard links to the same files (same inodes, same bytes).
  if [[ "$(alg_prop 'docs\.dir' reuters-out)" == reuters21578 ]]; then
    local dir="$ALG_WORK/reuters21578"
    [[ ! -L "$dir" ]] || die "$dir is a symlink; remove it (see check_dataset)"
    if [[ ! -d "$dir" ]]; then
      rm -rf "$dir.part"
      cp -al "$REUTERS_OUT" "$dir.part"
      mv "$dir.part" "$dir"
    fi
    local n
    n="$(find "$dir" -maxdepth 1 -type f -name '*.txt' -links +1 | wc -l)"
    [[ "$n" -eq "$EXPECTED_DOCS" ]] \
      || die "$dir holds $n hard-linked documents, expected $EXPECTED_DOCS"
  fi
}

# Index and taxonomy directories of PerfRunData (work.dir/index, work.dir/taxo),
# removed before each run.
wipe_index() {
  rm -rf "$ALG_WORK/index" "$ALG_WORK/taxo"
}

run_dir() {
  mkdir -p "$OUT_DIR"
  echo "$OUT_DIR"
}

# Runs "$@" in the capped scope from $BENCH_HOME, output to $1's log; writes
# the process wall-clock seconds to <log>.wall.
timed_run() {
  local out="$1"; shift
  local t0 t1 rc=0
  t0="$(date +%s.%N)"
  (cd "$BENCH_HOME" && capped "$@") >"$out" 2>&1 || rc=$?
  t1="$(date +%s.%N)"
  echo "$t0 $t1" | LC_ALL=C awk '{printf "%.3f\n", $2 - $1}' >"$out.wall"
  return "$rc"
}

lucene_cp() {
  local cp="" jar
  for jar in "${JARS[@]}"; do
    [[ -f "$M2/$jar" ]] || die "missing jar $M2/$jar"
    cp="${cp:+$cp:}$M2/$jar"
  done
  echo "$cp"
}

# Input index of addIndexes.alg (addindexes.input.dir=input/index, opened by
# AddIndexesTask relative to the cwd). Lucene ships none: it is built once,
# by Lucene 10.5.0, from the 21578 Reuters documents (StandardAnalyzer,
# FSDirectory and the default codec, as addIndexes.alg configures its target),
# and then read by both engines.
ADDINDEXES_INPUT="$BENCH_HOME/input/index"
cmd_addindexes_input() {
  check_lucene_src
  [[ -d "$REUTERS_OUT" ]] || die "dataset missing: run '$0 fetch' first"
  [[ ! -e "$ADDINDEXES_INPUT" ]] || die "$ADDINDEXES_INPUT exists; remove it to rebuild"
  mkdir -p "$TMPDIR" "$(run_dir)"
  local alg="$TMPDIR/addIndexes-input.alg"
  cat >"$alg" <<ALG
analyzer=org.apache.lucene.analysis.standard.StandardAnalyzer
directory=FSDirectory
work.dir=$BENCH_HOME/input
docs.dir=$REUTERS_OUT
content.source=org.apache.lucene.benchmark.byTask.feeds.ReutersContentSource
ResetSystemErase
CreateIndex
{ AddDoc } : $EXPECTED_DOCS
CloseIndex
OpenReader
CloseReader
RepSumByName
ALG
  local out
  out="$(run_dir)/addindexes-input.out"
  echo "Building $ADDINDEXES_INPUT with Lucene 10.5.0 -> $out"
  timed_run "$out" "$JAVA" -Xmx1g -Djava.io.tmpdir="$TMPDIR" -cp "$(lucene_cp)" \
    org.apache.lucene.benchmark.byTask.Benchmark "$alg" \
    || { tail -20 "$out" >&2; die "input index build failed (log: $out)"; }
  cp "$alg" "$(run_dir)/addIndexes-input.alg"
}

check_addindexes_input() {
  if [[ -n "$(alg_prop 'addindexes\.input\.dir' '')" ]]; then
    [[ -d "$BENCH_HOME/$(alg_prop 'addindexes\.input\.dir' '')" ]] \
      || die "addIndexes input index missing: run '$0 addindexes-input' first"
  fi
}

cmd_lucene() {
  check_alg
  check_dataset
  local cp
  cp="$(lucene_cp)"
  local out
  out="$(run_dir)/lucene.out"
  wipe_index
  echo "Running Lucene 10.5.0 ($("$JAVA" -version 2>&1 | head -1)) on $ALG_NAME -> $out"
  timed_run "$out" "$JAVA" -Xmx1g -Djava.io.tmpdir="$TMPDIR" -cp "$cp" \
    org.apache.lucene.benchmark.byTask.Benchmark "$ALG" \
    || { tail -20 "$out" >&2; die "Lucene run failed (log: $out)"; }
}

cmd_rucene() {
  check_alg
  check_dataset
  mkdir -p "$TMPDIR"
  if [[ "$RUCENE_BUILD" != 0 ]]; then
    echo "Building rucene-benchmark (release) ..."
    (cd "$REPO" && "$RUCENE_CARGO" build --release -j 6 -p rucene-benchmark --bin benchmark)
  fi
  local bin="$CARGO_TARGET_DIR/release/benchmark"
  [[ -x "$bin" ]] || die "missing $bin (run with RUCENE_BUILD=1)"
  local out
  out="$(run_dir)/rucene.out"
  wipe_index
  echo "Running Rucene ($bin) on $ALG_NAME -> $out"
  timed_run "$out" "$bin" "$ALG" \
    || { tail -20 "$out" >&2; die "Rucene run failed (log: $out)"; }
}

cmd_compare() {
  local d="$OUT_DIR"
  [[ -f "$d/lucene.out" && -f "$d/rucene.out" ]] || die "run '$0 lucene' and '$0 rucene' first"
  python3 -I "$TOOL/compare.py" "$d/lucene.out" "$d/rucene.out" | tee "$d/compare.txt"
}

case "${1:-all}" in
  fetch) cmd_fetch ;;
  addindexes-input) cmd_addindexes_input ;;
  lucene) cmd_lucene ;;
  rucene) cmd_rucene ;;
  compare) cmd_compare ;;
  all)
    [[ -d "$REUTERS_OUT" ]] || cmd_fetch
    cmd_lucene
    cmd_rucene
    cmd_compare
    ;;
  *) die "usage: $0 [fetch|addindexes-input|lucene|rucene|compare|all] [ALG]" ;;
esac
