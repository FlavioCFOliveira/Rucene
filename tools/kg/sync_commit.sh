#!/usr/bin/env bash
# Sync the knowledge graph with one commit, end to end, and audit it.
#
#   tools/kg/sync_commit.sh [--full] <commit> [roadmap]
#
# Two modes (`sync_plan.py` decides and prints why):
#
# * **full** -- every layer re-derived from its sources. Forced by `--full`, and
#   taken whenever the incremental mode is not sound: no recorded sync, a graph
#   whose `syncedCommit` is not the recorded one, or a change to `tools/kg`, a
#   clone or the toolchain since (the sprint close runs `--full`).
# * **incremental** -- from the recorded sync's commit (the graph's
#   `syncedCommit`), re-derives only what `git diff` can change: the Lucene and
#   library survey (`port_modules_kg.py --phase modules/structure`, pinned
#   clones) is not re-run; the macro expansion of a crate the diff cannot reach
#   is reused; rust-analyzer indexes only when a crate changed, and the
#   `REFERENCES` edges are written as the difference from the recorded ones.
#   Every other step runs as in the full mode; the audit is the same.
#
# `--pretend-unchanged <crate>` drops a changed crate from the incremental plan:
# the positive control of `compare_kg.py`, never a way to sync.
#
# A step is followed by a server restart (`checkpoint`) only when it wrote to
# the graph (`kgio`'s `KG_WRITES` record). `KG_PROFILE_DIR=<dir>` runs every
# Python step under `cProfile`, one `<n>-<script>.prof` per step.
#
# The commit is surveyed from a `git archive` export, so the working tree may
# hold uncommitted work (a subagent's) while the sync runs. Every step prints
# `=== <epoch> <command>` before and `exit=<code> t=<epoch>` after, so the log
# is its own timing record. The chain stops at the first failing step. The pass
# condition is the audit's `problems: 0`.
#
# Outputs go to the KG target directory (`kgio.kg_target()`:
# `$RUCENE_KG_TARGET`, else `.rucene-kg-target` beside the checkout):
# the survey export `rucene-survey-<short>/`, the survey JSON, the triage
# snapshot `triage-<short>/`, the no-caller list and the log
# `kg_sync_<short>.log` (tee'd to stdout).
set -u
FORCE_FULL=
PRETEND=()
while [ $# -gt 0 ]; do
  case $1 in
    --full) FORCE_FULL=--full; shift ;;
    --pretend-unchanged) PRETEND+=(--pretend-unchanged "$2"); shift 2 ;;
    *) break ;;
  esac
done
COMMIT_ARG=${1:?usage: sync_commit.sh [--full] <commit> [roadmap]}
R=${2:-rucene}
REPO=$(cd "$(dirname "$0")/../.." && pwd)
cd "$REPO"
FULL=$(git rev-parse --verify "${COMMIT_ARG}^{commit}") || exit 2
SHORT=${FULL:0:7}
DATE=$(git show -s --format=%cs "$FULL")
K=${RUCENE_KG_TARGET:-$(dirname "$REPO")/.rucene-kg-target}
S=$K/rucene-survey-$SHORT
T=$K/triage-$SHORT
LUCENE=${LUCENE_ROOT:-/tmp/lucene1050}
# randomizedtesting 2.8.1, one of the port modules (`port_modules_kg.py`):
#   git clone --branch release/2.8.1 --single-branch \
#       https://github.com/randomizedtesting/randomizedtesting.git "$RT"
RT=${RANDOMIZEDTESTING_ROOT:-/tmp/randomizedtesting281}
[ -d "$RT/randomized-runner/src/main/java" ] || {
  echo "no randomizedtesting 2.8.1 clone at $RT (set RANDOMIZEDTESTING_ROOT)"; exit 2; }
# spatial4j 0.8 and s2-geometry-library-java 1.0.0, the two libraries sprint 17
# ports (`port_modules_kg.py`, which reads the same variables):
#   git clone --depth 1 --branch spatial4j-0.8 \
#       https://github.com/locationtech/spatial4j.git "$SPATIAL4J_ROOT"
#   git clone --depth 1 --branch v1.0.0 \
#       https://github.com/sgr-io/s2-geometry-library-java.git "$S2_GEOMETRY_ROOT"
export SPATIAL4J_ROOT=${SPATIAL4J_ROOT:-/tmp/spatial4j-0.8}
export S2_GEOMETRY_ROOT=${S2_GEOMETRY_ROOT:-/tmp/s2-geometry-1.0.0}
[ -d "$SPATIAL4J_ROOT/src/main/java/org/locationtech/spatial4j" ] || {
  echo "no spatial4j 0.8 clone at $SPATIAL4J_ROOT (set SPATIAL4J_ROOT)"; exit 2; }
[ -d "$S2_GEOMETRY_ROOT/src/main/java/com/google/common/geometry" ] || {
  echo "no s2-geometry 1.0.0 clone at $S2_GEOMETRY_ROOT (set S2_GEOMETRY_ROOT)"; exit 2; }
# nekohtml 1.9.17 with the Xerces-J 2.12.2 and SAX types of its closure, the
# third sprint-17 library: the three Maven Central source jars unpacked into
# `neko/`, `xerces/` and `xmlapis/` under one root (`port_modules_kg.py`,
# `NEKOHTML_ROOT`, gives the coordinates and checksums).
export NEKOHTML_ROOT=${NEKOHTML_ROOT:-/tmp/nekohtml-1.9.17}
for t in neko/org/cyberneko/html xerces/org/apache/xerces xmlapis/org/xml/sax; do
  [ -d "$NEKOHTML_ROOT/$t" ] || {
    echo "no nekohtml 1.9.17 sources at $NEKOHTML_ROOT/$t (set NEKOHTML_ROOT)"; exit 2; }
done
# icu4j 77.1, the library `lucene/analysis/icu` is built on (`port_modules_kg.py`,
# `ICU4J_ROOT`): the `icu4j` directory of the ICU `release-77-1` clone.
export ICU4J_ROOT=${ICU4J_ROOT:-/tmp/icu4j-77.1}
[ -d "$ICU4J_ROOT/main/core/src/main/java/com/ibm/icu" ] || {
  echo "no icu4j 77.1 sources at $ICU4J_ROOT (set ICU4J_ROOT)"; exit 2; }
LOG=$K/kg_sync_$SHORT.log
mkdir -p "$K"
rm -rf "$S" && mkdir -p "$S" && git archive "$FULL" | tar -x -C "$S" || exit 2

# Restart the graph server, which checkpoints the store. Measured on rmp 1.21.0
# (2026-11-11): nodes written since the server started are missed by an
# `AllNodesScan` until a checkpoint -- `MATCH (n) WHERE n:Class` returned 2557
# against 2816 for `MATCH (n:Class)`, and every pattern opening on an unlabelled
# node lost the same edges -- and the engine has no checkpoint procedure. So a
# step that reads must never follow a step that wrote without a restart between
# them. This is the one place the tooling stops and starts the server.
checkpoint() {
  local pid log
  pid=$(pgrep -f "^rmp graph serve -r $R\$")
  if [ -n "$pid" ]; then
    kill -TERM $pid
    while kill -0 $pid 2>/dev/null; do sleep 0.2; done
  fi
  log=$K/kg_serve_$SHORT.log
  setsid nohup rmp graph serve -r "$R" > "$log" 2>&1 < /dev/null &
  for _ in $(seq 150); do
    grep -q '"socket"' "$log" 2>/dev/null && return 0
    sleep 0.2
  done
  echo "graph server did not start; see $log"; exit 3
}

# A step that may write to the graph is run by `run`, and followed by a
# checkpoint when it did write: `kgio` appends one line to `$KG_WRITES` per
# statement that changed the graph, so an empty record means no node was
# written since the last restart and a later scan can miss nothing. A step
# outside `kgio` (the final `rmp graph client` stamp) is always followed by one.
# A step that only reads the graph or works off it (`offline`) never is.
export KG_WRITES=$K/kg_writes_$SHORT
PROFILE_N=0
# `python3 <script> ...`, under `cProfile` when `KG_PROFILE_DIR` is set.
profiled() {
  if [ -n "${KG_PROFILE_DIR:-}" ] && [ "$1" = python3 ]; then
    PROFILE_N=$((PROFILE_N + 1))
    mkdir -p "$KG_PROFILE_DIR"
    local out
    out=$KG_PROFILE_DIR/$(printf %02d $PROFILE_N)-$(basename "$2" .py).prof
    shift
    python3 -m cProfile -o "$out" "$@"
  else
    "$@"
  fi
}

run() {
  echo "=== $(date +%s) $*"
  : > "$KG_WRITES"
  profiled "$@"
  local rc=$?
  echo "exit=$rc t=$(date +%s)"
  if [ $rc -ne 0 ]; then echo "STOPPED at: $*"; exit $rc; fi
  if [ "$1" = python3 ] && [ ! -s "$KG_WRITES" ]; then
    echo "no graph write: checkpoint skipped"
  else
    checkpoint
  fi
}

offline() {
  echo "=== $(date +%s) $*"
  profiled "$@"
  local rc=$?
  echo "exit=$rc t=$(date +%s)"
  if [ $rc -ne 0 ]; then echo "STOPPED at: $*"; exit $rc; fi
}

# A step the incremental plan leaves out, logged with the reason.
skipped() {
  echo "=== $(date +%s) [skipped: $1] ${*:2}"
  echo "exit=0 t=$(date +%s)"
}

# The heaviest work needs no graph: the macro-expanded survey of the commit
# (`extract_rucene_kg.py --expand`, whose compiler dumps `expanded_evidence_kg.py`
# then reuses from its cache) and the rust-analyzer SCIP index. Both start with
# the sync and are joined just before the step that consumes them, so they run
# while the graph steps before that point run. `bg <name> <cmd...>` starts one;
# `join <name>` waits for it, replays its log and stops the sync if it failed.
declare -A BG_PID
bg() {
  local name=$1; shift
  echo "=== $(date +%s) [background $name] $*"
  ( profiled "$@" > "$K/bg_${name}_$SHORT.log" 2>&1 ) &
  BG_PID[$name]=$!
}

join() {
  local name=$1 rc
  wait "${BG_PID[$name]}"; rc=$?
  cat "$K/bg_${name}_$SHORT.log"
  echo "exit=$rc t=$(date +%s) [background $name joined]"
  if [ $rc -ne 0 ]; then echo "STOPPED at: background $name"; exit $rc; fi
}

KG=tools/kg
{
  echo "sync $FULL ($DATE) roadmap=$R"
  # The plan reads the graph's `syncedCommit`, so the server must be up.
  pgrep -f "^rmp graph serve -r $R\$" > /dev/null || checkpoint
  MODE=full BASE= EXPAND= INDEX= PREV_REFS= PLAN=
  PLAN_OUT=$(python3 $KG/sync_plan.py plan "$FULL" --roadmap "$R" $FORCE_FULL "${PRETEND[@]}") || {
    echo "STOPPED at: sync_plan.py plan"; exit 2; }
  echo "$PLAN_OUT"
  eval "$(echo "$PLAN_OUT" | grep -E '^(MODE|BASE|EXPAND|INDEX|PREV_REFS|PLAN)=')"
  echo "mode: $MODE"
  REFS=$K/no_caller/rust_refs_$SHORT.jsonl
  if [ "$MODE" = incremental ]; then
    SURVEY_REUSE=(--reuse-expanded-from "rucene-survey-${BASE:0:7}" --expand-only "$EXPAND")
    if [ -n "$INDEX" ]; then
      REFS_SOURCE=(--scip "$K/no_caller/index_$SHORT.scip" --previous-refs "$PREV_REFS")
    else
      REFS_SOURCE=(--reuse-refs "$PREV_REFS" --previous-refs "$PREV_REFS")
    fi
  else
    SURVEY_REUSE=()
    REFS_SOURCE=(--scip "$K/no_caller/index_$SHORT.scip")
  fi
  # The port modules first: their Module scope, JPMS dependencies, scope
  # decisions, and the packages, files, types and members of every module
  # beyond lucene/core, which port_coverage_kg.py scopes below.
  bg survey python3 $KG/extract_rucene_kg.py --source-root "$S" --expand --commit "$FULL" --date "$DATE" --output "$K/rucene_kg/survey.json" "${SURVEY_REUSE[@]}"
  if [ "$MODE" = full ] || [ -n "$INDEX" ]; then
    bg scip python3 $KG/rust_refs_kg.py "$R" --commit "$FULL" --phase scip --scip "$K/no_caller/index_$SHORT.scip"
  fi
  if [ "$MODE" = full ]; then
    run python3 $KG/port_modules_kg.py "$R" --lucene-root "$LUCENE" --rt-root "$RT" --commit "$FULL" --date "$DATE" --phase modules
    run python3 $KG/port_modules_kg.py "$R" --lucene-root "$LUCENE" --rt-root "$RT" --commit "$FULL" --date "$DATE" --phase structure
  else
    # Their inputs are the pinned clones and `PORT_MODULES`, which the
    # fingerprint the plan checked has not changed since the recorded sync.
    skipped "scope and clones unchanged" port_modules_kg.py --phase modules
    skipped "scope and clones unchanged" port_modules_kg.py --phase structure
  fi
  join survey
  run python3 $KG/load_rucene_kg.py "$R" --survey "$K/rucene_kg/survey.json"
  run python3 $KG/port_coverage_kg.py "$R" --survey "$K/rucene_kg/survey.json" --lucene-root "$LUCENE" --rt-root "$RT" --commit "$FULL" --date "$DATE"
  offline python3 $KG/port_evidence_kg.py "$R" --source-root "$S" --phase extract
  run python3 $KG/port_evidence_kg.py "$R" --source-root "$S" --phase load --commit "$FULL" --date "$DATE"
  run python3 $KG/expanded_evidence_kg.py "$R" --source-root "$S" --commit "$FULL" --date "$DATE"
  run python3 $KG/nested_port_kg.py "$R" --lucene-root "$LUCENE" --commit "$FULL" --date "$DATE"
  run python3 $KG/port_coverage_kg.py "$R" --survey "$K/rucene_kg/survey.json" --lucene-root "$LUCENE" --rt-root "$RT" --source-root "$S" --commit "$FULL" --date "$DATE" --phase scope
  # The member depth runs after the scope phase: it measures the nested types
  # with a type-level port too, and the scope phase writes the `host-file-name`
  # ports of the added modules' nested types (`port_evidence_kg.DEPTH_PAIRS`).
  run python3 $KG/port_evidence_kg.py "$R" --source-root "$S" --phase depth --commit "$FULL" --date "$DATE"
  run python3 $KG/port_coverage_kg.py "$R" --survey "$K/rucene_kg/survey.json" --lucene-root "$LUCENE" --rt-root "$RT" --commit "$FULL" --date "$DATE" --phase candidates
  # The member triage: `memberAbsent` is the number to plan from, so it is
  # re-derived at every sync from the floor the depth phase just measured.
  offline python3 $KG/port_modules_kg.py "$R" --lucene-root "$LUCENE" --rt-root "$RT" --commit "$FULL" --date "$DATE" --phase coverage
  offline python3 $KG/triage_dump.py "$R" "$T"
  offline python3 $KG/triage_repro.py "$T" "$T/unmatched.json"
  offline python3 $KG/triage_members.py "$T" "$T/triaged.json" --root "$S"
  run python3 $KG/triage_load.py "$R" "$T/triaged.json" --commit "$FULL" --date "$DATE"
  offline python3 $KG/rmp_closings.py "$R" "$K/rmp_closings.json"
  run python3 $KG/commits_kg.py "$R" --closings "$K/rmp_closings.json"
  if [ "$MODE" = full ] || [ -n "$INDEX" ]; then join scip; fi
  run python3 $KG/rust_refs_kg.py "$R" --commit "$FULL" --refs "$REFS" "${REFS_SOURCE[@]}"
  run python3 $KG/no_caller_kg.py "$R" --commit "$FULL"
  run python3 $KG/no_caller_verdicts_kg.py "$R" --commit "$FULL" --date "$DATE"
  run python3 $KG/divergences_kg.py "$R" --source-root "$S" --commit "$FULL" --date "$DATE"
  # The port modules' own tests and resources, and their Rust counterparts.
  run python3 $KG/lucene_tests_kg.py "$R" --lucene-root "$LUCENE" --rt-root "$RT" --source-root "$S" --commit "$FULL" --date "$DATE" --phase framework
  offline python3 $KG/audit_kg.py "$R" --survey "$K/rucene_kg/survey.json" --lucene-root "$LUCENE" --rt-root "$RT" --source-root "$S" --commit "$FULL"
  # The marker `ask.py status` reads: written only once the audit has passed, so
  # a sync still running, or one that failed, never reads as current.
  # A positive-control run (`--pretend-unchanged`) leaves a graph it knows to be
  # wrong: it neither sets the marker nor records a state.
  if [ ${#PRETEND[@]} -gt 0 ]; then
    echo "positive control: syncedCommit not set, no state recorded"
  else
    run rmp graph client -r "$R" --query "MATCH (p:Project {name: 'Rucene'}) SET p.syncedCommit = '$FULL', p.syncedDate = '$DATE', p.gitCommit = '$FULL', p.gitDate = '$DATE'"
    # The state the next incremental sync starts from, written only after the
    # audit passed and the marker was set.
    offline python3 $KG/sync_plan.py record "$FULL" --roadmap "$R" --refs "$REFS"
  fi
  echo "SYNC_DONE $FULL ($MODE)"
} 2>&1 | tee "$LOG"
exit "${PIPESTATUS[0]}"
