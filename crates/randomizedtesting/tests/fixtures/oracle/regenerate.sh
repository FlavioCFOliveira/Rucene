#!/usr/bin/env bash
# Rebuilds every fixture of crates/randomizedtesting from the Java oracle.
#
# Requirements: JDK 25 on PATH (`java`; javac is run as the jdk.compiler
# module), and in ~/.m2 (or $M2_REPO): randomizedtesting-runner 2.8.1,
# junit 4.13.1, hamcrest-core 1.3; for self_tests.tsv also assertj-core 2.2.0
# and RANDOMIZEDTESTING_SRC, a checkout of randomizedtesting tag release/2.8.1
# (without it SelfTestsOracle keeps the committed self_tests.tsv).
#
# Usage: [RANDOMIZEDTESTING_SRC=dir] crates/randomizedtesting/tests/fixtures/oracle/regenerate.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
fixtures="$(dirname "$here")"
crate="$(dirname "$(dirname "$fixtures")")"
m2="${M2_REPO:-$HOME/.m2/repository}"
cp="$m2/com/carrotsearch/randomizedtesting/randomizedtesting-runner/2.8.1/randomizedtesting-runner-2.8.1.jar"
cp="$cp:$m2/junit/junit/4.13.1/junit-4.13.1.jar:$m2/org/hamcrest/hamcrest-core/1.3/hamcrest-core-1.3.jar"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mapfile -t sources < <(find "$here/src" -name '*.java' | sort)
java -m jdk.compiler/com.sun.tools.javac.Main -nowarn -cp "$cp" -d "$work/classes" "${sources[@]}"
# randomized-runner's own test classes (SelfTestsOracle runs them).
rt_src="${RANDOMIZEDTESTING_SRC:-}"
if [ -n "$rt_src" ]; then
  cp="$cp:$m2/org/assertj/assertj-core/2.2.0/assertj-core-2.2.0.jar"
  mapfile -t rt_tests < <(find "$rt_src/randomized-runner/src/test/java" -name '*.java' | sort)
  java -m jdk.compiler/com.sun.tools.javac.Main -nowarn -cp "$cp" -d "$work/rt-tests" "${rt_tests[@]}"
  cp="$cp:$work/rt-tests"
fi
mkdir -p "$work/out"
grep -v '^#' "$here/oracles.txt" | while read -r main extra; do
  [ -n "$main" ] || continue
  extra="${extra//@RANDOMIZEDTESTING_SRC@/$rt_src}"
  # --add-opens: ZigguratOracle reads RandomSupport.DoubleZigguratTables.
  # shellcheck disable=SC2086 # `extra` holds the further arguments.
  java -ea --add-opens java.base/jdk.internal.util.random=ALL-UNNAMED \
    -cp "$work/classes:$cp" "$main" "$work/out" $extra 2>/dev/null
done
cp "$work/out"/*.tsv "$fixtures/"
# Data the crate itself embeds (JDK 25 lists), not test fixtures.
for f in available_locales.txt available_time_zones.txt bmp_decimal_zeros.txt ziggurat_tables.rs; do
  [ -f "$work/out/$f" ] && cp "$work/out/$f" "$crate/src/jdk/data/$f"
done
echo "fixtures regenerated in $fixtures"
