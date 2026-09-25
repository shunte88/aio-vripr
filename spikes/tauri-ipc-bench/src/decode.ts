/*
 *  decode.ts
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  Receiving side of the three encodings in `src-tauri/src/payload.rs`.
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

// Receiving side of the three encodings in `src-tauri/src/payload.rs`.
//
// The decode cost belongs in the measurement: `raw` only wins if the bytes it
// saves on the wire are not handed straight back as DataView work on the main
// thread. So every decoder writes into one preallocated object per stream and
// returns it, rather than allocating.

export type Encoding = 'serde' | 'manual' | 'raw'
export type Transport = 'channel' | 'event'

export interface Meter {
  seq: number
  tUs: number
  peakL: number
  peakR: number
  rmsL: number
  rmsR: number
  clip: number
}

export interface Wave {
  seq: number
  tUs: number
  level: number
  startBucket: number
  /** Interleaved [minL, maxL, minR, maxR] per bucket. */
  buckets: Int16Array
  /** How many entries of `buckets` are valid this message. */
  len: number
}

export interface Position {
  seq: number
  tUs: number
  frame: number
  dropped: number
}

export const newMeter = (): Meter => ({
  seq: 0,
  tUs: 0,
  peakL: 0,
  peakR: 0,
  rmsL: 0,
  rmsR: 0,
  clip: 0,
})

export const newWave = (cap = 4096): Wave => ({
  seq: 0,
  tUs: 0,
  level: 0,
  startBucket: 0,
  buckets: new Int16Array(cap),
  len: 0,
})

export const newPosition = (): Position => ({ seq: 0, tUs: 0, frame: 0, dropped: 0 })

// A shared DataView, re-pointed per message. Constructing one per frame is a
// measurable allocation at 750 Hz.
let view: DataView | null = null
let viewBuf: ArrayBuffer | null = null

function viewOf(buf: ArrayBuffer): DataView {
  if (viewBuf !== buf || view === null) {
    view = new DataView(buf)
    viewBuf = buf
  }
  return view
}

/** `u64` fields exceed Number.MAX_SAFE_INTEGER only past ~285 years of us. */
function readU64(v: DataView, off: number): number {
  return Number(v.getBigUint64(off, true))
}

export function decodeMeter(msg: unknown, enc: Encoding, out: Meter): Meter {
  if (enc === 'raw') {
    const v = viewOf(msg as ArrayBuffer)
    // kind tag at 0 is not checked on the hot path: the channel is per-stream,
    // so a wrong tag is a bug in the bench, not a runtime condition.
    out.seq = v.getUint32(1, true)
    out.tUs = readU64(v, 5)
    out.peakL = v.getFloat32(13, true)
    out.peakR = v.getFloat32(17, true)
    out.rmsL = v.getFloat32(21, true)
    out.rmsR = v.getFloat32(25, true)
    out.clip = v.getUint8(29)
    return out
  }
  if (enc === 'manual') {
    const m = msg as { s: number; t: number; p: [number, number]; r: [number, number]; c: number }
    out.seq = m.s
    out.tUs = m.t
    out.peakL = m.p[0]
    out.peakR = m.p[1]
    out.rmsL = m.r[0]
    out.rmsR = m.r[1]
    out.clip = m.c
    return out
  }
  const m = msg as { seq: number; t_us: number; peak: [number, number]; rms: [number, number]; clip: number }
  out.seq = m.seq
  out.tUs = m.t_us
  out.peakL = m.peak[0]
  out.peakR = m.peak[1]
  out.rmsL = m.rms[0]
  out.rmsR = m.rms[1]
  out.clip = m.clip
  return out
}

export function decodeWave(msg: unknown, enc: Encoding, out: Wave): Wave {
  if (enc === 'raw') {
    const v = viewOf(msg as ArrayBuffer)
    out.seq = v.getUint32(1, true)
    out.tUs = readU64(v, 5)
    out.level = v.getUint8(13)
    out.startBucket = readU64(v, 14)
    const n = v.getUint32(22, true)
    if (n > out.buckets.length) out.buckets = new Int16Array(n * 2)
    for (let i = 0; i < n; i++) out.buckets[i] = v.getInt16(26 + i * 2, true)
    out.len = n
    return out
  }
  if (enc === 'manual') {
    const m = msg as { s: number; t: number; l: number; b: number; v: number[] }
    out.seq = m.s
    out.tUs = m.t
    out.level = m.l
    out.startBucket = m.b
    if (m.v.length > out.buckets.length) out.buckets = new Int16Array(m.v.length * 2)
    for (let i = 0; i < m.v.length; i++) out.buckets[i] = m.v[i]
    out.len = m.v.length
    return out
  }
  const m = msg as { seq: number; t_us: number; level: number; start_bucket: number; buckets: number[] }
  out.seq = m.seq
  out.tUs = m.t_us
  out.level = m.level
  out.startBucket = m.start_bucket
  if (m.buckets.length > out.buckets.length) out.buckets = new Int16Array(m.buckets.length * 2)
  for (let i = 0; i < m.buckets.length; i++) out.buckets[i] = m.buckets[i]
  out.len = m.buckets.length
  return out
}

export function decodePosition(msg: unknown, enc: Encoding, out: Position): Position {
  if (enc === 'raw') {
    const v = viewOf(msg as ArrayBuffer)
    out.seq = v.getUint32(1, true)
    out.tUs = readU64(v, 5)
    out.frame = readU64(v, 13)
    out.dropped = v.getUint32(21, true)
    return out
  }
  if (enc === 'manual') {
    const m = msg as { s: number; t: number; f: number; d: number }
    out.seq = m.s
    out.tUs = m.t
    out.frame = m.f
    out.dropped = m.d
    return out
  }
  const m = msg as { seq: number; t_us: number; frame: number; dropped: number }
  out.seq = m.seq
  out.tUs = m.t_us
  out.frame = m.frame
  out.dropped = m.dropped
  return out
}

/**
 * On-wire size as the webview sees it. For `raw` this is the real byte count;
 * for the JSON arms tauri has already parsed it into a JS value by the time we
 * get it, so the Rust-side byte count is the authority and this only
 * cross-checks it.
 */
export function wireSize(msg: unknown, enc: Encoding): number {
  if (enc === 'raw') return (msg as ArrayBuffer).byteLength
  return 0
}
