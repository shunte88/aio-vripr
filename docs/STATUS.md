# VCW — project status

**As of:** 2026-09-23
**Phase:** 0 (de-risking spikes), gate G0 not yet passed
**Branch:** `main`, Phase 0 spikes committed (`f4a408c`)

This is the running snapshot: where Phase 0 actually stands, what is proven versus
assumed, what is waiting on a decision, and what is waiting on hardware. The plan of
record is [`PROJECT_PLAN.md`](../PROJECT_PLAN.md); the spec is
[`REQUIREMENTS.md`](../REQUIREMENTS.md).

---

## The rename — complete

`aio-vripr` → **VCW, The Vinyl Capture Workstation**. Fully landed: GitHub repo renamed,
`origin` on `github.com/shunte88/vcw`, working directory `/data2/vcw`, Phase 0 spikes
committed, and the naming convention adopted across the deliverables including the `.vcw`
project extension. The only surviving mention of the old name is the deliberate
provenance note in the plan.

Predecessor **VRipr** keeps its own name — VCW is its successor, not a rebrand — and
`/data2/vripr` remains the source of the ~30 % of ported functionality.

## Phase 0 spikes

| Spike | Question | Verdict |
|---|---|---|
| **S1** | Bit-perfect capture and playback through CPAL? | **Yes on Linux/x86_64**, on stock CPAL 0.18.2. Other platforms open. |
| **S2** | Can SQLite absorb sustained 24/192 and survive a kill? | **Yes on x86_64/SSD**, 90-minute soak passed. Other platforms open. |
| **S3** | Will Tauri IPC carry meter and waveform rates? | Not started. |
| **S4** | Does `chromaprint-next` fingerprint from a stream? | Not started. |
| **S5** | Is the Audacity project format readable? | **AUP3 yes, decisively.** AUP4 unmeasured. |

~3,640 lines of Rust across three spike crates, plus a 243-line Python format probe.
Write-ups in [`docs/spikes/`](spikes/).

### S1 — what it proved, and what upstream then fixed

Proven: CPAL's *audio path* is genuinely conversion-free. Re-measured 2026-09-23 on
**stock CPAL 0.18.2, no patches**: 2,883,584 frames at 24/192, zero drops, kernel
confirming the negotiated format; clean playback; 3/3 crash recovery.

The spike's durable output is the **kernel cross-check**. On 0.16 CPAL's ALSA device list
was the plug layer's fiction: a request for 48 kHz / 2 ch / I32 was reported as honoured
while the hardware ran 8 kHz mono S16 — a silent upsample no CPAL API surfaced. The check
against `/proc/asound/.../hw_params` caught it, and it is a mandatory WP-04 obligation:
*bit-perfection is never claimed without OS confirmation.*

**Both 0.16 defects are fixed in the released 0.18.2**, which also ships
`HostTrait::device_by_id` and enumerates `hw:` PCMs — the exact API S1 had recommended
upstreaming. The vendored fork is deleted. R2 went L → M on the 0.16 evidence and back to
**L** on 0.18, with the verifier keeping it there.

**New finding: recovery loss has a floor the config cannot cross.** Loss is
`commit granularity + driver buffer`, rounded to a block boundary — not
`block_ms × batch_blocks` alone. Ring capacity swept 100–1000 ms changed nothing; the
170 ms ALSA buffer sets the floor. 250 ms blocks sit *at* that floor, which refines D3 and
corrected a crash-test budget that had been failing correct runs.

### S2 — acceptance met on one platform

The 90-minute soak passed: 5400.196 s, real-time factor 0.99996, 1,036,800,512 frames,
**zero dropped**, 8.29 GB of audio into an 8.41 GB database. Commit p99 63.2 ms and
worst-ever commit 102.3 ms against a 250 ms budget; peak WAL 4.57 MiB; RSS flat at
8.1 MiB; 207,707 concurrent reader queries across 4,236,810 blocks with zero checksum
failures. The commit distribution is stationary over 90 minutes.

The load-bearing conclusion: **throughput is not the constraint and is not near being
one.** The real design variable is recovery granularity, so the budget goes to smaller,
more frequent commits rather than bigger batches — which inverts the usual instinct.
R1's sidecar-file fallback looks unnecessary; §12's single-file project survives.

S2 measured worst-case loss as exactly `block_ms × batch_blocks`, which is true *of this
harness* — it has no audio device. S1 Finding 4 above shows the rest of the picture on
real hardware, and puts a floor under it that smaller commits cannot cross.

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

## Decisions resolved 2026-09-23

1. **The project extension is `.vcw`** (was `.vripr`). Applied to the spike CLIs,
   `.gitignore` patterns, D1 in the plan, the local-dev config path (`~/.config/vcw/`,
   `vcw.toml`) and every doc.
2. **CPAL: track the released crate.** `shunte88/cpal` is the fork of record for any
   defect we find that is not already fixed upstream — fixes go there first, then
   upstream. It is currently **unused**, because 0.18.2 fixed both defects we had found,
   which is the better outcome. Standing policy: prefer the released crate; fork only
   with a reproduction we cannot get upstream in time; never carry a `[patch.crates-io]`
   path copy again — that is what we had, and it quietly hid how stale our pin was.

## Still open

- **Corpus fixture strategy.** The 25 real rips are the import regression set; they need
  a shrinker (WP-20) so fixtures are committable.
- **Crate naming** — `vcw-audio`, `vcw-signal`, … — settled at WP-01.

## Next up

`WP-01` proper: the full crate layout per §6, CI matrix, `cargo-deny`, ported
`THIRD-PARTY-NOTICES.md` and `LICENSE-LGPL-2.1`, and ADR-0001 (D1) / ADR-0002 (D2). No
vendored-cpal notice is needed any more — cpal is a plain Apache-2.0 dependency again.

## Housekeeping

- Phase 0 spikes are committed at `f4a408c`. The CPAL 0.18 upgrade, the `.vcw` rename and
  this revision are working-tree changes on top of it.
- `.bench/` holds **~8 GB** of scratch databases — the 8.4 GB soak artefact plus older
  `.vripr`-suffixed files from before the rename. All gitignored and safe to delete; the
  S1 and S2 numbers are recorded here and in the spike write-ups.
- Licensing today: MIT core, cpal Apache-2.0 as an ordinary dependency.
  `chromaprint-next` adds an LGPL-2.1-or-later relink obligation at Phase 2.
- **Stay current on CPAL.** Two blocking defects and the device-id API all landed within
  two minor releases; pinning 0.16 had already cost us a fork.
