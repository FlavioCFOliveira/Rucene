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
LOG=$K/kg_sync_$SHORT.log
mkdir -p "$K"
rm -rf "$S" && mkdir -p "$S" && git archive "$FULL" | tar -x -C "$S" || exit 2

run() {
  echo "=== $(date +%s) $*"
  "$@"
  local rc=$?
  echo "exit=$rc t=$(date +%s)"
  if [ $rc -ne 0 ]; then echo "STOPPED at: $*"; exit $rc; fi
}

KG=tools/kg
{
  echo "sync $FULL ($DATE) roadmap=$R"
  run python3 $KG/extract_rucene_kg.py --source-root "$S" --expand --commit "$FULL" --date "$DATE" --output "$K/rucene_kg/survey.json"
  run python3 $KG/load_rucene_kg.py "$R" --survey "$K/rucene_kg/survey.json"
  run python3 $KG/port_coverage_kg.py "$R" --survey "$K/rucene_kg/survey.json" --lucene-root "$LUCENE" --commit "$FULL" --date "$DATE"
  run python3 $KG/port_evidence_kg.py "$R" --source-root "$S" --phase extract
  run python3 $KG/port_evidence_kg.py "$R" --source-root "$S" --phase load --commit "$FULL" --date "$DATE"
  run python3 $KG/expanded_evidence_kg.py "$R" --source-root "$S" --commit "$FULL" --date "$DATE"
  run python3 $KG/port_evidence_kg.py "$R" --source-root "$S" --phase depth --commit "$FULL" --date "$DATE"
  run python3 $KG/nested_port_kg.py "$R" --lucene-root "$LUCENE" --commit "$FULL" --date "$DATE"
  run python3 $KG/port_coverage_kg.py "$R" --survey "$K/rucene_kg/survey.json" --lucene-root "$LUCENE" --commit "$FULL" --date "$DATE" --phase scope
  run python3 $KG/port_coverage_kg.py "$R" --survey "$K/rucene_kg/survey.json" --lucene-root "$LUCENE" --commit "$FULL" --date "$DATE" --phase candidates
  # The member triage: `memberAbsent` is the number to plan from, so it is
  # re-derived at every sync from the floor the depth phase just measured.
  run python3 $KG/triage_dump.py "$R" "$T"
  run python3 $KG/triage_repro.py "$T" "$T/unmatched.json"
  run python3 $KG/triage_members.py "$T" "$T/triaged.json" --root "$S"
  run python3 $KG/triage_load.py "$R" "$T/triaged.json" --commit "$FULL" --date "$DATE"
  run python3 $KG/rmp_closings.py "$R" "$K/rmp_closings.json"
  run python3 $KG/commits_kg.py "$R" --closings "$K/rmp_closings.json"
  run python3 $KG/rust_refs_kg.py "$R" --commit "$FULL"
  run python3 $KG/no_caller_kg.py "$R" --commit "$FULL"
  run python3 $KG/no_caller_verdicts_kg.py "$R" --commit "$FULL" --date "$DATE"
  run python3 $KG/divergences_kg.py "$R" --source-root "$S" --commit "$FULL" --date "$DATE"
  run python3 $KG/audit_kg.py "$R" --survey "$K/rucene_kg/survey.json" --lucene-root "$LUCENE" --source-root "$S" --commit "$FULL"
  echo "SYNC_DONE $FULL"
} 2>&1 | tee "$LOG"
exit "${PIPESTATUS[0]}"
