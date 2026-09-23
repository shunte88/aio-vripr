# VCW — project status

**As of:** 2026-09-22
**Phase:** 0 (de-risking spikes), gate G0 not yet passed
**Branch:** `main`, nothing committed yet

This is the running snapshot: where Phase 0 actually stands, what is proven versus
assumed, what is waiting on a decision, and what is waiting on hardware. The plan of
record is [`PROJECT_PLAN.md`](../PROJECT_PLAN.md); the spec is
[`REQUIREMENTS.md`](../REQUIREMENTS.md).

---

## The rename

`aio-vripr` → **VCW, The Vinyl Capture Workstation**. Done inside the deliverables:
`README.md`, `PROJECT_PLAN.md`, `Cargo.toml` (`description`, and `repository` pointing
forward at `github.com/shunte88/vcw`). The only surviving mention of the old name is the
deliberate provenance note in the plan.

**Deliberately deferred** — the user's call, so the build wasn't blocked on it:

- GitHub repo still named `aio-vripr`; `origin` still points there.
- Working directory still `/data2/aio-vripr`.
- The keyed memory directory `~/.claude/projects/-data2-aio-vripr/` moves with it.

The `Cargo.toml` `repository` URL is a **forward reference to a repo that does not exist
yet** — it will 404 until the GitHub rename happens.

## Phase 0 spikes

| Spike | Question | Verdict |
|---|---|---|
| **S1** | Bit-perfect capture and playback through CPAL? | **Yes on Linux/x86_64, with caveats.** Other platforms open. |
| **S2** | Can SQLite absorb sustained 24/192 and survive a kill? | **Yes on x86_64/SSD**, 90-minute soak passed. Other platforms open. |
| **S3** | Will Tauri IPC carry meter and waveform rates? | Not started. |
| **S4** | Does `chromaprint-next` fingerprint from a stream? | Not started. |
| **S5** | Is the Audacity project format readable? | **AUP3 yes, decisively.** AUP4 unmeasured. |

~3,520 lines of Rust across three spike crates, plus a 243-line Python format probe.
Write-ups in [`docs/spikes/`](spikes/).

### S1 — what it actually proved, and what it disproved

Proven: CPAL's *audio path* is genuinely conversion-free. A verified 24/192 stereo
capture ran 960,152 frames with zero drops, the kernel confirming the negotiated format;
playback was clean; crash recovery was exact to the block, 3 for 3.

Disproved, and this matters: **CPAL's ALSA device list is the plug layer's fiction.** A
request for 48 kHz / 2 ch / I32 was reported as honoured while the hardware was actually
running 8 kHz mono S16 — a silent upsample that no CPAL API surfaces. The cross-check
against `/proc/asound/.../hw_params` is what caught it, and that check is now a mandatory
WP-04 obligation: *bit-perfection is never claimed without kernel confirmation.* Risk R2
was re-scored L → **M** on this evidence.

Also found: a CPAL 0.16.0 bug that made ALSA capture deliver **zero frames** on this
hardware (the timestamp-capability probe runs before `start()`, so every callback then
fails). Root-caused and fixed in 32 lines by moving the probe after `start()`; carried as
a vendored `[patch.crates-io]` copy under `spikes/vendor/cpal`. 0 frames → 960,152.

### S2 — acceptance met on one platform

The 90-minute soak passed: 5400.196 s, real-time factor 0.99996, 1,036,800,512 frames,
**zero dropped**, 8.29 GB of audio into an 8.41 GB database. Commit p99 63.2 ms and
worst-ever commit 102.3 ms against a 250 ms budget; peak WAL 4.57 MiB; RSS flat at
8.1 MiB; 207,707 concurrent reader queries across 4,236,810 blocks with zero checksum
failures. The commit distribution is stationary over 90 minutes.

The load-bearing conclusion: **throughput is not the constraint and is not near being
one.** The real design variable is recovery granularity, so the budget goes to smaller,
more frequent commits rather than bigger batches — which inverts the usual instinct.
Worst-case loss on a power cut is exactly `block_ms × batch_blocks`, demonstrated to the
block. R1's sidecar-file fallback looks unnecessary; §12's single-file project survives.

One honest gap: the soak ran the harness defaults (`synchronous=NORMAL`, interleaved),
not the D3-firmed `FULL` + per-channel. The short matrix says the firmed config should be
no worse, but *should be* is not *measured* — **D3 should not close until the firmed
config is soaked.**

### S5 — the format is not a mystery any more

The AUP3 `ProjectSerializer` document format is decoded and verified against **all 25**
real vinyl rips in the corpus: full byte consumption, zero dangling block references. The
acceptance test was chosen to be unforgiving — a wrong tag-length grammar desynchronises
within a few records, so 25/25 clean parses is evidence rather than optimism. Clean-room
throughout: Audacity is GPL, this codebase is MIT.

Two consequences worth carrying forward:

- Audacity has **no 32-bit integer sample format**, which is the concrete justification
  for D1's superset schema rather than an aesthetic preference.
- Audacity stores **mono 1 MiB blocks**, so the per-channel layout S2 measured and AUP4
  compatibility now *agree*. The D4 tension dissolves.

And a finding about the existing library rather than the code: **24 of the 25 rips are
stored float32**, so the current Audacity workflow has never been bit-perfect. Those
files are good masters, but they are not captures. That is a decision for the user, not a
defect to fix.

## G0 exit criteria still outstanding

- **Per-platform capture-mode matrix.** Windows/WASAPI exclusive, Android/AAudio, macOS
  (no hardware available — see the test-hardware gap).
- **S2 on Tier 1 hardware.** Pi 5 (aarch64, SD *and* NVMe — the honest worst case) and
  the Windows rig. Expect materially worse tails on the Pi.
- **S2 abuse matrix.** Disk-full, induced fsync stalls, `VACUUM`/compaction, live-project
  copy, page-size sweep, WAL2, and the full `sweep` matrix to completion.
- **Real converters.** Everything so far is onboard audio. The HiFiBerry DAC+ADC Pro and
  the Tascam DA-3000 are untested, as is `snd-aloop` bit-exactness (needs root).
- **S3** Tauri IPC throughput (2 sessions) and **S4** `chromaprint-next` streaming
  (1 session) — both unstarted.
- **AUP4.** Blocked on obtaining Audacity 4.x. Also unexercised: a project with a
  non-empty `envelope`, and one with a populated `autosave`.

## Open decisions — waiting on the user

1. **Does the native project extension change `.vripr` → `.vcw`?** 18 references; the
   cost is near zero now and grows steadily from here.
2. **Where does the CPAL fork live?** The vendored `[patch.crates-io]` copy is the wrong
   long-term shape. The replacement is a git dependency on a fork we own, plus an
   upstream report — but the fork's location and owner is a call to make, not assume.
3. **Corpus fixture strategy.** The 25 real rips are the import regression set; they need
   a shrinker (WP-20) so fixtures are committable.

## Next up

`WP-01` proper: the full crate layout per §6, CI matrix, `cargo-deny`, ported
`THIRD-PARTY-NOTICES.md` and `LICENSE-LGPL-2.1` (plus a new cpal/Apache-2.0 entry for the
vendored copy), and ADR-0001 (D1) / ADR-0002 (D2). Crate naming — `vcw-audio`,
`vcw-signal`, … — gets settled there.

## Housekeeping

- **Nothing is committed.** Modified: `.gitignore`, `PROJECT_PLAN.md`, `README.md`.
  Untracked: `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `docs/`, `spikes/`.
- `.bench/` holds **7.9 GB** of scratch databases, mostly the 8.4 GB soak artefact. It is
  gitignored and safe to delete once the S2 numbers above are considered recorded.
- Licensing today: MIT core, cpal Apache-2.0. `chromaprint-next` adds an
  LGPL-2.1-or-later relink obligation at Phase 2.
