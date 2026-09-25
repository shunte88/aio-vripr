/*
 *  stats.ts
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  Client-side histogram, deliberately the same log-bucket shape as the Rust
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

// Client-side histogram, deliberately the same log-bucket shape as the Rust
// `metrics::Hist`, so the two halves of a report are comparable without
// reconciling two different percentile definitions.
//
// Allocation-free after construction: this runs inside the rAF callback whose
// duration is the thing being measured, so it must not create garbage.

export interface HistSummary {
  count: number
  meanUs: number
  p50UsUpper: number
  p99UsUpper: number
  maxUs: number
}

const BUCKETS = 32

export class Hist {
  private readonly buckets = new Float64Array(BUCKETS)
  private count = 0
  private sum = 0
  private max = 0

  record(us: number): void {
    if (!(us >= 0)) return
    const i = us < 1 ? 0 : Math.min(BUCKETS - 1, 31 - Math.clz32(Math.floor(us)))
    this.buckets[i] += 1
    this.count += 1
    this.sum += us
    if (us > this.max) this.max = us
  }

  /**
   * Upper edge of the bucket the pth percentile falls in - deliberately not an
   * interpolated percentile, because with `performance.now()` clamped to
   * milliseconds in WebKit an interpolated figure would imply a precision the
   * clock does not have.
   */
  private pct(p: number): number {
    if (this.count === 0) return 0
    const target = this.count * p
    let seen = 0
    for (let i = 0; i < BUCKETS; i++) {
      seen += this.buckets[i]
      if (seen >= target) return 2 ** (i + 1)
    }
    return this.max
  }

  summary(): HistSummary {
    return {
      count: this.count,
      meanUs: this.count ? this.sum / this.count : 0,
      p50UsUpper: this.pct(0.5),
      p99UsUpper: this.pct(0.99),
      maxUs: this.max,
    }
  }

  reset(): void {
    this.buckets.fill(0)
    this.count = 0
    this.sum = 0
    this.max = 0
  }
}

/** Monotonic counters that need no distribution, only a total. */
export class Counters {
  private readonly m = new Map<string, number>()
  bump(k: string, by = 1): void {
    this.m.set(k, (this.m.get(k) ?? 0) + by)
  }
  get(k: string): number {
    return this.m.get(k) ?? 0
  }
  toJSON(): Record<string, number> {
    return Object.fromEntries(this.m)
  }
  reset(): void {
    this.m.clear()
  }
}
