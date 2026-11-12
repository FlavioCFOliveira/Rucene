#!/usr/bin/env bash
# Run Rucene's test suites in parallel, memory-capped, with early stall
# detection. Usage: tools/test/run_suite.sh <out-dir> [extra nextest args...]
#
# * cargo-nextest runs every test of the workspace (except randomizedtesting,
#   whose two harness=false targets nextest cannot list) as its own process,
#   12 at a time (.config/nextest.toml), each inside rucene-tests.slice through
#   ~/.local/bin/rucene-mem-runner (aggregate MemoryMax 14G);
# * `cargo test -p randomizedtesting` runs beside it;
# * tools/test/stall_watchdog.py watches every test process: a BLOCKED one
#   (no CPU progress, no running thread for 90 s) is dumped with gdb and killed
#   at once; SPINNING and GROWING ones are dumped and left to nextest's
#   slow-timeout. Alerts and dumps: <out-dir>/alerts.log.
# Outputs: <out-dir>/nextest.log, randomizedtesting.log, alerts.log, summary.
set -u
OUT=${1:?usage: run_suite.sh <out-dir> [nextest args...]}; shift
REPO=$(cd "$(dirname "$0")/../.." && pwd)
mkdir -p "$OUT"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/data/dev/github.com/FlavioCFOliveira/.rucene-dev-target}
export TMPDIR=${TMPDIR:-/data/dev/github.com/FlavioCFOliveira/.rucene-test-tmp/suite}
mkdir -p "$TMPDIR"
CARGO=${CARGO_WRAPPER:-$HOME/.local/bin/rucene-cargo}
cd "$REPO"

# The Java harness once, so each test's `mvn -q compile` has nothing to do.
(cd tests/fixtures/java-codec-harness && mvn -q -o compile) || { echo "harness compile failed"; exit 2; }

# Build everything first (outside the test slice), then run.
"$CARGO" nextest run --workspace --exclude randomizedtesting --no-run -j 6 > "$OUT/build.log" 2>&1 \
  || { echo "build failed: $OUT/build.log"; exit 2; }
"$CARGO" test -p randomizedtesting --no-run -j 6 >> "$OUT/build.log" 2>&1 \
  || { echo "build failed: $OUT/build.log"; exit 2; }

: > "$OUT/alerts.log"
python3 tools/test/stall_watchdog.py --log "$OUT/alerts.log" --kill-blocked &
WD=$!
trap 'kill $WD 2>/dev/null' EXIT

start=$(date +%s)
"$CARGO" nextest run --workspace --exclude randomizedtesting --no-fail-fast "$@" > "$OUT/nextest.log" 2>&1 &
NX=$!
"$CARGO" test -p randomizedtesting --no-fail-fast > "$OUT/randomizedtesting.log" 2>&1 &
RT=$!
wait $NX; nx=$?
wait $RT; rt=$?
kill $WD 2>/dev/null

{
  echo "elapsed $(( $(date +%s) - start ))s  nextest rc=$nx  randomizedtesting rc=$rt"
  grep -E '^\s+Summary' "$OUT/nextest.log" | tail -1
  grep -E '^test result' "$OUT/randomizedtesting.log"
  echo "alerts: $(grep -c '^ALERT' "$OUT/alerts.log")"
  grep '^ALERT' "$OUT/alerts.log"
  echo "failed tests:"
  grep -E '^\s+(FAIL|TIMEOUT|SIGKILL|SIGSEGV|SIGABRT|LEAK)' "$OUT/nextest.log" | sed 's/^\s*//' | sort -u
} > "$OUT/summary"
cat "$OUT/summary"
[ $nx -eq 0 ] && [ $rt -eq 0 ]
