#!/usr/bin/env bash
# Rebuilds every fixture of crates/randomizedtesting from the Java oracle.
#
# Requirements: JDK 25 on PATH (`java`; javac is run as the jdk.compiler
# module), and in ~/.m2 (or $M2_REPO): randomizedtesting-runner 2.8.1,
# junit 4.13.1, hamcrest-core 1.3; for the Lucene oracles (classes under
# org.apache.lucene) also lucene-core, lucene-codecs and lucene-test-framework
# 10.5.0 and hamcrest 2.2; for self_tests.tsv also assertj-core 2.2.0
# and RANDOMIZEDTESTING_SRC, a checkout of randomizedtesting tag release/2.8.1
# (without it SelfTestsOracle keeps the committed self_tests.tsv); for
# framework_tests.tsv LUCENE_SRC, a checkout of Lucene tag
# releases/lucene/10.5.0 (without it FrameworkTestsOracle keeps the committed
# file).
#
# Usage: [RANDOMIZEDTESTING_SRC=dir] [LUCENE_SRC=dir] crates/randomizedtesting/tests/fixtures/oracle/regenerate.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
fixtures="$(dirname "$here")"
crate="$(dirname "$(dirname "$fixtures")")"
m2="${M2_REPO:-$HOME/.m2/repository}"
cp="$m2/com/carrotsearch/randomizedtesting/randomizedtesting-runner/2.8.1/randomizedtesting-runner-2.8.1.jar"
junit="$m2/junit/junit/4.13.1/junit-4.13.1.jar"
# lucene-core, lucene-codecs, lucene-test-framework: the class-path order of the
# port's class loaders (each a child of the previous one, parents scanned
# first), so that SPI scans list the providers in the order the crates do
# (`availableKnnVectorsFormats()` behind `LuceneTestCase.randomVectorFormat`).
lucene="$m2/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar"
lucene="$lucene:$m2/org/apache/lucene/lucene-codecs/10.5.0/lucene-codecs-10.5.0.jar"
lucene="$lucene:$m2/org/apache/lucene/lucene-test-framework/10.5.0/lucene-test-framework-10.5.0.jar"
# The Lucene oracles run on Lucene 10.5.0's own Hamcrest (2.2); the others keep
# junit 4.13.1's hamcrest-core 1.3.
lucene_cp="$cp:$junit:$m2/org/hamcrest/hamcrest/2.2/hamcrest-2.2.jar:$lucene"
cp="$cp:$junit:$m2/org/hamcrest/hamcrest-core/1.3/hamcrest-core-1.3.jar"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mapfile -t sources < <(find "$here/src" -name '*.java' | sort)
java -m jdk.compiler/com.sun.tools.javac.Main -nowarn -cp "$lucene_cp" -d "$work/classes" "${sources[@]}"
# randomized-runner's own test classes (SelfTestsOracle runs them).
rt_src="${RANDOMIZEDTESTING_SRC:-}"
if [ -n "$rt_src" ]; then
  cp="$cp:$m2/org/assertj/assertj-core/2.2.0/assertj-core-2.2.0.jar"
  mapfile -t rt_tests < <(find "$rt_src/randomized-runner/src/test/java" -name '*.java' | sort)
  java -m jdk.compiler/com.sun.tools.javac.Main -nowarn -cp "$cp" -d "$work/rt-tests" "${rt_tests[@]}"
  cp="$cp:$work/rt-tests"
fi
# Lucene's own lucene/test-framework tests (FrameworkTestsOracle runs them).
lucene_src="${LUCENE_SRC:-}"
if [ -n "$lucene_src" ]; then
  mapfile -t lt_tests < <(find "$lucene_src/lucene/test-framework/src/test" -name '*.java' | sort)
  java -m jdk.compiler/com.sun.tools.javac.Main -nowarn -cp "$lucene_cp" -d "$work/lucene-tests" "${lt_tests[@]}"
  lucene_cp="$lucene_cp:$work/lucene-tests"
fi
mkdir -p "$work/out"
grep -v '^#' "$here/oracles.txt" | while read -r main extra; do
  [ -n "$main" ] || continue
  extra="${extra//@RANDOMIZEDTESTING_SRC@/$rt_src}"
  main_cp="$cp"
  case "$main" in org.apache.lucene.*) main_cp="$lucene_cp" ;; esac
  # --add-opens: ZigguratOracle reads RandomSupport.DoubleZigguratTables,
  # TimeZoneOracle the transition table of sun.util.calendar.ZoneInfo.
  # shellcheck disable=SC2086 # `extra` holds the further arguments.
  java -ea --add-opens java.base/jdk.internal.util.random=ALL-UNNAMED \
    --add-opens java.base/sun.util.calendar=ALL-UNNAMED \
    -cp "$work/classes:$main_cp" "$main" "$work/out" $extra 2>/dev/null
done
cp "$work/out"/*.tsv "$fixtures/"
# Data the crate itself embeds (JDK 25 lists), not test fixtures.
for f in available_locales.txt available_time_zones.txt bmp_decimal_zeros.txt ziggurat_tables.rs; do
  [ -f "$work/out/$f" ] && cp "$work/out/$f" "$crate/src/jdk/data/$f"
done
# The fixtures of the other crates of the workspace, written by the Lucene
# oracles under out/<crate>/: *.tsv to <crate>/tests/fixtures/, the JDK data
# lists to <crate>/src/jdk/data/.
for dir in "$work/out"/*/; do
  [ -d "$dir" ] || continue
  other="$(dirname "$crate")/$(basename "$dir")"
  mkdir -p "$other/tests/fixtures"
  cp "$dir"*.tsv "$other/tests/fixtures/"
  for f in available_time_zone_ids.txt; do
    [ -f "$dir$f" ] && cp "$dir$f" "$other/src/jdk/data/$f"
  done
  echo "fixtures regenerated in $other/tests/fixtures"
done
echo "fixtures regenerated in $fixtures"
