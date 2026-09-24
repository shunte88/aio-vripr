#!/usr/bin/env bash
# Whole-app CPU per arm — the comparison the bench itself cannot make.
#
# The in-bench figures are all time inside a callback it owns, so they cannot
# see the GTK main thread dispatching evals or the web process painting. This
# runs a short arm at a time, samples /proc across a steady-state window in the
# middle of it, and prints one table per arm.
#
# The arms are chosen to separate the costs: control-raf is the do-nothing
# floor, control-nodraw adds traffic with no drawing (so the difference is eval
# dispatch), and the three renderers add drawing on top.
set -euo pipefail
cd "$(dirname "$0")/../.."

ARMS=(${S3_CPU_ARMS:-control-raf control-nodraw channel-manual render-naive render-worker})
SECS="${S3_CPU_SECS:-75}"
WARMUP="${S3_CPU_WARMUP:-20}"
WINDOW="${S3_CPU_WINDOW:-40}"
OUT="${S3_CPU_OUT:-.bench/s3-cpu.md}"

: > "$OUT"
echo "# Whole-app CPU per arm (external /proc sampling)" >> "$OUT"
echo >> "$OUT"
echo "Each arm runs ${SECS}s; sampling skips the first ${WARMUP}s and covers ${WINDOW}s of steady state." >> "$OUT"
echo >> "$OUT"

for arm in "${ARMS[@]}"; do
  echo "=== $arm" >&2
  DISPLAY=:0 S3_AUTORUN="$arm" S3_SECS="$SECS" \
    ./target/release/tauri-ipc-bench > "/tmp/s3-cpu-$arm.log" 2>&1 &
  app=$!
  # Wait for the window to actually be up before starting the clock.
  for _ in $(seq 60); do
    grep -q 'arm start' "/tmp/s3-cpu-$arm.log" 2>/dev/null && break
    sleep 1
  done
  sleep "$WARMUP"
  ./spikes/tauri-ipc-bench/cpu-sample.sh "$WINDOW" "$arm" >> "$OUT" || true
  echo >> "$OUT"
  wait "$app" 2>/dev/null || true
done

echo "wrote $OUT" >&2
