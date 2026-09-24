// The bench UI. React drives the chrome only: the meter and waveform are
// imperative canvas work inside `bench.run`, because putting the hot path
// through the reconciler in every arm would hide the one number the
// `render-react-dom` arm exists to produce.

import { invoke } from '@tauri-apps/api/core'
import React, { useCallback, useEffect, useRef, useState } from 'react'
import { createRoot } from 'react-dom/client'

import { ARMS, armConfig, BASE, SOAK, type Arm } from './arms'
import { run, WAVE_WIDTH, type BenchConfig, type ClientReport, type RenderMode } from './bench'

const WAVE_H = 220

interface Row {
  name: string
  cfg: BenchConfig
  client: ClientReport
  reportPath: string
}

function fmtUs(us: number): string {
  return us >= 1000 ? `${(us / 1000).toFixed(2)} ms` : `${Math.round(us)} us`
}

function App(): React.JSX.Element {
  const meterRef = useRef<HTMLCanvasElement | null>(null)
  const waveRef = useRef<HTMLCanvasElement | null>(null)
  // `transferControlToOffscreen` is one-way, so the worker arm needs a canvas
  // element it has never touched. Bumping the key remounts one.
  const [canvasKey, setCanvasKey] = useState(0)

  const [duration, setDuration] = useState(20)
  const [busy, setBusy] = useState<string | null>(null)
  const [elapsed, setElapsed] = useState(0)
  const [rows, setRows] = useState<Row[]>([])
  const [meterState, setMeterState] = useState<[number, number]>([0, 0])
  const [error, setError] = useState<string | null>(null)

  const runArm = useCallback(
    async (arm: Arm, secs: number, say: (m: string) => void = () => {}): Promise<Row> => {
      const cfg = armConfig(arm, secs)
      const renderMode: RenderMode = arm.renderMode ?? 'incremental'
      setCanvasKey((k) => k + 1)
      // Let React commit the fresh canvas before the run grabs its context.
      await new Promise((r) => requestAnimationFrame(r))
      const meterCanvas = meterRef.current
      const waveCanvas = waveRef.current
      if (!meterCanvas || !waveCanvas) throw new Error('canvases not mounted')

      const client = await run({
        cfg,
        renderMode,
        meterCanvas,
        waveCanvas,
        onProgress: setElapsed,
        say,
        onFrame:
          renderMode === 'react-dom'
            ? (peakL, peakR) => setMeterState([peakL, peakR])
            : undefined,
      })
      const reportPath = await invoke<string>('write_report', { name: arm.name })
      return { name: arm.name, cfg, client, reportPath }
    },
    [],
  )

  const runOne = useCallback(
    async (arm: Arm) => {
      setError(null)
      setBusy(arm.name)
      try {
        const row = await runArm(arm, duration)
        setRows((r) => [...r, row])
      } catch (e) {
        setError(String(e))
      } finally {
        setBusy(null)
        setElapsed(0)
      }
    },
    [duration, runArm],
  )

  const runList = useCallback(
    async (arms: Arm[], secs: number, say: (m: string) => void = () => {}) => {
    setError(null)
    setRows([])
    for (const arm of arms) {
      setBusy(arm.name)
      say(`arm start: ${arm.name}`)
      try {
        const row = await runArm(arm, secs, say)
        say(`arm done: ${arm.name} -> ${row.reportPath}`)
        setRows((r) => [...r, row])
      } catch (e) {
        // Logged as well as shown: an unattended run that fails must say so on
        // stderr, or a matrix that aborts on its first arm looks exactly like
        // one that finished.
        say(`arm failed: ${arm.name}: ${String(e)}`)
        setError(`${arm.name}: ${String(e)}`)
        break
      }
      // A pause between arms so one arm's GC does not land in the next one.
      await new Promise((r) => window.setTimeout(r, 1500))
    }
    setBusy(null)
    setElapsed(0)
    },
    [runArm],
  )

  const runMatrix = useCallback(() => runList(ARMS, duration), [duration, runList])
  const runSoak = useCallback(() => runList(SOAK, duration), [duration, runList])

  // Unattended mode. The 10-arm matrix and the 30-minute soak are both too long
  // to click through, and a run driven by a human clicking is a run whose start
  // conditions are not reproducible.
  const started = useRef(false)
  useEffect(() => {
    if (started.current) return
    started.current = true
    void (async () => {
      const say = (m: string): void => void invoke('log', { msg: m })
      window.addEventListener('error', (e) => say(`uncaught: ${e.message} @ ${e.filename}:${e.lineno}`))
      window.addEventListener('unhandledrejection', (e) => say(`unhandled: ${String(e.reason)}`))
      say('frontend up')
      try {
        const auto = await invoke<{ what: string; secs: number; thenExit: boolean } | null>('autorun')
        say(`autorun: ${JSON.stringify(auto)}`)
        if (!auto) return
        const all = [...ARMS, ...SOAK]
        const arms =
          auto.what === 'matrix'
            ? ARMS
            : auto.what === 'soak'
              ? SOAK
              : all.filter((a) => a.name === auto.what)
        if (arms.length === 0) {
          setError(`no such arm: ${auto.what}`)
          if (auto.thenExit) await invoke('finish', { code: 2 })
          return
        }
        setDuration(auto.secs)
        await runList(arms, auto.secs, say)
        say('run list complete')
        if (auto.thenExit) await invoke('finish', { code: 0 })
      } catch (e) {
        say(`autorun failed: ${String(e)}`)
        await invoke('finish', { code: 3 })
      }
    })()
  }, [runList])

  return (
    <div className="app">
      <h1>VCW S3 — Tauri IPC throughput</h1>
      <p className="sub">
        base: {BASE.captureRate / 1000} kHz / {BASE.callbackFrames}-frame callbacks ={' '}
        {Math.round(BASE.captureRate / BASE.callbackFrames)} Hz worker rate; coalesced to{' '}
        {BASE.meterHz} Hz meter, {BASE.waveHz} Hz waveform, {BASE.positionHz} Hz position
      </p>

      <div className="controls">
        <label>
          seconds per arm{' '}
          <input
            type="number"
            min={5}
            max={3600}
            value={duration}
            disabled={busy !== null}
            onChange={(e) => setDuration(Number(e.target.value))}
          />
        </label>
        <button onClick={runMatrix} disabled={busy !== null}>
          run matrix ({ARMS.length} arms)
        </button>
        <button onClick={runSoak} disabled={busy !== null}>
          run soak
        </button>
        <button onClick={() => void invoke('memory').then((m) => console.log(m))} disabled={busy !== null}>
          log memory
        </button>
        {busy && (
          <span className="busy">
            running <b>{busy}</b> — {elapsed.toFixed(0)}/{duration}s
          </span>
        )}
      </div>

      {error && <p className="error">{error}</p>}

      <div className="stage">
        <canvas key={`m${canvasKey}`} ref={meterRef} width={180} height={WAVE_H} />
        <canvas key={`w${canvasKey}`} ref={waveRef} width={WAVE_WIDTH} height={WAVE_H} />
      </div>

      <div className="dommeter">
        {/* Only meaningful during the react-dom arm; present always so the DOM
            shape is identical across arms. */}
        <div className="bar" style={{ height: `${Math.min(1, meterState[0]) * 100}%` }} />
        <div className="bar" style={{ height: `${Math.min(1, meterState[1]) * 100}%` }} />
      </div>

      <table>
        <thead>
          <tr>
            <th>arm</th>
            <th>fps</th>
            <th>frame p99</th>
            <th>draw p99</th>
            <th>jank &gt;20/33ms</th>
            <th>meter recv</th>
            <th>handler p99</th>
            <th>latency p50</th>
            <th>report</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.name}>
              <td>{r.name}</td>
              <td>{(r.client.frames / r.client.elapsedSecs).toFixed(1)}</td>
              <td>{fmtUs(r.client.frameIntervalUs.p99UsUpper)}</td>
              <td>{fmtUs(r.client.drawUs.p99UsUpper)}</td>
              <td>
                {r.client.jank20}/{r.client.jank33}
              </td>
              <td>{r.client.counters.meterRecv ?? 0}</td>
              <td>{fmtUs(r.client.meterHandlerUs.p99UsUpper)}</td>
              <td>{fmtUs(r.client.meterLatencyUs.p50UsUpper)}</td>
              <td className="path">{r.reportPath}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <details>
        <summary>arms</summary>
        <ul className="arms">
          {ARMS.map((a) => (
            <li key={a.name}>
              <button onClick={() => void runOne(a)} disabled={busy !== null}>
                run
              </button>{' '}
              <b>{a.name}</b> — {a.question}
            </li>
          ))}
        </ul>
      </details>
    </div>
  )
}

const el = document.getElementById('root')
if (!el) throw new Error('no #root')
createRoot(el).render(<App />)
