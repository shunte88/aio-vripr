#!/usr/bin/env bash
#  cpu-matrix.sh
#
#  VCW - The Vinyl Capture Workstation
#  (c) 2026 Stue Hunter
#
#  Whole-app CPU per arm - the comparison the bench itself cannot make.
#
# MIT License
#
# Copyright (c) 2026 Stue Hunter
#
# Permission is hereby granted, free of charge, to any person obtaining a copy
# of this software and associated documentation files (the "Software"), to deal
# in the Software without restriction, including without limitation the rights
# to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
# copies of the Software, and to permit persons to whom the Software is
# furnished to do so, subject to the following conditions:
#
# The above copyright notice and this permission notice shall be included in all
# copies or substantial portions of the Software.
#
# THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
# IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
# FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
# AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
# LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
# OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
# SOFTWARE.
#

# Whole-app CPU per arm - the comparison the bench itself cannot make.
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
