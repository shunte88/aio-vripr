// The OffscreenCanvas arm. If this one wins, §44's waveform belongs in a worker
// and D6 needs a third clause saying so.
//
// The main thread posts only the new buckets — a transferred Int16Array, so the
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
