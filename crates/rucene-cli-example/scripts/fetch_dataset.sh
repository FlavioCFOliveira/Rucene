#!/usr/bin/env bash
# Download every USGS Earthquake Catalog event (all magnitudes) between
# 2019-01-01T00:00:00Z and 2025-01-01T00:00:00Z into
# data/usgs-2019-2024.csv, one request per calendar month.
#
# Usage: scripts/fetch_dataset.sh [--force]
#
# - Each window is checked with the FDSN `count` endpoint first. A month that
#   holds 20,000 events or more (the API's per-request cap) is split into two
#   half-months, and a half-month over the cap into days; a day over the cap
#   fails the script, so no window is ever silently truncated.
# - Every request is retried up to 5 times with a growing back-off.
# - Completed months are kept under data/parts/ so an interrupted run resumes.
# - The merged file has one header, one row per unique `id` (the row with the
#   latest `updated` wins), ordered by `time` then `id`.
# - The script fails unless the merged file holds at least 300,000 unique ids.
# - Idempotent: if data/usgs-2019-2024.csv exists, nothing is downloaded
#   unless --force is given (which also discards the month parts).
#
# USGS data is a work of the United States Government and is in the public
# domain.
set -euo pipefail

API="https://earthquake.usgs.gov/fdsnws/event/1"
START_YEAR=2019
END_YEAR=2024 # inclusive
MAX_PER_REQUEST=20000
MIN_UNIQUE=300000
HEADER='time,latitude,longitude,depth,mag,magType,nst,gap,dmin,rms,net,id,updated,place,type,horizontalError,depthError,magError,magNst,status,locationSource,magSource'

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
data_dir="$(cd "${script_dir}/../data" && pwd)"
out="${data_dir}/usgs-2019-2024.csv"
parts="${data_dir}/parts"

force=0
case "${1:-}" in
  "") ;;
  --force) force=1 ;;
  *) echo "usage: $0 [--force]" >&2; exit 2 ;;
esac

if [[ -f "${out}" && ${force} -eq 0 ]]; then
  echo "${out} exists; nothing to do (use --force to download again)"
  exit 0
fi
if [[ ${force} -eq 1 ]]; then
  rm -rf "${parts}"
fi
mkdir -p "${parts}"

# fetch URL DEST: download with retries; fails after the last attempt.
fetch() {
  local url="$1" dest="$2" attempt
  for attempt in 1 2 3 4 5; do
    if curl -fsS --connect-timeout 30 --max-time 600 -o "${dest}" "${url}"; then
      return 0
    fi
    echo "  attempt ${attempt} failed: ${url}" >&2
    sleep $((attempt * 10))
  done
  echo "error: giving up on ${url}" >&2
  return 1
}

# window START END LABEL LEVEL: download [START, END) into parts/LABEL.csv,
# splitting it when the count endpoint reports the cap or more.
# LEVEL 0 = month (split into half-months), 1 = half-month (split into days),
# 2 = day (no further split).
window() {
  local start="$1" end="$2" label="$3" level="$4"
  local part="${parts}/${label}.csv" count rows d next
  if [[ -f "${part}" ]]; then
    echo "${label}: kept $(($(wc -l < "${part}") - 1)) rows"
    return 0
  fi
  local query="starttime=${start}&endtime=${end}"
  fetch "${API}/count?${query}" "${part}.count"
  count=$(tr -d '[:space:]' < "${part}.count")
  rm -f "${part}.count"
  if ! [[ "${count}" =~ ^[0-9]+$ ]]; then
    echo "error: ${label}: count endpoint returned '${count}'" >&2
    exit 1
  fi
  if (( count >= MAX_PER_REQUEST )); then
    case "${level}" in
      0)
        echo "${label}: ${count} events, split into half-months"
        window "${start}" "${start:0:8}16T00:00:00" "${label}-a" 1
        window "${start:0:8}16T00:00:00" "${end}" "${label}-b" 1
        ;;
      1)
        echo "${label}: ${count} events, split into days"
        d="${start}"
        while [[ "${d}" < "${end}" ]]; do
          next="$(date -u -d "${d:0:10} +1 day" +%Y-%m-%dT00:00:00)"
          window "${d}" "${next}" "${d:0:10}" 2
          d="${next}"
        done
        ;;
      *)
        echo "error: ${label} holds ${count} events (>= ${MAX_PER_REQUEST}) in one day" >&2
        exit 1
        ;;
    esac
    return 0
  fi
  fetch "${API}/query?format=csv&${query}&orderby=time-asc" "${part}.tmp"
  if [[ "$(head -n 1 "${part}.tmp")" != "${HEADER}" ]]; then
    echo "error: ${label}: unexpected CSV header" >&2
    exit 1
  fi
  rows=$(($(wc -l < "${part}.tmp") - 1))
  if (( rows >= MAX_PER_REQUEST )); then
    echo "error: ${label}: ${rows} rows returned, the response may be truncated" >&2
    exit 1
  fi
  mv "${part}.tmp" "${part}"
  echo "${label}: ${rows} rows (count endpoint: ${count})"
}

for year in $(seq "${START_YEAR}" "${END_YEAR}"); do
  for month in $(seq 1 12); do
    start=$(printf '%04d-%02d-01T00:00:00' "${year}" "${month}")
    if [[ ${month} -eq 12 ]]; then
      end=$(printf '%04d-01-01T00:00:00' $((year + 1)))
    else
      end=$(printf '%04d-%02d-01T00:00:00' "${year}" $((month + 1)))
    fi
    window "${start}" "${end}" "$(printf '%04d-%02d' "${year}" "${month}")" 0
  done
done

# Merge, deduplicate by id (latest `updated` wins) and order deterministically.
python3 -I - "${parts}" "${out}.tmp" "${MIN_UNIQUE}" <<'PY'
import csv, pathlib, sys

parts, dest, min_unique = pathlib.Path(sys.argv[1]), sys.argv[2], int(sys.argv[3])
best, header, rows_in = {}, None, 0
for part in sorted(parts.glob("*.csv")):
    with part.open(encoding="utf-8", newline="") as f:
        reader = csv.reader(f)
        h = next(reader)
        if header is None:
            header = h
        elif h != header:
            sys.exit(f"error: {part}: header differs")
        i_id, i_upd = header.index("id"), header.index("updated")
        for lineno, row in enumerate(reader, start=2):
            if len(row) != len(header):
                sys.exit(f"error: {part}:{lineno}: {len(row)} fields, expected {len(header)}")
            rows_in += 1
            old = best.get(row[i_id])
            # ISO 8601 UTC timestamps of fixed shape compare correctly as text.
            if old is None or row[i_upd] >= old[i_upd]:
                best[row[i_id]] = row
i_time, i_id = header.index("time"), header.index("id")
with open(dest, "w", encoding="utf-8", newline="") as f:
    w = csv.writer(f, lineterminator="\n")
    w.writerow(header)
    w.writerows(sorted(best.values(), key=lambda r: (r[i_time], r[i_id])))
print(f"merged {rows_in} rows into {len(best)} unique ids ({rows_in - len(best)} duplicates removed)")
if len(best) < min_unique:
    sys.exit(f"error: {len(best)} unique ids < {min_unique}")
PY
mv "${out}.tmp" "${out}"
echo "wrote ${out}"
