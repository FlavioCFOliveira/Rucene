#!/usr/bin/env bash
# Build tools/test/ptracer_exec.c into the helper that ~/.local/bin/rucene-mem-runner
# execs inside each test's scope (falls back to ptracer_exec.py when absent).
# Rebuilds only when the source is newer than the binary; the binary is moved
# into place atomically, so a concurrent test launch never sees a partial file.
# Output: $RUCENE_PTRACER_EXEC (default: outside the repo, in the shared
# cargo target directory). Usage: tools/test/build_ptracer_exec.sh
set -eu
SRC=$(cd "$(dirname "$0")" && pwd)/ptracer_exec.c
OUT=${RUCENE_PTRACER_EXEC:-/data/dev/github.com/FlavioCFOliveira/.rucene-dev-target/rucene-tools/ptracer_exec}
if [ -x "$OUT" ] && [ ! "$SRC" -nt "$OUT" ]; then
  exit 0
fi
mkdir -p "$(dirname "$OUT")"
TMP=$(mktemp "$OUT.XXXXXX")
trap 'rm -f "$TMP"' EXIT
${CC:-cc} -O2 -Wall -Wextra -Werror -o "$TMP" "$SRC"
chmod 755 "$TMP"
mv -f "$TMP" "$OUT"
trap - EXIT
echo "built $OUT"
