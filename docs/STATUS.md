# VCW - project status

**As of:** 2026-09-25
**Phase:** 1 is underway - WP-01 through WP-06 are built, all on Linux x86_64 only.
All five Phase 0 spikes returned verdicts on their primary platform; gate G0 remains
open on hardware coverage, WP-05's soak settled D3's firmed-config run, and **WP-06
closes milestone M1, *it records*.**
**Branch:** `main` at `358c44a` (WP-05), plus WP-06 in the working tree.

This is the running snapshot: where Phase 0 actually stands, what is proven versus
assumed, what is waiting on a decision, and what is waiting on hardware. The plan of
record is [`PROJECT_PLAN.md`](../PROJECT_PLAN.md); the spec is
[`REQUIREMENTS.md`](../REQUIREMENTS.md).

---

## The rename - complete

`aio-vripr` → **VCW, The Vinyl Capture Workstation**. Fully landed: GitHub repo renamed,
`origin` on `github.com/shunte88/vcw`, working directory `/data2/vcw`, Phase 0 spikes
committed, and the naming convention adopted across the deliverables including the `.vcw`
project extension. The only surviving mention of the old name is the deliberate
provenance note in the plan.

Predecessor **VRipr** keeps its own name - VCW is its successor, not a rebrand - and
`/data2/vripr` remains the source of the ~30 % of ported functionality.

## Phase 0 spikes

| Spike | Question | Verdict |
|---|---|---|
| **S1** | Bit-perfect capture and playback through CPAL? | **Yes on Linux/x86_64**, on stock CPAL 0.18.2. Other platforms open. |
| **S2** | Can SQLite absorb sustained 24/192 and survive a kill? | **Yes on x86_64/SSD**, 90-minute soak passed. Other platforms open. |
| **S3** | Will Tauri IPC carry meter and waveform rates? | **Yes, with 12.5× headroom** on Linux/WebKitGTK, incl. a 30-min soak with zero loss. The constraint is main-thread *rendering* (29% naive vs 0.5% in a worker), not IPC. One open item: webview RSS +1.46 MiB/min (R8). Other platforms open. |
| **S4** | Does `chromaprint-next` fingerprint from a stream? | **Yes, bit-identically.** Linux/x86_64. Other platforms open. |
| **S5** | Is the Audacity project format readable? | **Yes, decisively - AUP3 and AUP4 both.** |

~6,200 lines of Rust across five spike crates, plus ~1,240 lines of TypeScript
(the S3 bench frontend) and ~570 lines of Python analysis and format probing.
Write-ups in [`docs/spikes/`](spikes/).

### S1 - what it proved, and what upstream then fixed

Proven: CPAL's *audio path* is genuinely conversion-free. Re-measured 2026-09-23 on
**stock CPAL 0.18.2, no patches**: 2,883,584 frames at 24/192, zero drops, kernel
confirming the negotiated format; clean playback; 3/3 crash recovery.

The spike's durable output is the **kernel cross-check**. On 0.16 CPAL's ALSA device list
was the plug layer's fiction: a request for 48 kHz / 2 ch / I32 was reported as honoured
while the hardware ran 8 kHz mono S16 - a silent upsample no CPAL API surfaced. The check
against `/proc/asound/.../hw_params` caught it, and it is a mandatory WP-04 obligation:
*bit-perfection is never claimed without OS confirmation.*

**Both 0.16 defects are fixed in the released 0.18.2**, which also ships
`HostTrait::device_by_id` and enumerates `hw:` PCMs - the exact API S1 had recommended
upstreaming. The vendored fork is deleted. R2 went L → M on the 0.16 evidence and back to
**L** on 0.18, with the verifier keeping it there.

**New finding: recovery loss has a floor the config cannot cross.** Loss is
`commit granularity + driver buffer`, rounded to a block boundary - not
`block_ms × batch_blocks` alone. Ring capacity swept 100–1000 ms changed nothing; the
170 ms ALSA buffer sets the floor. 250 ms blocks sit *at* that floor, which refines D3 and
corrected a crash-test budget that had been failing correct runs.

### S2 - acceptance met on one platform

The 90-minute soak passed: 5400.196 s, real-time factor 0.99996, 1,036,800,512 frames,
**zero dropped**, 8.29 GB of audio into an 8.41 GB database. Commit p99 63.2 ms and
worst-ever commit 102.3 ms against a 250 ms budget; peak WAL 4.57 MiB; RSS flat at
8.1 MiB; 207,707 concurrent reader queries across 4,236,810 blocks with zero checksum
failures. The commit distribution is stationary over 90 minutes.

The load-bearing conclusion: **throughput is not the constraint and is not near being
one.** The real design variable is recovery granularity, so the budget goes to smaller,
more frequent commits rather than bigger batches - which inverts the usual instinct.
R1's sidecar-file fallback looks unnecessary; §12's single-file project survives.

S2 measured worst-case loss as exactly `block_ms × batch_blocks`, which is true *of this
harness* - it has no audio device. S1 Finding 4 above shows the rest of the picture on
real hardware, and puts a floor under it that smaller commits cannot cross.

One honest gap, **closed 2026-09-25 by WP-05**: this soak ran the harness defaults
(`synchronous=NORMAL`, interleaved), not the D3-firmed `FULL` + per-channel. The short
matrix said the firmed config should be no worse, but *should be* is not *measured*.
WP-05's exit soak is that run, with the product code rather than the harness - see the
WP-05 section below, which also records the one thing the firmed config changed that
nobody had predicted: the WAL ceiling has to be stated in bytes, not pages.

### S3 - the experiment was aimed at the wrong thing

Thirteen arms × 60 s on the real compositor. The boundary itself is a non-issue: **zero
send errors, `recv/sent` = 1.000 and zero sequence gaps in every arm**, including two
that pushed 45,000 messages at the full 750 Hz worker rate (192 kHz, 256-frame
callbacks - the worst case §35 has to survive). The producer thread costs 0.14–0.19% of
a core coalesced and 0.75% uncoalesced, with send p99 ≤64 µs.

D6 asked three questions and got "doesn't matter", "backwards" and "backwards":

- **Channels vs the event bus** is indistinguishable at §35 payload sizes - 1.7–1.8%
  main-thread occupancy and 10.0–10.7 ms round trip for both, with the gap counts
  ordered *against* D6's prediction. Reading `tauri` 2.11.6 first explained why: under
  the 8192-byte threshold both transports are the same `webview.eval()` call.
- **Binary payloads are a pessimisation.** Under `MAX_RAW_DIRECT_EXECUTE_THRESHOLD`
  (1024 B) Tauri renders an `InvokeResponseBody::Raw` as a *decimal JSON array*. A 30 B
  meter frame becomes 116 B of evaluated JavaScript - 3.9× inflation, and 42% *more*
  source through the parser than the compact JSON it was supposed to beat.
- **Coalescing to 60 Hz is worse than not coalescing.** At 750 Hz occupancy was
  unchanged (1.6% vs 1.7%), long frames were *fewer* (71 vs 140), and delivery latency
  fell from 10.5 ms to **2.8 ms**. Coalescing sends bought nothing and cost ~7 ms.

**The load-bearing finding is one D6 never mentioned.** A full-canvas main-thread
redraw of a 1400×220 waveform at 60 Hz costs **29.0% of the main thread**, with 8.9% of
frames over 20 ms and nearly every frame over 4 ms. The same waveform in an
`OffscreenCanvas` worker costs **0.5%**, and was the only configuration measured that
never exceeded its frame budget - 3988 frames in a minute, maximum interval 16.0 ms,
where every main-thread arm reached 18–25 ms. So the engineering belongs in the
renderer, and the IPC layer can be chosen on ergonomics and never revisited.

The spike's other durable output is methodological, and it is why the numbers above are
stated the way they are. `performance.now()` on WebKitGTK is clamped to exactly 1 ms,
which made the first draft report every client-side latency as zero; the fix was to time
a round trip entirely on the Rust clock. The clamp truncates *timestamps*, not
durations, so the **mean** of clamped samples is unbiased while every individual sample
is useless - hence occupancy from means, corroborated by exact counts of frames over
1/4/8 ms, and no client-side percentile quoted as if it were precise. And
`requestAnimationFrame` is **not** vsync-paced here: 56–97 callbacks/s on a fixed 60 Hz
output, so fps is not a quality measure and **main-thread occupancy is the acceptance
metric instead**.

The 30-minute soak on the recommended configuration (channel + `manual` + worker) held
everything: 180,000 messages, **zero send errors, zero loss, zero sequence gaps**; the
worker rendered 119,133 frames with a **maximum interval of 18.0 ms**; main-thread draw
cost *fell* over the run (JIT warm-up) and whole-app CPU was flat at ~86–89% of one
core. Three frames out of 160,856 exceeded 33 ms.

**One thing did not hold, and it is now R8's first hard evidence.** Total RSS grew
491 → 538 MiB - a steady **+1.46 MiB/min with no plateau**, ~88 MiB/hour extrapolated.
That settles what the soak was run to settle (the matrix's 476 → 509 MiB drift was
time-based, not per-arm) and replaces it with a sharper question. It is orthogonal to
D6, and it is *not* diagnosed: this bench's UI allocates almost nothing per frame by
design, which makes application code the least likely cause and WebKit or tauri
per-message bookkeeping the most likely. It must be isolated before G2, because VCW is
meant to stay open across a multi-hour session.

Also worth recording because it nearly escaped: **every figure the bench reports is time
inside a callback it owns**, so it cannot see its own cost. Sampled from `/proc`, the
recommended configuration runs at ~89% of one core - 68% in `WebKitWebProcess`, 18% on
the tauri process's GTK main thread dispatching evals. The "0.5% occupancy" headline is
a main-thread *blocking* figure, not a CPU budget, and the Pi 5 conclusion will have to
rest on the latter. `cpu-sample.sh` and `cpu-matrix.sh` exist for that.

Honest gaps: one run per arm, so only the order-of-magnitude findings carry weight; this
rig mirrors a 59.98 Hz and a 29.96 Hz output, so absolute jank counts are contaminated
and only differences from the controls mean anything; the coalescing-raises-latency
effect is consistent across five arms but was not isolated by a controlled experiment;
and the bench UI is two canvases and a table, so these figures are a floor for a real
editor window.

### S4 - the acceptance was the easy half

Streamed and offline fingerprints are **bit-identical for every chunk shape tried**,
including one frame per `feed()` call, the ALSA period and buffer sizes, S2's 250 ms
block, a prime size and ragged ring drains - at 48 kHz and 192 kHz. Cost: **0.79 MiB and
0.6% of one core per stream**, with `feed()` taking 0.3% of its 250 ms block budget at
p99. A whole 22-minute 192 kHz side streams through in 6.5 MiB. Eight concurrent region
fingerprinters agree bit-for-bit and cost 2 ms of a 250 ms budget between them, so §46's
isolation requirement is satisfied several orders of magnitude over.

The more useful finding is one the plan did not ask for. **Region-boundary error is
bounded at ~0.064 BER**, reached at half a sub-fingerprint step (62 ms); a whole-step
error is a pure shift the matcher absorbs entirely. Unrelated audio scores 0.47–0.49 and
an MP3 320k round-trip costs 0.0009, so the worst boundary error a detector can make
still leaves a comfortable match. Consequence for Phase 2: **no re-fingerprint pass after
boundary refinement**, and the detector's precision requirement belongs to *editing*, not
identification. Capture rate (192k vs 44.1k), gain (−20 dB to +3 dB) and i16 narrowing
(truncate vs round) are all measurably **free** - fingerprint straight off the capture
stream at whatever rate the user chose.

Rather than trust the crate's "bit-identical to C" claim, it was checked against `fpcalc`
on byte-identical input. The pipeline after the resampler is exact on **9 of 9**
recordings. A resampler difference does exist - the packaged `fpcalc` links
`libswresample`/`libsoxr`, not chromaprint's bundled `av_resample` which the crate ports -
worth 2 to 6 single-bit flips out of 30,336, i.e. **five times smaller than an MP3 320k
round-trip**. Characterised, bounded, immaterial, and deliberately not root-caused.

One trap recorded for WP-11: `feed()` only `debug_assert!`s frame alignment, so a release
build handed a partial frame silently transposes the channel interleave and returns a
plausible wrong answer. The worker must guarantee whole frames at the type level.

### S5 - the format is not a mystery any more, in either version

The `ProjectSerializer` document format is decoded and verified against **all 30**
real vinyl projects in the corpus - 25 AUP3 and 5 AUP4: full byte consumption, zero
dangling block references. The acceptance test was chosen to be unforgiving - a wrong
tag-length grammar desynchronises within a few records, so 30/30 clean parses is evidence
rather than optimism. Clean-room throughout: Audacity is GPL, this codebase is MIT.

Consequences worth carrying forward:

- Audacity has **no 32-bit integer sample format**, which is the concrete justification
  for D1's superset schema rather than an aesthetic preference.
- Audacity stores **mono 1 MiB blocks**, so the per-channel layout S2 measured and AUP4
  compatibility now *agree*. The D4 tension dissolves.
- **AUP3 to AUP4 conversion is byte-identical on the audio layer.** Five albums were
  converted by Audacity 4.0.0 on 2026-09-24, giving matched pairs across both rates,
  both sample formats, both page sizes and 451 MB to 4.7 GB. Hashing all `sampleblocks`
  samples, and separately the formats and both summary pyramids, matches exactly in
  every pair: nothing is resampled, reformatted or repacked, int24 and a 4096-byte page
  size both survive, and sparse `blockid` ranges are preserved rather than renumbered.
  The file grows by a handful of pages regardless of size. **R12 was aimed at the
  document blob, and the document is exactly where all the change landed - the part we
  depend on did not move**, so R12 drops to L/L.
- **An exhaustive attribute diff narrows the claim usefully.** Comparing every attribute
  by element path and sibling index (1,647 to 8,941 shared per pair), only 8 to 22
  differ: version stamps, the metadata reordering, an assigned track `colorindex`,
  editor selection state, invented unity envelope points, and a 2.7e-15 s re-round of
  `waveclip/@trimLeft` in two clips. So the correct statement is **blocks
  byte-identical, audio-bearing f64 attributes preserved to within a ULP** - fixture
  tests must compare timings with a tolerance, not `==`. That 2.7e-15 s is 5.1e-10 of a
  sample, so it cannot move a clip onto a different sample.
- The AUP4 delta is one table (`project_history`, one full document per save), one new
  record (tag `0x10`, a length-prefixed blob used only for a PNG screenshot that
  Audacity renders as the preview tile in its recent-projects list), and ~25 new
  dictionary names of which all but three are view or spectrogram state. Metadata did
  not move: it has always been `tags`/`tag` elements in the document, and it is
  unchanged in content - but **reordered**, so fixture comparisons must be set-based.
- **Two findings that are worth real money to WP-20.** AUP4 adds `waveblock/@length`,
  the block's sample count, and it matched `sampleblocks` in all 5,664 cases - a free
  integrity check on the document before reading any audio. And **sample blocks are
  shared between clips**: the clip-split project has 532 `waveblock` references to 456
  distinct blocks, one referenced three times. Reference counting is mandatory, and
  anything that frees a block when one clip stops referencing it corrupts another clip.
  Only a clip-split project reveals that; the four single-clip albums are all 1:1.

And a finding about the existing library rather than the code: **24 of the 25 rips are
stored float32**, so the current Audacity workflow has never been bit-perfect. Those
files are good masters, but they are not captures. That is a decision for the user, not a
defect to fix.

#### The correction that matters most

The 2026-09-22 write-up had the sample rate backwards, and the wrong rule had already
reached WP-20's acceptance criteria. It said to trust `project/@rate` (which reads
`192000.0`) over `wavetrack/@rate` (`48000.0`), on the assumption that these were 192 kHz
captures. Three independent checks say the opposite: label extents are recorded in
seconds and fit `numsamples/48000` to within six seconds while overrunning
`numsamples/192000` fourfold; the same albums exist as WAV in `/data2/source_rips` with
matching exact frame counts and a 48000 header; and `project/@rate` is `192000.0` in all
30 files regardless of content, which is not what a measurement looks like.

**`wavetrack/@rate` is authoritative.** The importer rule is now the reverse of what was
written, and the cost of the error would have been playing 22 of 25 rips at 4x speed,
silently. `probe.py` reports `project_rate` and `track_rates` as separate fields with a
`rate_disagrees` flag so a caller cannot collapse them again.

## G0 exit criteria still outstanding

- **Per-platform capture-mode matrix.** Windows/WASAPI exclusive, Android/AAudio, macOS
  (no hardware available - see the test-hardware gap).
- **S2 on Tier 1 hardware.** Pi 5 (aarch64, SD *and* NVMe - the honest worst case) and
  the Windows rig. Expect materially worse tails on the Pi.
- **S2 abuse matrix.** Disk-full, induced fsync stalls, `VACUUM`/compaction, live-project
  copy, page-size sweep, WAL2, and the full `sweep` matrix to completion.
- **Real converters.** Everything so far is onboard audio. The HiFiBerry DAC+ADC Pro and
  the Tascam DA-3000 are untested, as is `snd-aloop` bit-exactness (needs root).
- **S3's memory growth (R8).** +1.46 MiB/min with no plateau over 30 minutes, measured
  and undiagnosed. Two cheap experiments split it: re-soak with the producer stopped
  (WebKit baseline vs per-message cost), then with `echo` disabled (tauri's
  per-invocation bookkeeping). Needed before G2, not before G0.
- **S3's whole-app CPU per arm.** `cpu-matrix.sh` is written and ready but unrun: it
  samples `/proc` across `control-raf` → `control-nodraw` → `channel-manual` →
  `render-naive` → `render-worker`, which is the comparison the bench cannot make
  itself. Until it runs, **whether the worker reduces total CPU - as opposed to
  main-thread blocking - is unmeasured**, and that is the figure the Pi 5 decision needs.
- **S3 on other webviews.** Windows/WebView2 and the Pi 5. Tauri's two direct-execute
  thresholds were tuned upstream against WebView2 v135 and macOS, and the 1 ms clock
  clamp plus non-vsync `rAF` that shape every S3 conclusion are WebKit-specific. The
  `Raw`-inflation finding in particular is threshold-dependent and should be re-checked,
  not assumed portable.
- **S4 on other platforms.** Pure Rust, so low risk, but the local checkout's SIMD paths
  are NEON/x86-specific and deserve the aarch64 cross-check on the Pi 5.
- **AUP4 breadth**, now narrow enough to block nothing. Covered: both rates, both
  sample formats, both page sizes, 451 MB to 4.7 GB, single-clip and 19-clips-per-channel.
  Still unmeasured: `project_history` generation 2 (needs one `.aup4` saved a second
  time), a project *created* natively in Audacity 4 rather than converted, an envelope
  with more than one point or a non-unity `val`, and a populated `autosave`.

## Decisions resolved 2026-09-25

1. **One crate per §6 group, not one per leaf** ([ADR-0003](adr/0003-workspace-layout.md)).
   §6's thirty-five leaves become modules. The boundaries worth enforcing - core
   independent of the UI, analysis independent of device access, one crate owning the
   database - are all group-level, and splitting further is cheap later and expensive
   to undo.
2. **`vcw-types` exists although §6 does not list it.** Sample formats, capture modes
   and rates are spoken everywhere; without a shared leaf they would live in
   `vcw-audio` and drag CPAL into the dependency closure of every crate that merely
   wants an enum. The test that makes the case concrete: WP-11's A/B harness has to run
   against the labelled corpus on a machine with no audio stack.
3. **The Phase 0 spikes leave the product workspace.** Finished evidence, not shipped
   code. A Linux-only CI job keeps them compiling so `docs/spikes/` stays reproducible,
   while the product's four-target matrix stays about the product.
4. **D2 locked** ([ADR-0002](adr/0002-sqlite-binding.md)): `rusqlite` with `bundled`.
   The bundling is not convenience - §15 makes recovery a correctness requirement and
   recovery depends on WAL semantics that vary across the SQLite versions distributions
   ship. A recovery test that passes in CI and fails on a Pi because the OS shipped an
   older SQLite is not a test.
5. **D7 and D10 locked** ([ADR-0004](adr/0004-licence-and-toolchain.md)). The two
   clauses that are easy to get wrong: the LGPL exception in `deny.toml` stays
   commented out until `chromaprint-next` actually lands, and the MSRV is enforced by a
   pinned CI job because an untested floor is not a floor.

## Decisions resolved 2026-09-24

1. **`chromaprint-next 0.1.0` from crates.io is the dependency of record**, applying the
   CPAL policy below to the second dependency that had a local checkout. The checkout at
   `/data2/chromaprint-next` sits two SIMD commits ahead of the release; both were
   verified **fingerprint-neutral** out of tree rather than patched in. Same rule as
   cpal: fork only for a defect not already fixed upstream, never a `[patch.crates-io]`
   path copy.
2. **Fingerprinting runs off the capture stream, at the capture rate, with plain `>> 16`
   narrowing.** S4 measured rate, gain and narrowing to be free, so no staging file, no
   pre-decimation and no dither on the fingerprint path.
3. **D6 rewritten, not confirmed.** S3 rejected two of its three clauses. Channels are
   kept for API shape, not speed (indistinguishable from the event bus). `Raw` binary
   payloads are **out** - Tauri eval's them as decimal JSON arrays, making them 42%
   larger than the compact JSON they replace. Send-side coalescing is **out** - 750 Hz
   cost no extra main-thread time and a quarter of the latency; coalesce *paints*
   instead. And D6 gained the clause that actually mattered: **the waveform renders in
   an `OffscreenCanvas` worker**, against 29% of the main thread for a naive redraw.
   Acceptance metric is main-thread occupancy, not fps.
4. **Main-thread occupancy is the UI acceptance metric**, because `rAF` on WebKitGTK is
   not vsync-paced (56–97/s on a fixed 60 Hz output), so fps is not comparable across
   configurations. Corollary recorded the hard way: occupancy is a *blocking* measure and
   says nothing about CPU, which needs external `/proc` sampling.
5. **The Audacity importer takes `wavetrack/@rate` and ignores `project/@rate`**,
   reversing the 2026-09-22 S5 conclusion, which had already reached WP-20's acceptance
   criteria. `project/@rate` is a stored editor preference and reads `192000.0` in all 29
   corpus projects regardless of content. Trusting it would have played 22 of 25 rips at
   4x speed, silently.
6. **AUP4 is a superset of AUP3 in practice, not just in intent.** Same
   `application_id`, one added table, one added record type, ~25 added dictionary names,
   and an audio layer that converts byte-for-byte across five matched pairs spanning
   both rates, both sample formats and both page sizes. Version dispatch is on
   `user_version`, never on the extension or the magic.

## Decisions resolved 2026-09-23

1. **The project extension is `.vcw`** (was `.vripr`). Applied to the spike CLIs,
   `.gitignore` patterns, D1 in the plan, the local-dev config path (`~/.config/vcw/`,
   `vcw.toml`) and every doc.
2. **CPAL: track the released crate.** `shunte88/cpal` is the fork of record for any
   defect we find that is not already fixed upstream - fixes go there first, then
   upstream. It is currently **unused**, because 0.18.2 fixed both defects we had found,
   which is the better outcome. Standing policy: prefer the released crate; fork only
   with a reproduction we cannot get upstream in time; never carry a `[patch.crates-io]`
   path copy again - that is what we had, and it quietly hid how stale our pin was.

## Still open

- **Corpus fixture strategy.** The 30 real projects are the import regression set; they
  need a shrinker (WP-20) so fixtures are committable.
- ~~**Crate naming.**~~ Settled at WP-01 - see [ADR-0003](adr/0003-workspace-layout.md).

## Phase 1 - WP-01, the workspace scaffold

Built 2026-09-25. Ten crates under `crates/`, one per REQUIREMENTS §6 group with §6's
leaves as modules, named `vcw-*`, binary `vcw`. The full reasoning - including why not
one crate per leaf, and why `vcw-types` exists when §6 does not list it - is
[ADR-0003](adr/0003-workspace-layout.md).

What landed:

- **`crates/`**: `types`, `audio`, `signal`, `fingerprint`, `identify`, `metadata`,
  `project`, `export`, `core`, `cli`. Every module file states its scope, its
  requirement sections and the work package that fills it; `missing_docs` is a lint CI
  treats as an error, so a module cannot be added without saying what it is for.
- **Real content where a decision already fixed it.** `vcw-types` carries
  `SampleFormat` with the Audacity code mapping S5 measured - including the `None` arm
  for 32-bit integer, which is the concrete reason D1 is a superset and not a clone -
  plus `CaptureMode` from §9 and §8's six standard rates. `vcw-project::sqlite` carries
  the `.vcw` `application_id` and asserts it is not Audacity's. Seven tests, all
  passing.
- **`vcw doctor`** runs and prints host APIs, the bundled SQLite version and the
  supported rates. A scaffold that runs is worth more than one that only builds.
- **The spikes moved to their own workspace** at `spikes/`, excluded from the root.
  They are finished evidence, not shipped code; a Linux-only CI job keeps them
  compiling so the numbers in `docs/spikes/` stay reproducible.
- **CI** (`.github/workflows/ci.yml`): build + test on four targets, `fmt`, `clippy -D
  warnings`, an MSRV job pinned to 1.90, `cargo deny check`, the spikes job, and an
  assertion that **no crate under `crates/` depends on `tauri`, `wry`, `tao` or
  `webkit2gtk`** - §2 as a test rather than a code-review habit.
- **Licence gates**: `deny.toml` with the permissive allowlist, `THIRD-PARTY-NOTICES.md`
  rewritten for VCW's actual dependency set, and `LICENSE-LGPL-2.1` ported. D7 and D10
  locked in [ADR-0004](adr/0004-licence-and-toolchain.md).
- **ADRs 0001-0004** written, with an index at [`docs/adr/`](adr/).

#### What is verified, and what is not

`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace` and `cargo deny check` are all clean **on Linux x86_64 with
Rust 1.94.1**. The aarch64, Windows and macOS legs exist only as workflow YAML and are
**unverified until the workflow runs on a push** - as is the MSRV 1.90 job, since only
1.94.1 is installed locally. WP-01's exit criterion is "green CI on four targets", so
WP-01 is built but not yet met.

One choice worth flagging: aarch64 Linux runs **natively** on `ubuntu-24.04-arm` rather
than cross-compiled. `alsa-sys` and bundled SQLite are exactly the dependencies a
cross-build gets wrong quietly, and aarch64 is the Pi 5 target, so the cross-compile
saves nothing worth having.

## Phase 1 - WP-02, the `.vcw` schema

Built 2026-09-25. Schema v1 lives in `vcw-project` and is a deliberate superset of
Audacity's AUP4: `sampleblocks` is reproduced column for column, including the
AUTOINCREMENT key and the *absence* of `NOT NULL`, so an imported block needs no
rewriting and stays byte-identical. Everything else is the half Audacity has nowhere
to put.

| module | what it is |
|---|---|
| `schema.rs` | the v1 DDL, heavily commented, plus the identity constants |
| `migrate.rs` | the transactional runner: one transaction per step, `user_version` moved inside it |
| `sqlite.rs` | `Project::create` / `open` / `open_read_only` / `close`, connection pragmas, `block_checksum` |
| `meta.rs` | the six required `meta` keys, §16's versions among them |
| `validate.rs` | `validate()` with 17 finding codes, and `integrity_check()` |
| `error.rs` | a `NotAProject` that tells an Audacity file it wants the importer |
| `doc.rs` | generates [`SCHEMA.md`](SCHEMA.md) from the DDL, comments and all |

Six tables. `sampleblocks` is Audacity's; `captures`, `capture_blocks`,
`capture_diagnostics`, `meta` and `schema_migrations` are ours. `capture_blocks` is the
key one: Audacity keeps block provenance inside its document blob, and recovery cannot
parse a document that was never written, so ours is a table that recovery can read from
committed rows alone.

**Sample formats.** §8 requires 32-bit integer capture and D4 stores 24-bit verbatim,
and Audacity has a code for neither - it knows three formats and pads 24-bit to four
bytes. `StorageFormat` in `vcw-types` reuses Audacity's three codes unchanged and adds
`Int24Packed` (`0x00030002`) and `Int32` (`0x00040002`) in unused space, using
Audacity's own `(width << 16) | type` encoding with a type code it never emits. Nothing
imported is touched; nothing exported is a lie.

#### What is verified, and what is not

44 tests in `vcw-project`, 55 across the workspace, all green, plus `fmt`, `clippy -D
warnings` and `cargo deny check`.

- `tests/aup4_shape.rs` diffs `sampleblocks` against DDL extracted **verbatim** from a
  corpus `.aup3` *and* `.aup4`, column for column via `PRAGMA table_info`, and asserts
  the two Audacity versions agree with each other. This is D1's central claim, now
  checked by CI instead of asserted in prose.
- `tests/roundtrip.rs` writes and reads blocks in all five storage formats across seven
  shapes and compares bytes. It also proves a read-only open honours the `-wal` - the
  writer is held open so the rows exist *only* in the WAL, which is the S5 trap that
  `immutable=1` falls into.
- `tests/migrations.rs` runs a synthetic multi-step set, because one real migration
  proves nothing about the machinery: resumption from any intermediate version reaches
  the same schema, re-applying is a no-op, and a step that fails halfway leaves
  `user_version`, the table set and `schema_migrations` untouched and retryable.
- `tests/schema_doc.rs` regenerates `docs/SCHEMA.md` and fails on drift, then
  cross-checks the parse against `PRAGMA table_info` on a real database - a generator
  is only as trustworthy as its parser, and a self-consistent wrong document is worse
  than none.

Not verified *at the time*: anything a real capture writes. Every test in this section
builds its blocks synthetically. The writer thread, batching and checkpoint policy
arrived with WP-05, which is where the schema first held bytes a capture produced; the
kill-at-random-point recovery suite is still WP-06. D3's block parameters are baked
into `schema.rs` as constants and WP-05's soak is what settled them.

## Phase 1 - WP-03, devices

Built 2026-09-25, **on Linux x86_64 only**. `vcw-audio` answers §7's question - what can
this machine record, through which path, and is that path capable of being bit-perfect -
and §8's - which rates, formats and channel counts will it really accept.

| module | what it is |
|---|---|
| `devices.rs` | `DeviceKey`, `Transport`, `DeviceReport`, `Snapshot`, and `Snapshot::diff` for hot-plug |
| `probe.rs` | the §8 capability matrix, and the advertised-versus-confirmed distinction |
| `selection.rs` | independent in/out preferences, persisted by id, and `resolve()` |
| `error.rs` | errors that name the device and say what happened, including "unplugged" |

**One identity, and it is not the name.** Everything keys on `DeviceKey`, CPAL's
`host:id` - `alsa:hw:CARD=0,DEV=0`. Selection by name is *refused when ambiguous* rather
than resolved to the first match, because on ALSA the same card appears as `hw:` and
`plughw:` under one name and only one of them can be bit-perfect. Picking the wrong one
silently is the failure mode §9 exists to prevent.

**Transport classification.** `hw:` is direct hardware and the only bit-perfect
candidate; `plughw:` is converting; `default`, `pipewire`, `pulse`, `dsnoop`, the rate
converters and the rest are virtual. `Transport::can_be_bit_perfect()` returns
`Some(true)` for exactly one of those three, and `None` where the platform does not say -
an honest "unknown" rather than an optimistic guess.

**Advertised is not confirmed.** A `SupportedStreamConfigRange` is a claim, and S1 found
claims that fail at stream build. The matrix therefore carries three states -
`Advertised`, `Confirmed` (a stream was built and dropped), `Rejected` - and only
`--confirm` promotes anything.

#### What is verified, and what is not

103 tests across the workspace, 48 of them in `vcw-audio`, all green, plus `fmt`,
`clippy -D warnings` and `cargo deny check`.

Measured live on this host, through the new `vcw formats`:

- `hw:CARD=0,DEV=0` (ALC1150 analog in) advertised 8 configurations - 44.1/48/96/192 kHz
  x S16/S32, 2 ch - and **confirmed all 8**. A direct hardware path tells the truth.
- `plughw:CARD=2,DEV=0` (a mono 8 kHz webcam) advertised **1536**: every channel count
  from 1 to 64, at all six §8 rates, in all four formats. Confirming the first two
  channel counts shows it accepts one channel at every rate, plus a stereo fiction at
  48/96/192 kHz that the plug layer manufactures. This is S1's plug-layer fiction,
  reproduced and now machine-checkable.

That second measurement forced a design change: `confirm` opens the device once per
entry, so `Matrix::with_channels_at_most` bounds the sweep and `vcw formats --confirm`
defaults to 8 channels. The *advertisement* is still reported in full - what the backend
claimed is a fact about the backend.

- `tests/hotplug.rs` drives `Snapshot::diff` over synthetic snapshots: appear, disappear,
  reconfigure, rename, and the case that matters most - unplugging a card removes *every*
  path to it, `hw:` and `plughw:` alike, as one event and not three.
- `tests/preferences.rs` proves the rule with no exceptions: **an absent device resolves
  to `Missing`, never to a substitute**, even when the platform default and a plug path
  to the same card are both present and would work. S1 finding 3 is why - the platform
  default here is PipeWire at 44.1 kHz F32, a desktop-audio default that would quietly
  make an archival capture worse than the one asked for.
- `tests/enumerate.rs` runs against whatever hardware is actually present and asserts
  invariants rather than a device list, so it is meaningful on a Pi and on a CI runner
  with no sound card at all.

**Exit criteria only partly met**, and this is the honest position:

- *"Device matrix reported on 3 OS"* - reported on **one**. Windows and macOS are
  unverified. The code has no Linux-specific paths outside `Transport::classify`, but
  untested is untested.
- *"unplug during idle/record is non-corrupting"* - **idle only**. Unplug during record
  needs a running stream, which is WP-04. R9 keeps its fault-injection obligation there.

Two known warts, neither blocking. ALSA's C library writes diagnostics straight to
stderr during enumeration (`snd_pcm_dmix_open ... supports only playback stream`);
silencing it needs `snd_lib_error_set_handler` through `alsa-sys`. And CPAL enumerates
the same card twice on this host, once as `CARD=PCH` and once as `CARD=0` - harmless,
because both keys open the same PCM, but it makes the list longer than the hardware.

## Phase 1 - WP-04, capture

Built 2026-09-25, **on Linux x86_64 only**. This is the work package the whole project
turns on: §9's bit-perfect capture, §10's real-time callback, and §38's provenance. Its
one non-negotiable rule is that the application never claims bit-perfection on the
audio API's word.

| module | what it is |
|---|---|
| `types/capture.rs` | `CaptureState`, `Diagnostics`, `CaptureInfo` - the vocabulary `vcw-audio` and `vcw-project` share without depending on each other |
| `audio/buffers.rs` | the wait-free SPSC ring: whole-chunk-or-nothing writes, a 500 ms floor from S2 |
| `audio/verify.rs` | the OS cross-check. Reads `/proc/asound/.../hw_params` and compares rate, channels and format |
| `audio/capture.rs` | `Request` -> `Negotiated`, the RT callback as `Sink::on_data`, the counters, and `verdict()` |
| `audio/source.rs` | the `Source` trait, and `Simulated` - a device-free capture with fault injection |
| `project/session.rs` | the `captures` and `capture_diagnostics` rows: begin, advance, record, finish |
| `cli/capture.rs` | `vcw capture`, which drives all of it headlessly (§4.5) |

**The callback is provably allocation-free, not assertedly.** The whole body of the
CPAL callback is `Sink::on_data(&mut self, bytes: &[u8])`, an ordinary function over a
byte slice. That is what makes it testable at all - a closure inside a live stream
cannot be driven by a test. `tests/rt_safety.rs` installs a counting global allocator
and runs it 1000 times on the ordinary path, 1000 times while overrunning, 1000 times
on an empty callback, and once while recording a stream error. Every count is zero. The
file's first test is the control: it allocates a `Vec` and asserts the counter *saw*
it, because a broken counter would otherwise "prove" everything.

Lock-freedom is claimed only as far as it is measured. That rtrb is wait-free SPSC and
the counters are relaxed atomics is an argument from construction; what the file
actually demonstrates is the consequence that matters - a consumer parked forever
cannot make a callback take 100 ms, and the ring overruns instead.

**Bit-perfection has exactly one source of truth**, the free function `verdict()`. It
returns `Confirmed` only when all four hold: the OS agrees with the negotiated format,
every field the caller pinned came back unchanged, the transport is direct hardware,
and all four counters are zero. Anything else is `Refuted` with reasons or
`Unconfirmed` with the gap named - never a pass by default. `Source::verdict` is a
*defaulted* trait method precisely so no implementation can override it; a source that
could override it could also lie. And `Negotiated::simulated` reports an unknown
transport and a shared mode, so a simulated capture is refused the claim on two
independent grounds however clean its counters are.

**The verifier is pointed only at `hw:`.** Resolving a `plughw:` id to the card beneath
it would confirm a format the application never received, which is worse than not
checking. `plughw:`, `default`, `pipewire` and `pulse` all resolve to `None` and report
`Unavailable`, and path traversal in a device id is rejected.

**Counters are persisted, not merely counted.** `session.rs` writes both rows in one
transaction, advances `frames` as the capture runs rather than once at the end, and
keys recovery on `finished_at IS NULL` - the absence of a write, which is the one thing
a crash cannot forge.

#### What is verified, and what is not

185 tests across the workspace, all green, plus `fmt`, `clippy -D warnings` and
`cargo deny check`. 106 are in `vcw-audio`, 57 in `vcw-project`, 17 in `vcw-types`, and
5 in `vcw-core`, which is where a test can finally watch a capture's counters land in a
project file - the two crates are deliberately independent, so neither could prove it
alone.

Measured live on this host, through the new `vcw capture`:

- `hw:CARD=0,DEV=0` with nothing pinned negotiated 96 kHz / 2 ch / S32 in exclusive mode
  over a direct hardware path, and `/proc/asound/card0/pcm0c/sub0/hw_params` read back
  `S32_LE 96000 Hz 2 ch`. 294912 frames in 3 s, every counter zero, verdict **bit-perfect,
  confirmed against the OS**. That is the first end-to-end confirmation in the project.
- The same card through `plughw:CARD=0,DEV=0` was refused: exclusive mode was downgraded
  to shared and reported as a divergence, the transport is converting, and the verifier
  declined to resolve the id at all. Two reasons, no claim.
- A request for 22050 Hz - not a §8 rate and not one the card offers - is an error
  naming what the device does offer, not a quiet capture at 44100.

Device-free, in CI:

- `audio/tests/capture_path.rs` runs the deterministic source through the ring and
  compares every byte against a recomputed expectation. "The capture completed" becomes
  "the capture contains exactly the right bytes in exactly the right order".
- **R9, device removal mid-capture**, is closed for the capture path.
  `Faults::unplug_after` makes the source go quiet without ending the stream; the tests
  assert one stream error counted, no overruns, byte-exact data up to the moment it
  went, the counters reaching the file, and `validate()` still clean afterwards. A cable
  cannot be pulled by CI, so it is pulled here instead.
- `core/tests/capture_to_project.rs` also drops a project handle mid-capture with no
  `finish()` call, reopens the file, and asserts recovery can see how far it got and
  that an unfinished capture does not read as a damaged project.

**Exit criteria partly met.** "Callback provably allocation-free and lock-free",
"requested vs negotiated reported" and "counters persisted" are done and demonstrated.
The third clause - *"negotiated format cross-checked against the OS"* - is met **on
Linux only**. Windows needs the WASAPI exclusive-mode format and macOS needs its own
reading; both return `Unavailable` today, which downgrades the verdict to `Unconfirmed`
rather than passing it, so the failure is in the safe direction. But an honest refusal
to claim is not the same as a cross-check, and until those two exist WP-04 stays open.

One deliberate omission: `vcw capture` drains the ring and discards the samples. WP-05
owns the writer, and a second block-committing implementation that nothing else uses
would be worse than none.

## Phase 1 - WP-05, the persistence writer

Built 2026-09-25. WP-02 gave the schema a shape and WP-04 gave the capture path a
voice; until now nothing had ever written a byte of real audio into a `.vcw` file.
Every block in every test was synthetic. This is what closes that.

| module | what it is |
|---|---|
| `types/format.rs` | `StorageFormat::decode_sample` - one stored sample to a normalised `f32`, with the scaling **measured against the corpus**, not assumed |
| `types/capture.rs` | the `PcmSource` trait: `read` and `is_finished`, and nothing else |
| `audio/buffers.rs` | `impl PcmSource for RingReader` - four lines, and the only thing joining the two halves |
| `project/persistence.rs` | `Config`, `Checkpoint`, `Summary`, `pyramid`, `Latencies`, `Progress`, `Writer`, `Outcome`, `spawn`/`Handle` |
| `cli/capture.rs` | `vcw capture --project` now writes the audio, not just the row |
| `cli/soak.rs` | `vcw soak` - the long-run measurement, with a byte-for-byte readback |

**The two crates still do not know about each other.** `vcw-audio` owns the ring and
`vcw-project` owns the writer, and making one depend on the other to join them would
have undone the layering the whole design rests on. Instead `vcw-types` gained a
two-method trait, `vcw-audio` implements it for its own `RingReader`, and the writer
takes `impl PcmSource`. No newtype, no orphan rule, no dependency - and the same writer
can be driven by a generator in CI and by a turntable on the bench with nothing
changing between them.

**The frame count moves inside the block transaction.** `captures.frames` is advanced
in the same transaction that commits the blocks. Were it a separate write, a crash
between the two would leave a project claiming more audio than it holds, and recovery
would have to choose which of two committed facts to believe. Written together, the
count can never run ahead of the data.

**A failed commit stops the writer.** Blocks tile each channel's timeline with no gaps;
`validate()` enforces it and WP-06 will depend on it. Carrying on after a failed commit
would punch a hole no later write could close, so the writer stops, the session is
marked interrupted and the error is surfaced. A short capture that says why it is short
beats a long one with a hole in it.

**The summary pyramid was measured, not ported.** The spike's summariser scaled every
format by `2^(8*bps-1)`, which for Audacity's padded 24-bit would have been 256 times
too quiet - that format is a little-endian `i32` holding a value in +/-2^23, not a
left-justified one. Reading the corpus instead of the spike also turned up two things
worth knowing about `summary256`/`summary64k`, both confirmed against real files:
Audacity sizes the arrays to the block's *capacity* and pads the tail with
`(FLT_MAX, -FLT_MAX, 0)`, and it builds the 64k level from the 256 level by weighting
every group as a full 256 samples before dividing by the true count, so its 64k rms
runs slightly high wherever a block does not divide evenly. VCW computes each level
from the samples with true denominators and emits exactly the groups that exist. The
divergence is under 0.1 % and only in a final partial group; imported Audacity blocks
keep their own summaries untouched under D4, so the two conventions coexist without
either being rewritten.

**A finding that changes D3: the WAL ceiling has to be stated in bytes.** S2 measured a
peak write-ahead log of 4.57 MiB, on a harness using SQLite's default 4 KiB pages. VCW's
pages are 64 KiB, and SQLite's autocheckpoint threshold counts *pages*, so the stock
setting of 1000 is a 64 MiB log rather than a 4 MiB one. `Config::wal_bytes` now states
the ceiling in bytes and converts to pages against the file's actual page size, and a
unit test reads `PRAGMA wal_autocheckpoint` back to prove the conversion happened.

Measured as a pair on `/data2` (ext4), three minutes at 192 kHz each, everything but
the ceiling identical:

| Ceiling | WAL peak | commit p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| 64 MiB (the stock 1000 pages) | 64.71 MiB | 4.9 ms | 8.3 ms | 84.0 ms | 100.1 ms |
| 4 MiB (`Config::wal_bytes`) | **4.50 MiB** | 5.1 ms | 15.8 ms | **31.9 ms** | **54.3 ms** |

4.50 MiB is within 2 % of S2's 4.57 MiB, which is the point: the spike's figure was
right and VCW was silently not reproducing it. The tail improves by 2.6x at p99 and
1.8x at the maximum, because a checkpoint that has 4 MiB to fold back finishes inside
a block period and one with 64 MiB does not. The trade is real and visible at p95,
which gets *worse* (8.3 -> 15.8 ms): checkpoints are more frequent, so more commits pay
a small share of one. That is the right way round for real-time capture, where a rare
100 ms stall is the thing that costs audio and a common 16 ms one against a 250 ms
budget costs nothing.

An earlier version of this section quoted four figures measured on `/tmp`, which is
tmpfs on this machine. They were RAM numbers presented as storage numbers and have been
replaced by the table above.

#### What is verified, and what is not

**The exit criterion is met.** 90 minutes of 24/192 stereo on x86_64/ext4, product
code rather than a spike harness, finished 2026-09-25 11:01:

```
ran         5400.1 s wall, 5400.1 s of audio, real-time factor 1.00001
written     1036824960 frames, 43202 blocks, 5.79 GiB of samples in 21601 commits
commit      p50 5.7 ms, p95 16.2 ms, p99 29.8 ms, max 79.4 ms, budget 250 ms
prepare     p50 6.1 ms, max 18.7 ms (deinterleave, summaries, crc)
wal         peak 4.81 MiB, 0 writer checkpoint(s)
file        5.94 GiB
counters    0 overruns, 0 underruns, 0 dropped frames, 0 stream errors
validate    clean
bytes       every one of 6220949760 matches what the source generated
```

Three things in there are worth more than the headline:

- **"Every byte" means every byte.** `vcw soak` does not compare a checksum. It streams
  `capture_blocks JOIN sampleblocks` in timeline order and recomputes
  `Simulated::expected_sample(frame, channel)` from the frame index *stored in each
  block*, so all 6,220,949,760 sample bytes were checked against what the generator
  would have produced at that exact offset on that exact channel. A block written at
  the wrong offset, on the wrong channel, or after a silent gap fails; it cannot pass on
  its own internal consistency.
- **The worst commit of 21,601 was 79.4 ms against a 250 ms budget**, and the ring holds
  1000 ms. The margin that matters is not the average, it is that the single worst
  moment in an hour and a half still left 170 ms of slack and never came close to the
  ring. Zero overruns is the consequence, not a separate result.
- **The WAL never grew and the writer never checkpointed.** Peak 4.81 MiB across 5.94
  GiB of project, with SQLite's autocheckpoint doing all of it at the threshold
  `Config::wal_bytes` computed; `Checkpoint::Automatic` needed no help. The `-wal` and
  `-shm` sidecars were gone after close, which is what a clean shutdown looks like and
  what WP-06 will use as its signal.

The real-time factor of 1.00001 is the generator's pacing, not a performance figure.
What it establishes is that the writer never became the bottleneck: had it fallen
behind, the ring would have overrun and the counter would say so.

**D3 is firmed by this run.** 250 ms per-channel blocks, batch 1, WAL, `synchronous=FULL`,
a 4 MiB log ceiling and a >= 500 ms ring are now measured on the shipping code rather
than inferred from a spike.


Device-free, in CI:

- `project/src/persistence.rs` holds 19 unit tests: block splitting at the configured
  duration, a short final block written rather than discarded, a trailing partial frame
  held over rather than padded, the frame count checked against the committed blocks
  after *every* commit, per-block checksums, batch size changing the transaction count
  and nothing else, summaries on and off, the pyramid's group count and its short final
  group, the WAL ceiling honoured in pages derived from bytes, progress visible while
  the writer runs rather than only after, and a zero-channel capture refused at
  `begin` rather than spun on - the one input that would make the block loop drain
  nothing for ever.
- `core/tests/capture_writes_audio.rs` is the cross-crate proof, and the only place it
  can live: the real ring on one side, the real writer on the other, joined by
  `PcmSource`. Every sample is checked against `Simulated::expected_sample` recomputed
  from the frame index *stored in the block*, so a block written at the wrong offset, on
  the wrong channel, or after a gap fails rather than passing on its own internal
  consistency. It also covers R9 with the device vanishing mid-capture, a writer dropped
  without `finish()` leaving a project that is valid and visibly unfinished, and the
  stored summaries matching the audio actually in each block.
- `the_writer_keeps_up_with_a_192k_device_in_real_time` is the soak in miniature, short
  enough for CI: three seconds at 192 kHz in a debug build, zero loss, every commit
  inside the block budget.

**R9 can now be closed outright.** WP-04 closed it for the capture path but left it open
because "nothing is writing blocks while the device vanishes". Something is now, and the
test asserts that everything delivered before the cut is in the file, byte-exact, with
the session marked interrupted and the project still valid.

**What is not verified.** The soak is one platform and one filesystem: x86_64 on ext4.
The Pi 5 - on SD *and* on NVMe, which S2 expected to differ - and Windows are both
unrun, and S2's other open questions stay open: disk-full behaviour, induced fsync
stalls, `VACUUM` and compaction, copying a live project, a page-size sweep, and WAL2.
The writer has also never been driven by a real converter for 90 minutes; the long run
is simulated, deliberately, because only a generated source can be checked byte for
byte afterwards.

**One unexplained test failure, recorded rather than forgotten.**
`a_device_that_vanishes_leaves_everything_it_did_deliver` failed once, on 2026-09-25,
during a full-workspace run, and its message was not captured. It has not recurred in
roughly a hundred subsequent executions, including sixteen full-workspace passes and
four with every core saturated. A separate and genuine race in a *different* test in
the same file was found and fixed in the same session - a delivered-frame count read
one line before the feeder thread was joined, so a callback landing in between made
the writer legitimately report more frames than the snapshot saw - but that cannot
explain this one, and no mechanism has been found that does. The assertion now prints
the outcome and the device counters, so a recurrence will be diagnosable. Until then
R9's automated proof should be read as strong but not yet unblemished.

## Phase 1 - WP-06, recovery

**Built 2026-09-25. Exit criterion met, and with it milestone M1.**

§15 asks that the next launch detect unfinished sessions and *offer* recovery.
`crates/project/src/recovery.rs` is that, plus the `vcw recover` verb that drives it.

### What a crash actually leaves behind

The audio survives and the bookkeeping does not. Every block was committed inside its
own transaction with `synchronous=FULL`, so it is on the disk or it never existed;
there is no half-written block. What is missing is the one write that was always going
to come last: the `captures` row's `finished_at`, and with it the final frame count and
state. So a crashed project is not damaged. It is *unfinished*, and the whole job is to
finish it the way the writer would have, using only what the writer already committed.

### The three decisions that shape the module

**Detection is `finished_at IS NULL`, not the state column.** The state says
`recording` because that is what it said while recording, and a crash cannot change it.
`finished_at` is different: it is the absence of a write, which is the one thing a
crash cannot forge. `CaptureState::is_unfinished` still exists but its documentation
now says it is advisory - a hint for a UI, never the thing recovery branches on.

**The blocks outrank the row.** `walk()` is deliberately a per-channel traversal and
not `SELECT SUM(frame_count)`, because a sum would report that a capture with a hole in
the middle has all its frames. It reads the blocks ordered by channel and sequence,
takes the longest *contiguous prefix* of each channel, and the usable length is the
shortest of those prefixes across all declared channels. Anything after a discontinuity
is stranded, not counted.

**`finished_at` is the last block's `committed_at`, never `now()`.** A capture that
died at 14:02 and is recovered at 09:15 the next morning did not run for nineteen
hours. Taking the timestamp from committed data is both honest and reproducible: run
recovery twice and it gives the same answer.

The state becomes a new `CaptureState::Recovered`, kept distinct from `Interrupted`
because the two mean different things. Interrupted means the writer *observed* the
fault and recorded it - a device unplugged, a stream error - so the counters are real
evidence. Recovered means nothing observed anything and every figure was inferred
afterwards. Collapsing them would throw away exactly the distinction an operator needs.
Neither state needed a schema migration: `captures.state` is TEXT, and
`captures.recovered_at` was considered and rejected as a column that would have to be
migrated to store something already derivable.

### D4 enforced rather than documented

Recovery will not silently discard audio. If the walk finds blocks stranded past the
recoverable end, `recover()` refuses with `Error::StrandedBlocks` and the CLI exits
non-zero. `--repair` is how an operator says the loss is accepted, and only then are
the rows removed - from `capture_blocks` and `sampleblocks` both, in foreign-key order,
because deleting one and not the other leaves an orphan that `validate` will rightly
complain about.

### The sidecars, and a finding about them

`Sidecars::inspect` stats the `-wal` and `-shm` **before** anything opens the project,
because opening it is what makes the evidence disappear.

That mattered more than expected. The first version of this was tested in-process, by
dropping a `Project` and reopening it - and the log was never there. Dropping a
`Project` runs `sqlite3_close`, and SQLite checkpoints and *deletes* the sidecars when
the last connection to a file goes. **So an in-process drop reproduces a crash's
database state and not its filesystem state.** Only a process that is really killed
leaves a hot log, which is why WP-06's exit criterion had to be out-of-process. A
second attempt, holding a keep-alive connection open, also failed: a SQLite connection
that has never *read* does not attach to the `-wal`/`-shm` at all, so it is not a
reference that keeps them alive. Both facts are now written into the tests that found
them.

The consequence for the operator is worth stating plainly, because it is the one thing
about `vcw recover` that could surprise someone: **a dry run writes nothing to the
database, but it is not side-effect-free on the filesystem.** Opening the project
replays the log and closing it folds the log into the main file and removes the
sidecars. That is the right behaviour - the log is committed data and folding it in is
how it stops being at risk - but it means a dry run is a report, not a snapshot.
Preserving the crashed state means copying the file and both sidecars together, before
running anything. The test `recovery_reports_before_it_writes` asserts this so nobody
starts believing otherwise.

### Two gaps closed on the way

**`validate` could not see the thing recovery relies on.** Recovery's whole method is
to trust the blocks over the row, and nothing checked that the blocks agreed with the
row or with each other. `check_coverage` adds three codes - `missing-channel`,
`ragged-channels`, `frame-count-mismatch` - taking `validate` to 20. The middle one is
the interesting case: a capture where one channel holds a block more than another would
play as a widening time offset between left and right, and nothing else in the file
would have noticed. The state check also stopped hardcoding its vocabulary and now
calls `CaptureState::parse`, so the list cannot drift from the type.

**The diagnostics were lying about killed captures.** The writer persisted its counters
only at `finish()`, which a killed capture never reaches, so the row held four zeros -
which is the spelling of a *flawless* capture. They are now written on a timer
(`Config::diagnostics_millis`, default 2000) with `updated_at` kept meaningful, so
recovery can report that the counters are, say, two seconds stale rather than quietly
present them as final. `stale-counters` is one of the seven note codes an assessment
can carry.

### The exit criterion

`crates/cli/tests/kill_and_recover.rs` spawns a real `vcw soak` child, sleeps a
pseudorandom interval, and `SIGKILL`s it. Then, before anything opens the file, it
asserts a hot log is present - the state the in-process tests provably cannot create.
Then it runs `vcw recover --apply --verify` as a child process, as an operator would.

The audit afterwards does not trust `captures.frames`, or `sequence`, or the order rows
come back in. Every sample is recomputed from the frame index **stored in that block**
and compared against the generator, which makes the claim not "a plausible frame count"
but "exactly the audio the device delivered, at the offsets it delivered it, and not one
invented sample". It then asserts contiguity from frame 0, equal length on every
channel, `state = recovered`, a non-null `finished_at`, an empty `survey`, and a clean
`validate` with checksums recomputed.

**94 random kills across this session, every one recovered and audited, no failures.**

A real `vcw recover` on a capture killed 3.4 seconds in:

```
/data2/vcw_soak/rec/demo.vcw
  log         4.44 MiB left behind: the last process to hold this file did not close it
  capture 1   156000 frames on every channel, 3.250 s, 26 block(s)
              48000 Hz, 2 ch, Int32, started 1790356014
              0 overrun(s), 0 underrun(s), 0 dropped frame(s), 0 stream error(s), last written 1 s before the end
  verdict     1 capture(s) recoverable; nothing written to the project. Re-run with --apply
```

3.4 seconds of process life, 3.25 seconds recovered: one uncommitted block.

### The loss is smaller than the model allowed for

Across all 94 kills, every recovered length came back an **exact multiple of the 250 ms
block**, and the shortfall never reached one whole block - with a 1000 ms ring in play
the entire time. So the ring contributes nothing to crash loss: a writer that keeps up
drains it before the crash matters. That confirms the correction S1 made to S2's floor,
**crash loss = commit granularity + driver buffer, and ring size is irrelevant**, and
the test now asserts the tight bound rather than the safe one, because a regression
that let the ring leak into the loss would sail through the loose one.

#### What is verified, and what is not

Verified: detection, reconstruction, the stranded-block refusal, the sidecar lifecycle,
the timestamp provenance, the periodic counters, the three new validate codes, and the
byte-for-byte audit after 94 out-of-process kills. 234 tests, full gate green.

**Not verified, and it is not a portability gap.** `SIGKILL` ends a process; it does
not cut power. The page cache survives, so this exercises SQLite's crash recovery and
not the storage stack's. `synchronous=FULL` fsyncs every commit before it returns,
which *should* mean the two are the same, but "should" is the honest word and nothing
here demonstrates it. Closing the gap needs real power cuts on an expendable rig or a
fault-injecting filesystem; both are already on S2's open list.

Also unverified: every platform except this one. The kill suite is the cheapest of the
outstanding portability runs, needing neither a sound card nor an operator - one
`cargo test -p vcw-cli -- --ignored` per rig.

## Next up

**`WP-07`, the engine and the state machine.** The CLI already drives a capture end to
end and now recovers one, which is most of what WP-07's exit criterion asks for
behaviourally; what it does not have is the type-level guarantee that an invalid
transition cannot be written down. WP-06 also hands it a new obligation: a project
opened at launch with an unfinished capture in it is a state the machine has to model
from the outside, not discover.

Still open on WP-03 and WP-04, and both for the same reason: **Windows and macOS.** The
device matrix is reported on one OS, and the OS format verifier exists for Linux/ALSA
only. On the other two the verdict degrades to `Unconfirmed` rather than to a false
pass, which is the right failure, but neither work package can close on it.

WP-05 and WP-06 have the same shape of gap: the 90-minute soak and the kill suite have
both run on x86_64/ext4 and nowhere else. The Pi 5 on SD and on NVMe, and Windows, are
the runs that would close them, and they need no new code - `vcw soak` and
`cargo test -p vcw-cli -- --ignored` are the harnesses.

Two measurement jobs stay queued and can run on the machine's own time: S3's
`cpu-matrix.sh`, and S3's two R8 isolation soaks. **D3's firmed-config soak is no
longer among them** - WP-05's exit soak is that run, with the product code rather than
the spike harness.

## Housekeeping

- All of Phase 0 is committed: the spikes, the CPAL 0.18 upgrade and the `.vcw` rename
  at `a28fd85`, the S3 IPC bench at `096a8a0`, and S4, S5 and the AUP4 delta at
  `19dd459`. WP-01 is committed at `cd8e445`, WP-02 at `acb8835`, WP-03 at `941981a`,
  WP-04 at `95f1f52` and WP-05 at `358c44a`. **WP-06 is the current working-tree change
  and is uncommitted.**
- **`/data2/vcw_soak/`** holds what is left of the WP-05 soak: `wp05.log`, the run
  transcript quoted above, and `live.vcw`, the four-second hardware capture. The 5.94
  GiB `wp05.vcw` has been deleted, as have the two three-minute WAL-pair projects.
  `rec/demo.vcw` is the killed capture quoted in the WP-06 section. Nothing here is in
  the repository and all of it is disposable.
- **`/data2/source_rips`** is the source-audio corpus S4 ran against: 62 real vinyl rips,
  71 GB, 48 kHz and 192 kHz 32-bit WAV plus 24-bit FLAC. Distinct from
  `/data2/vinyl_rips`, which holds the 30 *projects* S5 used - 25 AUP3 and 5 AUP4, the
  latter five being conversions of AUP3 projects already in the set. Several titles
  appear in both corpora, which will make a good import round-trip test at WP-20, and
  the five matched pairs are the AUP3/AUP4 equivalence fixture.
- `.bench/` holds **~8 GB** of scratch databases - the 8.4 GB soak artefact plus older
  `.vripr`-suffixed files from before the rename. All gitignored and safe to delete; the
  S1 and S2 numbers are recorded here and in the spike write-ups.
- Licensing today: MIT core, cpal Apache-2.0 as an ordinary dependency.
  `chromaprint-next` adds an LGPL-2.1-or-later relink obligation at Phase 2.
- **Stay current on CPAL.** Two blocking defects and the device-id API all landed within
  two minor releases; pinning 0.16 had already cost us a fork.
