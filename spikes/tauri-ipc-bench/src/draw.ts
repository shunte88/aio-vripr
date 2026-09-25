/*
 *  draw.ts
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  The canvas half of the spike. §44 wants VU meters and a waveform that
 *  grows
 *
 * MIT License
 *
 * Copyright (c) 2026 Stue Hunter
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 *
 */

// The canvas half of the spike. §44 wants VU meters and a waveform that grows
// as the capture runs, and the open question D6 leaves is whether that drawing
// belongs on the main thread at all.
//
// One bucket per pixel column, so appending a bucket scrolls the view by
// exactly one pixel. That makes the naive/incremental comparison honest: the
// incremental renderer's only advantage is that it redraws the columns that
// changed instead of all of them, with no resampling difference to muddy it.

import type { Wave } from './decode'

export const WAVE_W = 1400
export const WAVE_H = 220

/** Ring of level-0 buckets, interleaved [minL, maxL, minR, maxR]. */
export class WaveStore {
  private readonly ring: Int16Array
  private readonly cap: number
  /** Total buckets ever appended; the ring holds the last `cap` of them. */
  total = 0

  constructor(cap = WAVE_W * 4) {
    this.cap = cap
    this.ring = new Int16Array(cap * 4)
  }

  append(w: Wave): number {
    const n = w.len >> 2
    for (let i = 0; i < n; i++) {
      const o = ((this.total + i) % this.cap) * 4
      const s = (i << 2)
      this.ring[o] = w.buckets[s]
      this.ring[o + 1] = w.buckets[s + 1]
      this.ring[o + 2] = w.buckets[s + 2]
      this.ring[o + 3] = w.buckets[s + 3]
    }
    this.total += n
    return n
  }

  /** Read bucket `i` (absolute index) into `out`; false if it has aged out. */
  at(i: number, out: Int16Array): boolean {
    if (i < 0 || i >= this.total || i < this.total - this.cap) return false
    const o = (i % this.cap) * 4
    out[0] = this.ring[o]
    out[1] = this.ring[o + 1]
    out[2] = this.ring[o + 2]
    out[3] = this.ring[o + 3]
    return true
  }
}

const SCALE = WAVE_H / 4 / 32768

/** Draws columns [from, to) of the view whose right edge is bucket `total`. */
export function drawColumns(
  g: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D,
  store: WaveStore,
  from: number,
  to: number,
  scratch: Int16Array,
): void {
  const first = store.total - WAVE_W
  g.fillStyle = '#12121a'
  g.fillRect(from, 0, to - from, WAVE_H)
  const midL = WAVE_H / 4
  const midR = (WAVE_H * 3) / 4
  g.fillStyle = '#4ea3d6'
  for (let x = from; x < to; x++) {
    if (!store.at(first + x, scratch)) continue
    const l0 = midL + scratch[0] * SCALE
    const l1 = midL + scratch[1] * SCALE
    g.fillRect(x, Math.min(l0, l1), 1, Math.max(1, Math.abs(l1 - l0)))
  }
  g.fillStyle = '#d68f4e'
  for (let x = from; x < to; x++) {
    if (!store.at(first + x, scratch)) continue
    const r0 = midR + scratch[2] * SCALE
    const r1 = midR + scratch[3] * SCALE
    g.fillRect(x, Math.min(r0, r1), 1, Math.max(1, Math.abs(r1 - r0)))
  }
}

/** Redraws the entire view every frame. The straw man, and the default anyone
 * reaches for first. */
export class NaiveWave {
  private readonly scratch = new Int16Array(4)
  constructor(private readonly g: CanvasRenderingContext2D) {}
  frame(store: WaveStore): void {
    drawColumns(this.g, store, 0, WAVE_W, this.scratch)
  }
}

/** Shifts the existing pixels left with `drawImage` and draws only the new
 * columns. */
export class IncrementalWave {
  private readonly scratch = new Int16Array(4)
  private drawn = -1
  constructor(private readonly g: CanvasRenderingContext2D) {}
  frame(store: WaveStore): void {
    if (this.drawn < 0) {
      drawColumns(this.g, store, 0, WAVE_W, this.scratch)
      this.drawn = store.total
      return
    }
    const shift = Math.min(WAVE_W, store.total - this.drawn)
    if (shift <= 0) return
    // Self-blit. On WebKitGTK this is the operation most likely to fall off the
    // accelerated path, so it is the one worth measuring rather than assuming.
    this.g.drawImage(this.g.canvas, -shift, 0)
    drawColumns(this.g, store, WAVE_W - shift, WAVE_W, this.scratch)
    this.drawn = store.total
  }
}

const METER_W = 180
const METER_H = 220

/** Peak/RMS bars with a decaying peak-hold, which is what §44 specifies and
 * also the cheapest possible canvas work - the control against which the
 * waveform's cost is read. */
export class MeterView {
  private holdL = 0
  private holdR = 0
  constructor(private readonly g: CanvasRenderingContext2D) {}

  frame(peakL: number, peakR: number, rmsL: number, rmsR: number, clip: number): void {
    const g = this.g
    g.fillStyle = '#12121a'
    g.fillRect(0, 0, METER_W, METER_H)
    this.holdL = Math.max(peakL, this.holdL - 0.01)
    this.holdR = Math.max(peakR, this.holdR - 0.01)
    const bars: Array<[number, number, number, number]> = [
      [20, peakL, rmsL, this.holdL],
      [100, peakR, rmsR, this.holdR],
    ]
    for (const [x, peak, rms, hold] of bars) {
      const ph = Math.min(1, peak) * (METER_H - 20)
      const rh = Math.min(1, rms) * (METER_H - 20)
      g.fillStyle = '#2f6d4a'
      g.fillRect(x, METER_H - 10 - ph, 60, ph)
      g.fillStyle = '#5ec98a'
      g.fillRect(x, METER_H - 10 - rh, 60, rh)
      g.fillStyle = '#e0e0e8'
      g.fillRect(x, METER_H - 11 - Math.min(1, hold) * (METER_H - 20), 60, 2)
    }
    if (clip) {
      g.fillStyle = '#d64e4e'
      if (clip & 1) g.fillRect(20, 2, 60, 6)
      if (clip & 2) g.fillRect(100, 2, 60, 6)
    }
  }
}
