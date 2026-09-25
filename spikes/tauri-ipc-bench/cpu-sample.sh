#!/usr/bin/env bash
#  cpu-sample.sh
#
#  VCW - The Vinyl Capture Workstation
#  (c) 2026 Stue Hunter
#
#  Whole-app CPU for a running bench, sampled from outside it.
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

# Whole-app CPU for a running bench, sampled from outside it.
#
# The bench times code *inside* its own rAF callback and inside `send()`.
# Neither captures the GTK main thread dispatching evals, nor the web process
# painting, so a run can report "0.5% of the main thread" while the application
# costs most of a core. This samples the truth from /proc, per process and per
# thread of the tauri process.
#
# usage: cpu-sample.sh [seconds] [label]
set -euo pipefail
WINDOW="${1:-20}"
LABEL="${2:-sample}"

main=$(pgrep -f 'release/tauri-ipc-bench' | head -1) \
  || { echo "no bench process running" >&2; exit 1; }

# utime + stime, in clock ticks (100/s on Linux). Fields 14 and 15 of
# /proc/<pid>/stat -- read from the end, because comm (field 2) can contain
# spaces and parens and would otherwise shift every later field.
ticks() { awk -F')' '{split($NF,f," "); print f[12]+f[13]}' "$1/stat" 2>/dev/null || echo 0; }
# comm can contain spaces, so tab-delimit rather than space-delimit.
comm_of() { tr -d '\0' < "$1/comm" 2>/dev/null || echo '?'; }

pids=$(pgrep -f 'release/tauri-ipc-bench|WebKitWebProcess|WebKitNetworkProcess' | tr '\n' ' ')

declare -A p0 t0 tname
for p in $pids; do p0[$p]=$(ticks "/proc/$p"); done
for t in /proc/$main/task/*; do
  tid=$(basename "$t"); t0[$tid]=$(ticks "$t"); tname[$tid]=$(comm_of "$t")
done

sleep "$WINDOW"

rate() { awk -v d="$1" -v w="$WINDOW" 'BEGIN{printf "%.1f", d/w}'; }

echo "## $LABEL - whole-app CPU over ${WINDOW}s (% of one core)"
echo
printf '%-32s %7s\n' "process" "%core"
total=0
for p in $pids; do
  [ -d "/proc/$p" ] || continue
  d=$(( $(ticks "/proc/$p") - ${p0[$p]:-0} ))
  total=$(( total + d ))
  printf '%-32s %6s%%\n' "$(comm_of "/proc/$p") ($p)" "$(rate "$d")"
done
printf '%-32s %6s%%\n' "TOTAL" "$(rate "$total")"

echo
printf '%-32s %7s\n' "thread of tauri process" "%core"
for tid in "${!t0[@]}"; do
  [ -d "/proc/$main/task/$tid" ] || continue
  d=$(( $(ticks "/proc/$main/task/$tid") - ${t0[$tid]} ))
  [ "$d" -gt 0 ] || continue
  printf '%-32s %6s%%\n' "${tname[$tid]}" "$(rate "$d")"
done | sort -t' ' -k2 -nr
