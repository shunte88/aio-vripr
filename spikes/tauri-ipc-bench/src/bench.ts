/*
 *  bench.ts
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  The run driver. One function per run, no React inside it: everything on
 *  the
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

// The run driver. One function per run, no React inside it: everything on the
// hot path here is imperative so that the React arm's cost is visible as a
// difference rather than baked into every arm.

import { Channel, invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

import {
  decodeMeter,
  decodePosition,
  decodeWave,
  newMeter,
  newPosition,
  newWave,
  type Encoding,
  type Transport,
} from './decode'
import { IncrementalWave, MeterView, NaiveWave, WAVE_W, WaveStore } from './draw'
import { Counters, Hist, type HistSummary } from './stats'

export type RenderMode = 'none' | 'naive' | 'incremental' | 'worker' | 'react-dom'

export interface BenchConfig {
  transport: Transport
  encoding: Encoding
  meterHz: number
  waveHz: number
  positionHz: number
  coalesce: boolean
  captureRate: number
  callbackFrames: number
  bucketFrames: number
  durationSecs: number
}

/**
 * One periodic sample. Cumulative rather than per-interval: differencing is
 * the analysis script's job, and cumulative values cannot silently lose a
 * sample's worth of counts.
 */
export interface TimelineSample {
  atSecs: number
  rssTotalKib: number
  processCount: number
  frames: number
  jank20: number
  jank33: number
  meterRecv: number
  drawMeanUs: number
}

export interface RunOptions {
  cfg: BenchConfig
  renderMode: RenderMode
  meterCanvas: HTMLCanvasElement
  waveCanvas: HTMLCanvasElement
  /** Phase timings, so a run that takes far longer than `durationSecs` says
   * where the time went instead of leaving it to be guessed at. */
  say?: (m: string) => void
  /** Called in the rAF callback with the latest meter values; the React arm
   * uses this to drive state, every other arm leaves it undefined. */
  onFrame?: (peakL: number, peakR: number, rmsL: number, rmsR: number, clip: number) => void
  onProgress?: (elapsedSecs: number) => void
}

export interface ClientReport {
  renderMode: RenderMode
  /**
   * Granularity of `performance.now()` as actually observed, in milliseconds.
   * WebKit clamps it for Spectre reasons; every client-side duration in this
   * report has this as its floor, so it is reported rather than assumed.
   */
  clockResolutionMs: number
  clockOffsetMs: number
  clockProbeRttUs: HistSummary
  frames: number
  frameIntervalUs: HistSummary
  drawUs: HistSummary
  meterHandlerUs: HistSummary
  waveHandlerUs: HistSummary
  positionHandlerUs: HistSummary
  /**
   * Cross-clock one-way estimates. Kept because a *bound* is still useful, but
   * with a millisecond-clamped client clock these are dominated by the offset
   * error; `echoRttUs` in the Rust half of the report is the number to quote.
   */
  meterLatencyUs: HistSummary
  waveLatencyUs: HistSummary
  counters: Record<string, number>
  /** Sampled every 30 s. For a 20 s matrix arm this is empty or near-empty;
   * for the 30-minute soak it is the answer. */
  timeline: TimelineSample[]
  worker?: unknown
  elapsedSecs: number
  /** Frames whose interval exceeded 20 ms / 33 ms - §37's "the UI must not
   * stutter" made countable. */
  jank20: number
  jank33: number
}

/**
 * Measures the real granularity of `performance.now()` by spinning until it
 * changes. Cheap, runs once per run, and turns a silent floor under every
 * client-side duration into a reported number.
 */
function clockResolutionMs(transitions = 64): number {
  let min = Infinity
  for (let n = 0; n < transitions; n++) {
    const a = performance.now()
    let b = a
    // Bounded spin: if the clock never moves we would otherwise hang.
    for (let i = 0; i < 5_000_000 && b === a; i++) b = performance.now()
    if (b > a) min = Math.min(min, b - a)
  }
  return min === Infinity ? -1 : min
}

/**
 * Estimates the offset between the Rust epoch and `performance.now()` by
 * round-tripping `now_us`. The sample with the smallest round trip is the least
 * contaminated, which is the standard trick and the only one available across
 * two clocks.
 */
async function clockOffset(samples = 64): Promise<{ offsetMs: number; rtt: HistSummary }> {
  const rtt = new Hist()
  let best = Infinity
  let offset = 0
  for (let i = 0; i < samples; i++) {
    const t0 = performance.now()
    const us = await invoke<number>('now_us')
    const t1 = performance.now()
    const r = t1 - t0
    rtt.record(r * 1000)
    if (r < best) {
      best = r
      offset = us / 1000 - (t0 + t1) / 2
    }
  }
  return { offsetMs: offset, rtt: rtt.summary() }
}

export async function run(opts: RunOptions): Promise<ClientReport> {
  const { cfg, renderMode } = opts
  const say = opts.say ?? ((): void => {})
  const tPhase = performance.now()
  const mark = (what: string): void => {
    say(`phase ${what}: ${((performance.now() - tPhase) / 1000).toFixed(1)}s`)
  }
  const resolutionMs = clockResolutionMs()
  mark('clockResolution')
  const { offsetMs, rtt } = await clockOffset()
  mark('clockOffset')
  // Echo at ~5 Hz whatever the meter rate, so the arms that send at 750 Hz are
  // not measured with 75 invokes a second of extra load the others do not have.
  const echoEvery = Math.max(1, Math.round((cfg.coalesce ? cfg.meterHz : cfg.captureRate / cfg.callbackFrames) / 5))

  const frameInterval = new Hist()
  const drawUs = new Hist()
  const meterHandler = new Hist()
  const waveHandler = new Hist()
  const posHandler = new Hist()
  const meterLatency = new Hist()
  const waveLatency = new Hist()
  const counters = new Counters()

  const meter = newMeter()
  const wave = newWave()
  const position = newPosition()

  const store = new WaveStore()
  let worker: Worker | null = null
  let workerReport: unknown = null

  const mg = opts.meterCanvas.getContext('2d', { alpha: false })
  if (!mg) throw new Error('no 2d context for the meter')
  const meterView = new MeterView(mg)
  void meterView

  let waveRenderer: { frame(s: WaveStore): void } | null = null
  if (renderMode === 'none') {
    // The control: receive everything, draw nothing. Separates the cost of the
    // transport from the cost of the canvas, which no single arm can do.
  } else if (renderMode === 'worker') {
    // `transferControlToOffscreen` can only happen once per canvas element, so
    // the caller hands us a fresh canvas for each run.
    const off = opts.waveCanvas.transferControlToOffscreen()
    worker = new Worker(new URL('./wave-worker.ts', import.meta.url), { type: 'module' })
    worker.postMessage({ kind: 'init', canvas: off, mode: 'incremental' }, [off])
  } else {
    const wg = opts.waveCanvas.getContext('2d', { alpha: false })
    if (!wg) throw new Error('no 2d context for the waveform')
    waveRenderer = renderMode === 'naive' ? new NaiveWave(wg) : new IncrementalWave(wg)
  }

  // --- receive side ---------------------------------------------------------

  const onMeter = (msg: unknown): void => {
    const t = performance.now()
    const prev = meter.seq
    decodeMeter(msg, cfg.encoding, meter)
    if (meter.seq !== 0 && meter.seq !== prev + 1) counters.bump('meterSeqGaps')
    meterLatency.record(Math.max(0, (t - (meter.tUs / 1000 + offsetMs)) * 1000))
    counters.bump('meterRecv')
    if (meter.seq % echoEvery === 0) {
      counters.bump('echoSent')
      void invoke('echo', { sentUs: meter.tUs })
    }
    meterHandler.record((performance.now() - t) * 1000)
  }

  const onWave = (msg: unknown): void => {
    const t = performance.now()
    const prev = wave.seq
    decodeWave(msg, cfg.encoding, wave)
    if (wave.seq !== 0 && wave.seq !== prev + 1) counters.bump('waveSeqGaps')
    waveLatency.record(Math.max(0, (t - (wave.tUs / 1000 + offsetMs)) * 1000))
    if (worker) {
      // Transfer a right-sized copy: the decode buffer is reused, so it cannot
      // be given away.
      const slice = wave.buckets.slice(0, wave.len)
      worker.postMessage({ kind: 'buckets', buckets: slice, len: wave.len }, [slice.buffer])
    } else {
      store.append(wave)
    }
    counters.bump('waveRecv')
    counters.bump('bucketsRecv', wave.len >> 2)
    waveHandler.record((performance.now() - t) * 1000)
  }

  const onPosition = (msg: unknown): void => {
    const t = performance.now()
    decodePosition(msg, cfg.encoding, position)
    counters.bump('positionRecv')
    posHandler.record((performance.now() - t) * 1000)
  }

  const unlisten: UnlistenFn[] = []
  // The channels are constructed either way, because `start_bench` takes three
  // of them; in event mode they simply never fire.
  const meterCh = new Channel<unknown>(cfg.transport === 'channel' ? onMeter : () => {})
  const waveCh = new Channel<unknown>(cfg.transport === 'channel' ? onWave : () => {})
  const posCh = new Channel<unknown>(cfg.transport === 'channel' ? onPosition : () => {})

  if (cfg.transport === 'event') {
    unlisten.push(await listen('meter-update', (e) => onMeter(e.payload)))
    unlisten.push(await listen('waveform-update', (e) => onWave(e.payload)))
    unlisten.push(await listen('recording-position', (e) => onPosition(e.payload)))
  }

  // --- draw side ------------------------------------------------------------

  let frames = 0
  let jank20 = 0
  let jank33 = 0
  let lastFrame = 0
  let stopped = false
  const t0 = performance.now()

  const tick = (): void => {
    if (stopped) return
    const t = performance.now()
    if (lastFrame) {
      const dt = t - lastFrame
      frameInterval.record(dt * 1000)
      if (dt > 20) jank20++
      if (dt > 33) jank33++
    }
    lastFrame = t
    if (renderMode !== 'none') {
      meterView.frame(meter.peakL, meter.peakR, meter.rmsL, meter.rmsR, meter.clip)
      waveRenderer?.frame(store)
    }
    opts.onFrame?.(meter.peakL, meter.peakR, meter.rmsL, meter.rmsR, meter.clip)
    const dus = (performance.now() - t) * 1000
    drawUs.record(dus)
    // With a 1 ms clock floor, "p99 draw" is not a precise figure but "how
    // many frames spent a whole millisecond or more drawing" is exact. These
    // are the countable version of the same question.
    if (dus >= 1000) counters.bump('drawGe1ms')
    if (dus >= 4000) counters.bump('drawGe4ms')
    if (dus >= 8000) counters.bump('drawGe8ms')
    frames++
    requestAnimationFrame(tick)
  }
  requestAnimationFrame(tick)

  const timeline: TimelineSample[] = []
  let ticks = 0
  const progress = window.setInterval(() => {
    const atSecs = (performance.now() - t0) / 1000
    opts.onProgress?.(atSecs)
    ticks++
    if (ticks % 30 !== 0) return
    void invoke<{ total_rss_kib: number; process_count: number }>('memory').then((m) => {
      timeline.push({
        atSecs,
        rssTotalKib: m.total_rss_kib,
        processCount: m.process_count,
        frames,
        jank20,
        jank33,
        meterRecv: counters.get('meterRecv'),
        drawMeanUs: drawUs.summary().meanUs,
      })
    })
  }, 1000)

  mark('renderersReady')
  await invoke('start_bench', { cfg, meter: meterCh, wave: waveCh, position: posCh })
  mark('benchStarted')

  await new Promise<void>((resolve) => {
    window.setTimeout(resolve, cfg.durationSecs * 1000 + 250)
  })

  stopped = true
  mark('runComplete')
  window.clearInterval(progress)
  await invoke('stop_bench')
  mark('stopBench')
  for (const u of unlisten) u()

  if (worker) {
    workerReport = await new Promise((resolve) => {
      const done = (e: MessageEvent): void => {
        worker?.removeEventListener('message', done)
        resolve(e.data)
      }
      worker?.addEventListener('message', done)
      worker?.postMessage({ kind: 'report' })
      window.setTimeout(() => resolve({ error: 'worker did not report' }), 2000)
    })
    worker.terminate()
  }

  const report: ClientReport = {
    renderMode,
    clockResolutionMs: resolutionMs,
    clockOffsetMs: offsetMs,
    clockProbeRttUs: rtt,
    frames,
    frameIntervalUs: frameInterval.summary(),
    drawUs: drawUs.summary(),
    meterHandlerUs: meterHandler.summary(),
    waveHandlerUs: waveHandler.summary(),
    positionHandlerUs: posHandler.summary(),
    meterLatencyUs: meterLatency.summary(),
    waveLatencyUs: waveLatency.summary(),
    counters: counters.toJSON(),
    timeline,
    worker: workerReport ?? undefined,
    elapsedSecs: (performance.now() - t0) / 1000,
    jank20,
    jank33,
  }

  mark('reportBuilt')
  await invoke('submit_client_stats', { stats: report })
  mark('submitted')
  return report
}

export const WAVE_WIDTH = WAVE_W
