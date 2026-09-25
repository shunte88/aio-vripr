/*
 *  wave-worker.ts
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  The OffscreenCanvas arm. If this one wins, §44's waveform belongs in a
 *  worker
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

// The OffscreenCanvas arm. If this one wins, §44's waveform belongs in a worker
// and D6 needs a third clause saying so.
//
// The main thread posts only the new buckets - a transferred Int16Array, so the
// handoff is a pointer move and not a copy. The worker owns the canvas and the
// ring, and its own rAF drives the draw, so a busy main thread cannot stall it.

import { IncrementalWave, NaiveWave, WaveStore } from './draw'
import { Hist } from './stats'

type Init = { kind: 'init'; canvas: OffscreenCanvas; mode: 'naive' | 'incremental' }
type Buckets = { kind: 'buckets'; buckets: Int16Array; len: number }
type Report = { kind: 'report' }
type Msg = Init | Buckets | Report

let store: WaveStore | null = null
let renderer: { frame(s: WaveStore): void } | null = null
const draw = new Hist()
const interval = new Hist()
let last = 0
let frames = 0

function tick(): void {
  const t = performance.now()
  if (last) interval.record((t - last) * 1000)
  last = t
  if (store && renderer) {
    renderer.frame(store)
    draw.record((performance.now() - t) * 1000)
    frames++
  }
  requestAnimationFrame(tick)
}

self.onmessage = (e: MessageEvent<Msg>) => {
  const m = e.data
  if (m.kind === 'init') {
    const g = m.canvas.getContext('2d', { alpha: false })
    if (!g) throw new Error('no 2d context in the worker')
    store = new WaveStore()
    // The renderers take a CanvasRenderingContext2D; the Offscreen variant has
    // the same drawing surface for everything used here.
    renderer =
      m.mode === 'naive'
        ? new NaiveWave(g as unknown as CanvasRenderingContext2D)
        : new IncrementalWave(g as unknown as CanvasRenderingContext2D)
    requestAnimationFrame(tick)
    return
  }
  if (m.kind === 'buckets') {
    store?.append({ seq: 0, tUs: 0, level: 0, startBucket: 0, buckets: m.buckets, len: m.len })
    return
  }
  if (m.kind === 'report') {
    self.postMessage({
      kind: 'report',
      frames,
      drawUs: draw.summary(),
      frameIntervalUs: interval.summary(),
    })
  }
}
