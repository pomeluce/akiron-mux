#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 /path/to/tauri-binary /path/to/gpui-binary" >&2
  exit 2
fi

tauri_binary=$1
gpui_binary=$2
runs=${AKMUX_BENCH_RUNS:-7}
idle_seconds=${AKMUX_BENCH_IDLE_SECONDS:-10}

if [[ ! -x $tauri_binary || ! -x $gpui_binary ]]; then
  echo "both release binaries must exist and be executable" >&2
  exit 2
fi

measure() {
  local name=$1
  local binary=$2
  local results
  results=$(mktemp)
  local rss_results
  rss_results=$(mktemp)
  trap 'rm -f "$results" "$rss_results"' RETURN

  for ((run = 1; run <= runs; run++)); do
    local marker
    marker=$(mktemp)
    rm -f "$marker"
    local started
    started=$(date +%s%N)
    AKMUX_STARTUP_MARKER=$marker "$binary" >/dev/null 2>&1 &
    local pid=$!
    local deadline=$((SECONDS + 30))
    while [[ ! -f $marker ]]; do
      if ! kill -0 "$pid" 2>/dev/null; then
        echo "$name exited before opening its window" >&2
        wait "$pid" || true
        exit 1
      fi
      if ((SECONDS >= deadline)); then
        echo "$name did not open a window within 30 seconds" >&2
        kill "$pid" 2>/dev/null || true
        wait "$pid" || true
        exit 1
      fi
      sleep 0.01
    done
    local ready
    ready=$(date +%s%N)
    awk -v start="$started" -v end="$ready" 'BEGIN { printf "%.3f\n", (end-start)/1000000 }' >>"$results"
    sleep "$idle_seconds"
    awk '/VmRSS:/ { print $2 }' "/proc/$pid/status" >>"$rss_results"
    kill "$pid" 2>/dev/null || true
    wait "$pid" || true
    rm -f "$marker"
  done

  local startup
  startup=$(awk '{ total += $1 } END { printf "%.3f", total/NR }' "$results")
  local rss
  rss=$(awk '{ total += $1 } END { printf "%.0f", total/NR }' "$rss_results")
  echo "$name startup_ms=$startup idle_rss_kib=$rss"
  printf '%s %s\n' "$startup" "$rss"
}

tauri_measurement=$(measure tauri "$tauri_binary")
gpui_measurement=$(measure gpui "$gpui_binary")
printf '%s\n%s\n' "$tauri_measurement" "$gpui_measurement"
tauri_result=$(tail -n 1 <<<"$tauri_measurement")
gpui_result=$(tail -n 1 <<<"$gpui_measurement")
read -r tauri_start tauri_rss <<<"$tauri_result"
read -r gpui_start gpui_rss <<<"$gpui_result"

awk -v ts="$tauri_start" -v gs="$gpui_start" -v tr="$tauri_rss" -v gr="$gpui_rss" 'BEGIN {
  start_gain = (ts-gs)/ts*100;
  rss_gain = (tr-gr)/tr*100;
  printf "cold-start improvement: %.1f%%\nidle RSS improvement: %.1f%%\n", start_gain, rss_gain;
  if (start_gain < 30 || rss_gain < 30) exit 1;
}'
