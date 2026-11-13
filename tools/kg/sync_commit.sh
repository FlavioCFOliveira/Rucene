#!/usr/bin/env bash
# Sync the knowledge graph with one commit, end to end, and audit it.
#
#   tools/kg/sync_commit.sh <commit> [roadmap]
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
COMMIT_ARG=${1:?usage: sync_commit.sh <commit> [roadmap]}
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

# A step that writes to the graph is followed by a checkpoint (`run`); one that
# only reads the graph or works off it (`offline`) is not, since it changed
# nothing a later scan could miss.
run() {
  echo "=== $(date +%s) $*"
  "$@"
  local rc=$?
  echo "exit=$rc t=$(date +%s)"
  if [ $rc -ne 0 ]; then echo "STOPPED at: $*"; exit $rc; fi
  checkpoint
}

offline() {
  echo "=== $(date +%s) $*"
  "$@"
  local rc=$?
  echo "exit=$rc t=$(date +%s)"
  if [ $rc -ne 0 ]; then echo "STOPPED at: $*"; exit $rc; fi
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
  ( "$@" > "$K/bg_${name}_$SHORT.log" 2>&1 ) &
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
  # The port modules first: their Module scope, JPMS dependencies, scope
  # decisions, and the packages, files, types and members of every module
  # beyond lucene/core, which port_coverage_kg.py scopes below.
  bg survey python3 $KG/extract_rucene_kg.py --source-root "$S" --expand --commit "$FULL" --date "$DATE" --output "$K/rucene_kg/survey.json"
  bg scip python3 $KG/rust_refs_kg.py "$R" --commit "$FULL" --phase scip --scip "$K/no_caller/index_$SHORT.scip"
  run python3 $KG/port_modules_kg.py "$R" --lucene-root "$LUCENE" --rt-root "$RT" --commit "$FULL" --date "$DATE" --phase modules
  run python3 $KG/port_modules_kg.py "$R" --lucene-root "$LUCENE" --rt-root "$RT" --commit "$FULL" --date "$DATE" --phase structure
  join survey
  run python3 $KG/load_rucene_kg.py "$R" --survey "$K/rucene_kg/survey.json"
  run python3 $KG/port_coverage_kg.py "$R" --survey "$K/rucene_kg/survey.json" --lucene-root "$LUCENE" --rt-root "$RT" --commit "$FULL" --date "$DATE"
  offline python3 $KG/port_evidence_kg.py "$R" --source-root "$S" --phase extract
  run python3 $KG/port_evidence_kg.py "$R" --source-root "$S" --phase load --commit "$FULL" --date "$DATE"
  run python3 $KG/expanded_evidence_kg.py "$R" --source-root "$S" --commit "$FULL" --date "$DATE"
  run python3 $KG/port_evidence_kg.py "$R" --source-root "$S" --phase depth --commit "$FULL" --date "$DATE"
  run python3 $KG/nested_port_kg.py "$R" --lucene-root "$LUCENE" --commit "$FULL" --date "$DATE"
  run python3 $KG/port_coverage_kg.py "$R" --survey "$K/rucene_kg/survey.json" --lucene-root "$LUCENE" --rt-root "$RT" --commit "$FULL" --date "$DATE" --phase scope
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
  join scip
  run python3 $KG/rust_refs_kg.py "$R" --commit "$FULL" --scip "$K/no_caller/index_$SHORT.scip"
  run python3 $KG/no_caller_kg.py "$R" --commit "$FULL"
  run python3 $KG/no_caller_verdicts_kg.py "$R" --commit "$FULL" --date "$DATE"
  run python3 $KG/divergences_kg.py "$R" --source-root "$S" --commit "$FULL" --date "$DATE"
  offline python3 $KG/audit_kg.py "$R" --survey "$K/rucene_kg/survey.json" --lucene-root "$LUCENE" --rt-root "$RT" --source-root "$S" --commit "$FULL"
  # The marker `ask.py status` reads: written only once the audit has passed, so
  # a sync still running, or one that failed, never reads as current.
  run rmp graph client -r "$R" --query "MATCH (p:Project {name: 'Rucene'}) SET p.syncedCommit = '$FULL', p.syncedDate = '$DATE', p.gitCommit = '$FULL', p.gitDate = '$DATE'"
  echo "SYNC_DONE $FULL"
} 2>&1 | tee "$LOG"
exit "${PIPESTATUS[0]}"
