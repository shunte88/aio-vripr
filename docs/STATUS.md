# VCW - project status

**As of:** 2026-09-26
**Phase:** 1 is underway - WP-01 through WP-11 are built, all on Linux x86_64 only.
All five Phase 0 spikes returned verdicts on their primary platform; gate G0 remains
open on hardware coverage, WP-05's soak settled D3's firmed-config run, **WP-06 closes
milestone M1, *it records*,** WP-07 locks D8, WP-08 adds the meters and the §10 fan-out
they read through, WP-09 draws the waveform - and cost the schema two covering
indexes to do it in milliseconds rather than seconds - **WP-10 closes milestone M2,
*it plays back*,** with a seek that joins in a median 19.8 ms on hardware and byte-exactly
in CI, and **WP-11 closes milestone M3, *it finds tracks*,** at 97.6% to 99.7% parity
with VRipr over 595 labelled snippets, exact to the frame.
**Branch:** `main` at `91ba45f` (WP-10), pushed. WP-11 is in the working tree,
gate-green, uncommitted.

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

The same hazard turned up in WP-11's own extractor, and the note is why it was looked
for: `features::Windows` was dropping the orphan sample at the end of each call rather
than transposing it, which is quieter and just as wrong. It now carries the part-frame
between calls. The fingerprint worker still has to do the same when it arrives.

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
6. **D8 locked** ([ADR-0005](adr/0005-concurrency-model.md)): dedicated OS threads on
   the capture path, `mpsc` between them, no async runtime near audio or SQLite, and
   Tokio only when network I/O arrives at WP-12. Two clauses of the original wording
   changed on contact with WP-07 - the engine thread is *required* rather than
   preferred, because `cpal`'s stream handle is `!Send`, and elevated thread priority
   is not implemented, because no soak has yet needed it.

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

## Phase 1 - WP-07, the engine

**Built 2026-09-25. Exit criterion met, both halves.** `vcw-core` was a directory of
module stubs; it is now the layer that composes `vcw-audio` and `vcw-project` into a
transport, and `vcw session` is the operator surface that drives it. About 2,800 lines
across four modules, plus 730 lines of integration test in two places.

### §11 as a type, not a check

The state machine is a **typestate**. Five concrete phase types - `Idle`, `Armed<D>`,
`Recording<D>`, `Paused<D>`, `Stopped<D>` - and each transition *consumes* the phase it
leaves and returns the one it enters. There is no `Phase::Recording` variant to be in
while the deck says otherwise, and no runtime guard to forget: `Idle` has no `stop`
method for anyone to call, `Stopped` has no `record`, and a `Paused` that has been
resumed no longer exists to be resumed a second time.

Two additions to §11's diagram, both documented as additions rather than slipped in.
`Armed -> Idle` exists because an operator who opens a device to set a level has to be
able to change their mind, and `Stopped -> Idle` exists because §11 says stopping does
not close the project, which only means anything if there is a way back to record the
second side.

What the phases drive is a `Deck` trait rather than a `Recorder` directly, which is what
lets §11 be exercised exhaustively with no device, no disk and no project. `Rehearsal`
is the test deck and it is *public*: the compile proofs need a concrete `Deck` that does
not need a sound card, and a UI being developed with nothing plugged in needs the same
thing.

`Refused<S, E>` is the part worth pointing at. A transition a deck declines hands the
**phase back** - `Err(Refused { from: Recording, error })` - so a failed pause does not
lose a capture. That is §36's "task failure shall be isolated wherever possible"
expressed in a signature rather than in a comment.

### The bus is §35's two halves and nothing else

`Command` in, `Event` out. A `Command` carries a *description* of what to open, never a
device: it crosses a process boundary at WP-15, and a `cpal` stream handle is `!Send`
and could not cross a thread boundary let alone that one. `Bus::publish` fans out to
any number of subscribers, prunes the ones that have gone, and **cannot fail the
engine** - a poisoned lock returns zero rather than propagating, which is the one place
that trade-off is made deliberately.

Both enums are `#[non_exhaustive]`, so `play`, `seek` and `export` are additions at
WP-10 onwards rather than breaking changes. Inside `vcw-core` the attribute does
nothing, which is the useful half: the engine's `match` over `Command` is exhaustive on
purpose, so adding a variant fails to compile until someone decides what the transport
does with it.

### Armed is a writer that is running and paused

§11's `Armed` could have been "device open, writer not started". It is instead "writer
started, and paused", and that one choice pays for three things at once: §50's *set the
level before you drop the needle* and §11's PAUSE become the same mechanism; the ring is
always drained by the thread built to drain it, so overrun counters stay honest while
nothing is being committed; and §15's early session row falls out for free, because the
capture exists in the project from the moment the device opens.

It needed a small addition to WP-05's writer: `Config::start_paused`, a `pause`/`resume`
pair on the handle, and a writer loop that reads the ring and **drops** what it reads
while paused. Pausing flushes the part-filled block on the way in, so the audio captured
before the pause is committed rather than held.

### One thread, and it is not a matter of taste

The engine is a dedicated OS thread, and the transport is a **local variable** moved
through its loop. No `Arc<Mutex<Machine>>`, and therefore no sixth "in transition" phase:
between any two statements the transport is exactly one of §11's five. The typestate only
works because a single thread owns it - a shared, locked transport would have to hand out
`&mut`, and the consuming transitions are precisely what make an illegal move
unrepresentable.

That the thread is *necessary* rather than merely tidy comes from CPAL: `Capture` is
`!Send`, so the thread that opens a device must be the thread that keeps it and therefore
the thread that takes every later command about it. This is D8, locked as
[ADR-0005](adr/0005-concurrency-model.md). Two clauses of the plan's original D8 wording
changed on contact with the work: the above, and **elevated thread priority is not
implemented** - WP-05's 192 kHz soak showed no overruns at ordinary priority, so it stays
available for a platform that needs it rather than applied speculatively.

### Three things the build got wrong first

**The finished frame count.** `Stopped` took its position from the deck *before* the
stop, and finalising flushes the part-filled block, so the transport reported a length
up to one block shorter than the row in the project. Fixed by giving `Deck` an
associated `frames(&Report)` function: only the deck knows what its own report means,
and by the time there is a report there is no deck to ask.

**When a capture is finished.** `capture-finished` was published when the transport was
*reset*, because that is where the report is yielded. An operator who stops a side and
walks away would never have been told what was recorded. It is now published on the stop,
and a test pins it to exactly one occurrence - the report lives in two places and
publishing it twice would have a UI catalogue the side twice.

**The terminator.** `closed` was the last statement in the thread function, which means
it was not sent if the thread panicked, and a consumer blocked on `Events::next` would
have waited for ever. It is now published from a **drop guard**, so it survives a panic;
`Bus::publish` was already infallible, which is what makes that safe to do while
unwinding.

### The exit criterion, first half: the compile proofs

Six `compile_fail` doctests, one per illegal move, each **paired with the legal twin that
must still compile**. The pairing is not decoration. Measured this session: stable
rustdoc **ignores the error code** in a ```compile_fail,E0599``` fence - a doctest
annotated `E0599` passed while the code was actually failing with `E0308`. So
`compile_fail` proves only "this did not compile", which a typo satisfies. The proof was
then verified live: one illegal snippet was temporarily made legal, and the doctest
failed as it should. It is a live proof, not a decorative one.

### The exit criterion, second half: a full session from the CLI

```
$ vcw session side-a.vcw --script "arm,record,sleep 1,pause,sleep 0.3,poll,resume,sleep 0.6,stop,poll,reset,quit"
[  0.043] armed               armed on 48000 Hz, 2 ch, S32, shared into side-a.vcw
[  0.043] phase-change        idle -> armed
[  0.043] phase-change        armed -> recording
[  0.343] recording-position  12000 frames, 0.250 s
[  0.844] recording-position  36000 frames, 0.750 s
[  1.000] phase-change        recording -> paused
[  1.302] status              paused, 48000 frames
[  1.302] phase-change        paused -> recording
[  1.602] recording-position  60000 frames, 1.250 s
[  1.941] phase-change        recording -> stopped
[  1.941] capture-finished    capture 1 finalised: 76800 frames, 0 overrun(s), 0 underrun(s), 0 dropped, 0 error(s), bit-perfect no
[  1.941] status              stopped, 76800 frames
[  1.941] phase-change        stopped -> idle
[  1.941] closed              closed
```

One verb per line from stdin, or a whole session on one line with `--script`. Two verbs
are the driver's rather than the core's: `sleep <seconds>`, which is what makes a script
a session rather than a list, and `#` for a comment. `--json` emits one object per event
per line. There is deliberately **no prompt**: events arrive on their own thread whenever
the engine has something to say, and a prompt would be scribbled over by the next
position report, so the session echoes each command into the transcript instead and the
whole run reads back in order afterwards.

`crates/cli/tests/session_from_cli.rs` runs six of these through the **shipped binary**,
then re-opens the project and checks that the audio matches what the transcript claimed,
with every checksum recomputed. Including: a script that forgets to `stop` (the shutdown
finalises the side rather than abandoning it), a verb with a typo in it (the run fails,
*after* the audio is safe), three commands issued out of turn (rejected, and the project
is left as it was found), and an arm that is thought better of (no capture row at all).

### What is verified, and what is not

Verified: the whole of §11's diagram walked in both directions; every step that is not in
the diagram illegal from every phase, checked exhaustively; a deck that refuses each of
its four operations, including a stop that fails; the clock discounting paused time; two
sides into one project; a device that cannot be opened leaving the transport idle; a
shutdown mid-capture finalising rather than abandoning; two subscribers seeing an
identical stream; a panicking engine still closing the stream; and a full capture driven
through the binary with no frontend compiled. 277 tests, full gate green.

**Not verified: any platform but this one.** Every claim here is Linux x86_64. The
transport itself is platform-independent, but the simulated source is what most of the
tests drive, so what has *not* been exercised anywhere is the engine holding a real
`!Send` stream on Windows or macOS - which is precisely the case that motivated the
thread. `vcw session --device <id>` is the one-line way to check it on a rig with a
converter attached.

**Not attempted: re-entering the transport from a recovered project.** A project opened
with an unfinished capture in it is a state `vcw recover` reports and the transport knows
nothing about. Correct for now, since §15 asks recovery to *close* a capture rather than
resume one, but WP-16's "there is unfinished audio here" banner will have to decide what
the transport shows while it is up.

## Phase 1 - WP-08, the meters

Built 2026-09-25. `vcw-signal::meter` measures the levels; `vcw_audio::buffers::Tee`
carries the audio to it; `vcw-core::metering` is the worker between them. Exit criterion
met: verified against known-level test signals, and then verified again against a live
engine.

### Full scale is not 1.0, and that is the whole module

The finding that shaped everything else. In two's complement the largest positive `i16`
code is 32767, which decodes to 32767/32768 = **0.99997**, while the most negative is
-32768, which decodes to exactly **-1.0**. Full scale is asymmetric, and it is asymmetric
differently in each storage format.

A clip detector written the obvious way - `if sample.abs() >= 1.0` - therefore never
fires on integer input at all. It would be silent through an entire side pinned against
the top of the converter, which is exactly the fault a clip light exists to show. So
`full_scale(format)` returns a *pair*, and each sample is tested against the ceiling and
the floor separately.

One limitation is measured and documented rather than papered over: at 32 bits, `f32`'s
24-bit mantissa rounds the top few hundred `i32` codes to exactly 1.0, so clipping there
is detected a few codes early. That is a rounding error of about -0.00001 dB and it errs
towards reporting a clip that was within a hair of being one.

### The three measurements, and what each trades away

**Peak is since the last read.** Taking a snapshot resets it, so no transient can pass
between two polls unseen. The cost is that the number depends slightly on how often the
UI looks. The alternative - a peak that decays on its own clock - reads the same at every
poll rate and loses transients to do it, which is the wrong way round for a meter whose
job is to catch the one loud moment in a side.

**RMS is a true sliding window**, 16 buckets over 300 ms, advanced by sample count and
not by the reader. A per-snapshot mean would have been simpler and would have made RMS a
measurement of the UI's frame rate: 30 Hz and 60 Hz would read differently on identical
audio. `rms_does_not_depend_on_how_often_the_ui_looks` pins that, and
`the_rms_window_forgets_what_has_left_it` pins the other half - a loud passage that has
left the window is gone from the figure.

**The hold needle starts falling from the moment of the peak,** not from the moment the
signal stops, and falls at a rate in dB/s. This cost a test a correction: the first
expectation was out by exactly one bucket-step of 0.125 dB, because it had assumed the
hold clock started when the tone ended.

The clip latch stays lit until it is cleared, with a configurable n-consecutive-samples
rule for anyone who wants more evidence than a single sample.

### The fan-out sits after the ring, and the taps are lossy

§10 draws one distribution point feeding the writer, the meter, the waveform and the
detector. `Tee<S: PcmSource>` is that point. It wraps whatever the writer was going to
read from, copies each read into every tap, and is inserted at exactly one place -
`Recorder::open`, where the reader is handed to `persistence::spawn_on`.

That one place was chosen over the tempting one. A tap inside the CPAL callback would
have been closer to the source, but the simulated generator has no callback and no ring
at all, so a device-only fan-out would have left the UI-with-nothing-plugged-in case
unmetered - and being able to develop the whole application against it is §4.5. The
callback also keeps its guarantee untouched: three atomics and one memcpy, still proven
allocation-free by WP-04's counting-allocator test.

**Taps drop rather than block.** A meter worker that stalls loses audio it was only going
to average; a writer that stalls loses the record. The trade is stated in both
directions in the module doc: a stalled writer freezes the meter, and that is the
direction §10 requires. A tap counts the bytes it could not keep, so falling behind is
visible rather than silent.

### The meters run while armed, because §50 says so

"Set Level" comes before "Drop Needle". The meters are therefore live in `Armed`, which
cost nothing to arrange: WP-07 implemented `Armed` as a writer that is *running but
paused*, so the ring is already being drained and everything drained already goes past
the tap. The meter measures what the device is producing; the transport phase is not its
business.

`meter-update` publishes at 50 Hz, the middle of §17's 30-60. One refinement came out of
watching a real transcript: a tick that finds no new audio publishes **nothing**. Reading
a snapshot resets the peak, so a tick that woke a millisecond early was reporting silence
the stream never contained, and the needle flicked to the floor roughly once a second.
Genuine silence still reports correctly, because a quiet device sends zeros and zeros are
frames.

### What is verified

Thirteen known-level tests in `crates/signal/tests/known_levels.rs`, all against signals
whose level is known before the meter runs rather than recorded from it:

- sines at -0.5, -6 and -20 dBFS, in all five storage formats, read back to within
  0.02 dB, with RMS exactly 3.0103 dB below peak as a sine must be
- a constant reading the same peak and RMS; silence reading the floor in every format
- channels metered independently; RMS independent of poll rate; the window forgetting
  what has left it
- one sample at full scale latching, and staying latched; the top integer code clipping
  although it is not 1.0; the n-consecutive rule
- the hold needle falling at the rate it was given
- the five formats agreeing with each other on the same signal

Then the other half, which known levels cannot prove: that what reaches the meter *is*
the capture stream. `crates/core/tests/metering_live.rs` drives the real engine and
asserts the reported RMS is **-4.771 dBFS**. That figure is not a recorded observation -
the deterministic source is a hash, so its output is uniform over the code range, and the
RMS of a uniform distribution on [-1, 1) is 1/sqrt(3). A fan-out that dropped, duplicated
or reordered a byte would move it. The same file pins the meters running before `RECORD`,
no `meter-update` arriving after `capture-finished`, and a capture with the fan-out
attached still reporting zero dropped frames and zero overruns.

299 tests, full gate green.

### Cost

192 kHz stereo, the worst case the product supports: 10 s of audio metered in 0.377 s in
a debug build and **0.033 s in release**, about a third of one percent of a core. The
meter is not a thing to budget for.

### What is not verified

Linux x86_64 only, like everything above it. And the fan-out has been exercised against
the simulated source and the ALSA device on this machine, not against a converter running
for an hour - the lossy-tap behaviour under sustained real load is the WP-05-style soak
that has not been run with meters attached.

## Phase 1 - WP-09, the waveform pyramid

Built 2026-09-25. `vcw-signal::waveform` renders; `vcw-project::waveform` reads;
`vcw-types::summary` holds the triplet both of them agree on. Exit criterion met, and
measured on a real 26-minute vinyl side rather than on a generated signal.

### The split, and why the query lives in the project crate

ADR-0003's rules that bite here are two: analysis must never reach a device, and only
one crate may open the project database. So the renderer knows nothing about SQLite - it
takes summaries and samples and returns columns - and every statement lives in
`vcw-project`, which is therefore allowed to depend on `vcw-signal`. `vcw-signal` depends
on `vcw-types` alone, so nothing circular is possible.

The triplet moved out of `vcw-project::persistence` into `vcw-types::summary` on the way,
because the writer computes it and the reader folds it and one definition is the only way
those two stay in agreement.

### RMS composes exactly, and Audacity's does not

`Summary::merge` weights by **true sample count**:
`sqrt((n1*r1^2 + n2*r2^2)/(n1+n2))`. That is exact, which is what makes the pyramid
honest: a column drawn from stored triplets is the same number the samples themselves
would have produced, so zooming out changes the resolution and not the answer.

Audacity weights its 64k level by block *capacity* instead of by the count actually in
the block, which makes its RMS slightly high on any short final block. Measured against
`/data2/vinyl_rips/simples_test.aup3` and pinned in
`weighting_by_capacity_instead_of_count_is_what_makes_audacity_high`, so the difference is
recorded rather than inherited by accident when import lands.

### `Summary64k` is dead weight for anything VCW records

The ladder is 1 frame, 256 frames, the block, and 65,536 frames - and at D3's 250 ms
block the last rung is *coarser* than the one below it: 65,536 frames against 12,000 at
48 kHz and 48,000 at 192 kHz. It also needs a blob parsed to reach the same rows the
block level already has as three scalar columns.

So it is never chosen for a capture VCW wrote. It is still *written*, for AUP4
compatibility (§49), and still readable, for imported Audacity blocks, which is the only
place it can ever be the right rung.
`the_64k_level_is_never_read_from_a_capture_we_wrote` is the test that keeps that true.

### The finding: a 192 KB blob makes three floats expensive

This is the part that was not predicted and that decided the schema.

A `sampleblocks` row at 24/192 carries 192 KB of audio, so with a 64 KiB page size it
occupies pages of its own and nothing else shares them. Reading `summin`, `summax` and
`sumrms` - twelve bytes - still costs a page fault per block. A 26-minute side is 12,528
blocks, so a full zoom-out is **784 MiB of page reads to obtain 150 KB of triplets**.
Cold on ext4 that measured **3.77 s**, against a requirement of sub-second, and no amount
of care in the renderer could have touched it.

Two covering indexes fix it by putting the coarse rungs somewhere the audio is not:

| Index | Columns | Size on a 2.3 GiB side |
|---|---|---|
| `sampleblocks_levels` | `blockid, summin, summax, sumrms` | 576 KiB, 0.02 % |
| `sampleblocks_summary256` | the same, plus `summary256` | 28 MiB, 1.2 % |

The query names them with `INDEXED BY`, which is deliberate in two ways. SQLite left to
itself prefers the integer primary key and produces the slow plan; and `INDEXED BY` is an
assertion rather than a hint, so if an index ever goes missing the query fails loudly
instead of quietly reverting to three seconds in front of a user.

The second index has to repeat the whole-block triplet as well - twelve bytes beside two
kilobytes - because the reader falls back to it for a block with no summary blob. Written
without those three columns it is a *lookup* index rather than a covering one, SQLite
fetches the row after all, and the entire 28 MiB buys nothing. That was measured, not
reasoned: the first version of the index made no difference at all, and
`EXPLAIN QUERY PLAN` said `SEARCH sb USING INDEX` where it now says `USING COVERING
INDEX`.

`the_coarse_levels_never_touch_a_row_that_holds_audio` locks it down **structurally**,
by asserting on the plan, not by timing anything. The difference it guards is two
hundred fold, and a timing test for it would still have been flaky.

There is no index for `summary64k`. Nothing VCW writes reads that rung, and the blocks
that do need it - imported ones - have no `capture_blocks` row and never reach this
query. If import makes it hot, that is the time to measure it.

### The measurement, on real music

A 26-minute 192 kHz stereo 32-bit side from `/data2/source_rips` was pushed through the
product writer onto `/data2` (ext4, SATA SSD): **300,627,479 frames, 12,528 blocks,
2.33 GiB**. Every read below had the page cache evicted first with
`posix_fadvise(DONTNEED)`, so these are cold numbers, and each draws **both** channels.

| Span | Width | Rung | Cold |
|---|---|---|---|
| whole side | 160 px | block | 21.8 ms |
| whole side | 1920 px | block | 17.6 ms |
| whole side | 4000 px | block | 17.0 ms |
| whole side | 8000 px | summary256 | 303.5 ms |
| 8 min | 1920 px | summary256 | 74.0 ms |
| 100 s | 1920 px | summary256 | 19.9 ms |
| 10 s | 1920 px | summary256 | 5.7 ms |
| 1 s | 1920 px | samples | 18.7 ms |
| 50 ms | 1920 px | samples | 6.2 ms |

Before the indexes the first row was 3,767 ms, the eight-minute span 1,347 ms and the
8000 px draw 4,170 ms. Worst case anywhere in the sweep is now 304 ms, for a whole-side
draw at a width no display can ask for.

The picture is worth having as well as the timings. At 160 columns the side shows its
track gaps as narrow notches and its lead-out as a single tall spike, peak 0.9332 on the
left and 0.9096 on the right - which is what a vinyl side looks like and is not what
uniform noise looks like. The generated source the tests use draws a featureless block,
correctly, and would have hidden any error that depended on real dynamics.

### Regeneration from PCM, on the same side

§19 asks for the pyramid to be regenerable from the audio, and a toy capture cannot
really test that. So the 26-minute side had every `summary256` and `summary64k` blob set
to `NULL` and `vcw waveform --rebuild` pointed at it: **12,528 of 12,528 blocks rebuilt
from the stored audio in 78.7 s**, reading all 2.3 GiB of PCM to do it.

The drawing that came out is **byte-identical** to the one the writer's own summaries
produced, at the block level and at the 256-frame level alike - 1,920 columns of a 100 s
span compared field by field. That is the claim worth making about a pyramid: it holds no
information the audio does not, so losing it costs time and nothing else.

### What §37 actually claims, restated

"Render cost independent of total length" is true and is easy to overclaim. Three
separate things, measured separately:

1. **The output is always exactly `pixels` columns.** Constant by construction, whatever
   the span.
2. **At any zoom coarse enough to reach the block level, the read is independent of
   sample count.** The same 10 s of audio costs 267 µs at 192 kHz and 292 µs at 48 kHz -
   four times the samples, no extra cost, no blobs opened - and a full zoom-out costs the
   same 17 to 22 ms at 160, 1920 and 4000 columns.
3. **At mid zoom the read is proportional to the samples in the span, never to the length
   of the recording the span came from.** The same 5 s drawn out of a 10 s capture and a
   200 s capture: 18.8 ms against 19.3 ms. Twenty times the recording for 2.5 % more
   time.

What is *not* claimed: that a wider span is free. It is not, and the table above shows
it climbing with span until the ladder steps up a rung, at which point it falls again.
A 500 s span at 1920 px costs 8 ms while a 100 s span costs 20 ms, because the wider one
reaches the block level and the narrower one does not.

### What it cost the capture path

Two more indexes to maintain on every commit. `blockid` is an autoincrement key, so both
inserts land at the end of their b-tree and neither rebalances. A five-minute real-time
24/192 soak: commit p50 6.2 ms, p95 17.3 ms, **p99 28.9 ms, max 42.5 ms** against the
250 ms block budget, peak WAL 5.06 MiB, zero loss and every one of 345,657,600 bytes
matched against what the source must have generated. File size grows 1.4 %.

**The 90-minute soak was re-run against the new schema on an idle machine and it
passes.** The two covering indexes cost the capture path nothing measurable.

| | With the indexes | Reference (pre-index) |
|---|---|---|
| real-time factor | 1.00001 | 1.00001 |
| commit p50 / p95 / p99 | 6.9 / 18.4 / **32.8** ms | - / - / **63.2** ms |
| commit max | **102.6 ms** | 102.3 ms |
| prepare p50 / max | 6.0 / 12.7 ms | - |
| peak WAL | 5.25 MiB | 4.57 MiB |
| overruns / underruns / dropped | 0 / 0 / 0 | 0 / 0 / 0 |
| `validate` | clean | clean |
| byte readback | all 6,220,938,240 | all of them |

The two numbers to look at are the maximum commit and the write-ahead log. **The worst
commit is 102.6 ms against the reference's 102.3 ms** - three tenths of a millisecond
apart over 21,601 commits, which is as close to "no effect" as a measurement of this
kind gets. The p99 is better rather than worse, at 32.8 ms against 63.2 ms, which is
machine state rather than the indexes helping; the honest reading of both together is
that maintaining two append-only b-trees on an autoincrement key disappears into the
noise of the commit the writer was already doing. **Peak WAL rose 15 %,** 4.57 MiB to
5.25 MiB, which is the indexes' own pages passing through the log and is the one cost
that is actually visible. It stays bounded and nowhere near a ceiling.

**The ring is not under-sized, and the earlier worry about it is closed.** A previous
attempt at this run recorded a 965 ms commit against a 1,000 ms ring and 28 overruns,
and raised the question of whether the ring is sized against the mean rather than the
tail. On an idle machine the worst commit in ninety minutes is 102.6 ms: ten times the
headroom, and the 965 ms stall was contention rather than anything the writer does.

That earlier attempt failed, **and the failure was mine** - I ran the full gate on the
same machine while a real-time soak was going, on the reasoning that a pass under
contention would be a stronger result. It is not a stronger test, it is a spoiled one,
and it cost ninety minutes and settled nothing. Kept at
`/data2/vcw-scratch/soak90-contended.log` because one thing in it is worth keeping: the
byte mismatch it reported was provably the dropped audio and not corruption. The
verifier found frame 195,888,000 of channel 0 holding `D9 90 24` where the source would
have produced `8D 94 7C`, and searching the generator over the next 300,000 frames finds
exactly one frame producing `D9 90 24` - frame 195,941,760, which is 53,760 later and
exactly the reported drop count. The writer stored what it was handed, in order,
unaltered; the ring lost 280 ms and everything after was shifted by it, with `validate`
clean saying the same from the checksum side. That is the failure mode a byte-for-byte
verifier exists to distinguish, working.

### The CLI

```sh
vcw waveform side-a.vcw --pixels 160 --rows 21
vcw waveform side-a.vcw --start 300 --end 400 --pixels 1920 --json
vcw waveform side-a.vcw --rebuild
```

It reports which rung it read and how long the read took, so every number above is
checkable on any machine without a UI. `--rebuild` recomputes the pyramid from the stored
PCM before drawing, by default only where a summary is missing; it never writes `samples`
and never touches a block with no `capture_blocks` row, so an imported Audacity project
cannot be rewritten by a redraw.

### Tests

329 in the workspace, full gate green. 11 unit tests on the renderer, including that the
answer has exactly as many columns as pixels were asked for, that the level chosen is the
coarsest that still fills every pixel, that a run straddling a column boundary is split
by how much falls each side, and that a backwards span is empty rather than enormous.
Nine integration tests on the reader, including that every level gives the same answer
for the same span, that the pyramid can be thrown away and rebuilt identically, and that
an unknown capture is an error rather than an empty picture. Four on the CLI verb. Two on
the query plan.

One test needed a genuine correction rather than a fixed expectation: the span-independence
test was reading *different audio* from its two captures, because the helper it used went
silent at the halfway point of whichever capture it was filling. A second helper whose
value is a function of frame index alone fixed it, and the first is now documented as
unusable for span comparisons.

### Fixed on the way past

`recovery::tests::a_log_left_behind_is_visible_before_anything_opens_the_project` was
flaky at about one run in twenty-five, and had been since WP-06 - confirmed by looping it
40 times in a worktree at `75123ab`, before any of this work. It asserted that a
checkpoint folds *exactly* the bytes an inspection saw, but `Project::open` stamps
`last_written_at` on its way in and can add a frame of its own, depending on the
checkpoint SQLite attempts when the writer's connection closes. The claim worth making is
that the checkpoint found everything the inspection did, so it is now a floor. 40 runs
clean.

### What is not verified

Linux x86_64 only, like everything above it. The measurements are from a SATA SSD with a
64 KiB page size; the Pi 5's SD card is where the `summary256` rung is most likely to
hurt, and that is the run to take before anyone adds a fourth rung on instinct. Nothing
publishes a waveform event yet either - §19's progressive build exists on the *write*
side, where the writer summarises every block as it commits, but the read side is polled
rather than pushed, which is a WP-16 question about what the view wants.

## Phase 1 - WP-10, playback

Built 2026-09-26. `vcw-audio::playback` owns the output stream, `vcw-project::pcm` reads
the PCM back, `vcw-core::playback` is the transport, and `vcw play` drives all of it from
a script. Exit criterion met on both halves: **a gapless seek**, proved byte-for-byte in
CI with no sound card and measured at **median 19.8 ms** on a real device, and **a
bit-perfect path reported honestly**, which on this machine reads `bit-perfect playback,
confirmed against the OS` and on a converting path says so instead.

**Milestone M2, *it plays back*, is met.** The whole chain runs headless and was run
end to end on hardware for the record:

```sh
vcw session m2.vcw --script "arm, record, sleep 20, stop, quit" --rate 48000 --format s32
vcw waveform m2.vcw --pixels 100 --rows 11
vcw play m2.vcw --capture 1 --device alsa:hw:CARD=PCH,DEV=1 \
    --script "play, sleep 2, seek 15, sleep 2, skip-back, sleep 2, stop"
```

Capture, progressive waveform, playback, seek. 960,480 frames recorded with no loss,
288,000 frames played with 0 gaps across a seek and a skip.

### The four targets of §21 are one span

§21 asks for playback of the complete capture, a selected region, an individual track and
a boundary audition. They differ only in which frames they cover, so `Scope` resolves all
four to a `Span` and everything below it plays a span and knows nothing else. A boundary
audition is [`BOUNDARY_CONTEXT_SECONDS`] of 3 s each side of a frame, clamped, which is
the only one of the four that needed a number invented; §21 does not give one.

The transport reduces the same way. Six verbs - `PLAY PAUSE STOP SEEK SKIP FORWARD
SKIP BACK` - and the last three are all `seek` with the arithmetic done first. `SKIP` is
[`SKIP_SECONDS`] of 10 s until WP-13 records boundaries, at which point the skips become
"next boundary" and "previous boundary", which is what they are for.

### Epoch-tagged chunks, not a byte ring

Capture's ring works because the producer is the callback. Playback inverts that, and a
ring inverts badly: the producer would be the feeder, which cannot clear a ring it does
not consume, so a seek would leave up to a second of the old position queued and the
listener would hear it. The queue is therefore chunks, each tagged with the epoch it was
filled in and the frame it starts at. A seek bumps the epoch; the callback discards every
chunk that does not match, unplayed, and recycles it.

Two things fall out of that for free. The position is **exact rather than inferred** -
the callback knows the frame number of the chunk in its hand, so nothing subtracts a
buffer depth it cannot see - and `drained` is per-epoch, so running out of audio at the
end of a span is the end of it while running out mid-span is an underrun, and the two
are never confused.

The chunks themselves are allocated once and circulate through two SPSC queues, full one
way and spent the other, exactly as capture's buffers do. `tests/rt_safety.rs` covers
`Source::on_data` under a counting allocator on four paths - the ordinary one, a seek, a
starved feeder and a stalled feeder - because the seek path is the one that tempts an
implementation into clearing a collection, and it runs *on the audio thread* by design.

### No resampler, and a capture plays at its own rate or not at all

Locked as [ADR-0006](adr/0006-playback-rate-policy.md). The rate is not a preference. A device that cannot do 192 kHz cannot play a 192 kHz
capture, and the answer is `Error::RateUnavailable` naming the rates the device does
offer, not a silently resampled side. The format is a preference, and the order is the
*opposite* of capture's: capture takes the best the device offers because a better
capture is strictly better, and playback takes the format that matches what is on disk
because anything else is a conversion.

`convert::natural` is what "matches" means: the device format a stored format would
rather be played in. Int24Padded maps to S24 rather than S32, because the bytes are the
same three bytes and the narrower stream is the one that can still be called
bit-perfect. It is lossless for every stored format, which is what lets the render path
be byte-exact.

### Two findings, both measured, both invisible from the code

**The queue depth is not a tuning knob, it is a correctness constraint.** The first live
run played at half speed with 16 underruns, while every counter except `underruns` said
the device was healthy. ALSA's own default buffer on this machine is **350 ms**; the
queue was `QUEUE_CHUNKS` of 20 ms, which is **160 ms**. A queue shallower than one
callback underruns on *every* callback and cannot be rescued by a faster feeder. So
playback now asks for a buffer it chose - [`TARGET_BUFFER_MILLIS`], four chunks, 80 ms -
and derives the queue from it; a backend that will not be told gets a
[`FALLBACK_QUEUE_MILLIS`] second-deep queue instead of an argument. On this device the
request is honoured: `buffer 3840 frames, fixed`.

**A gapless seek needs the feeder to be holding an empty chunk when the seek lands.**
With the buffer fixed and the queue sized, five seeks on a real device still cost exactly
five underruns and 34,560 frames of silence - one buffer per seek - while the render path
cost none. The render path tops the queue up synchronously before each callback; a real
feeder does not. When a seek lands, every chunk in the queue is stale *and* every empty
chunk is in the queue, so the feeder has nothing to fill and cannot take one back out of
an SPSC queue it is the producer of. The callback then discards the lot in one pass,
finds nothing behind them and plays a buffer of silence.

The fix is a reserve: `Feeder::hold_back` keeps one callback's worth of chunks out of
ordinary filling, and the feeder spends them the moment it sees the epoch change. The
new position is queued *behind* the audio the seek invalidated, so the callback walks
past the stale chunks and keeps reading in the same pass. The feeder's idle sleeps became
interruptible at the same time, because a 50 ms sleep is 50 ms of a 80 ms budget.

After both: **0 underruns** across five seeks, three consecutive runs reporting
identical counters.

### The seek join latency, measured

| | |
|---|---|
| min | 11.7 ms |
| median | 19.8 ms |
| max | 20.4 ms |

Five seeks per run, three runs, on `alsa:hw:CARD=PCH,DEV=1` at 48 kHz S32 with a
3,840-frame buffer. The median is one chunk, which is the granularity the design chose,
and the ceiling the test enforces is 500 ms.

The first version of that measurement reported 0.00 ms and was worthless, which is worth
recording because it is the shape of mistake a latency test invites. It polled
`Player::position` after `Player::seek`, and `seek` *stores* the frame it asked for -
so the poll was reading the write it had just made and nothing the device had done. The
playhead reading the target immediately is right for a UI and useless for a measurement,
so the callback now records `Cursor::delivered`, the epoch it last copied audio out of.
A seek has joined when `delivered` catches up with `epoch`, and that is a fact about the
device rather than about the caller.

### The render path is why gaplessness is testable without a device

`playback::render` drives the same `Pump`, the same epoch-tagged queue and the same
`Source::on_data`, synchronously, and writes the audio frames to a file. Silence is
reported in `health` and never written. So a gapless seek becomes a byte comparison that
runs in CI: play 0-2 s, seek to 4 s, and the output must equal `whole[..2 s] ++
whole[4 s..]` with nothing repeated and nothing missing.

Cues are reported rather than rounded away. A cue fires at the first period boundary at
or after the frame it names, so `Rendered::applied` carries `{ verb, after, landed }` and
the test computes its expectation from the join that actually happened. That turned a
flaky assertion into a documented granularity, and gave the CLI something true to print.

### Fidelity is three-way, like capture's verdict

`Fidelity` is Confirmed, Refuted or Unconfirmed, and "nothing rules it out" is never a
pass. One refutation is unique to this side: **the samples were converted for the
device.** A 24-bit side played on a 32-bit stream sounds identical and is not
bit-perfect, and it says so. A render is never verified against hardware, because there
is no hardware under it.

### The CLI

```sh
vcw play side.vcw --capture 1                       # the whole capture
vcw play side.vcw --start 65 --end 130               # a region
vcw play side.vcw --track 3                          # one track
vcw play side.vcw --boundary 65.4                    # 3 s either side of a boundary
vcw play side.vcw --render out.raw --start 0 --end 5  # no device needed
vcw play side.vcw --script "play, sleep 2, seek 15, skip-back, stop" --json
```

`--script` is a comma-separated list of the six verbs plus `sleep <seconds>`; it opens
paused, so a script that never says `play` plays nothing, deliberately. Three events go
on the bus - `auditioning`, `playback-position` and `playback-finished` - and §35 names
neither, so the names follow its kebab-case convention.

### Tests

411 in the workspace, full gate green, `cargo deny` clean. 15 on the transport, four
through the shipped binary including the byte-exact gapless seek, four on the real-time
contract of the playback callback, two on the chunk reserve, and two `#[ignore]`d
hardware tests that measured the numbers above.

### What is not verified

Linux x86_64 and ALSA only, like everything above it. The device half was run on this
machine's S/PDIF output, chosen because the simulated source is full-scale noise and the
digital output has nothing plugged into it; `VCW_TEST_OUTPUT` overrides it. WASAPI in
exclusive mode, CoreAudio and AAudio are all untried for output, and the buffer
negotiation is exactly where they are most likely to differ - the fallback path exists
for them and has never run.

Nothing has played a 192 kHz side yet, and nothing has played for an hour. The reserve
fixes the seek that lands between callbacks; a seek storm has not been tried.

## Phase 1 - WP-11, the detection port

Built 2026-09-26. VRipr's three detectors, ported into `vcw-signal`, published as §24
observations rather than tracks, resolved into decisions, and reachable from the command
line at both ends of §22: live while the record turns, and again over the committed
side. Exit criterion met on both halves. **Parity: 97.6% to 99.7% of VRipr's boundaries
reproduced over all 595 snippets of the labelled corpus, every agreement at the
identical frame**, with the residue traced to one documented cause. **Provenance and
confidence: every boundary carries both, plus the measurements behind them**, and
`vcw detect --evidence` prints the lot.

**Milestone M3, *it finds tracks*, is met.** On a real 26-minute side:

```sh
vcw detect side-a.vcw --min-sources 2
```

```
  analysis   15658 window(s), 3 detector(s), 12.839 s
  silence          6 boundary/ies at   -40.0 dB, floor -
  spectral-change  6 boundary/ies at   -40.0 dB, floor -
  hmm             266 boundary/ies at   -40.0 dB, floor -
  showing    6 of 270 boundary/ies, those 2 or more detectors reported
     1  start      0.000 s  conf 1.00  silence+spectral-change (2)
     2  end      282.800 s  conf 0.53  silence+spectral-change (2)
     3  start    283.800 s  conf 1.00  silence+spectral-change+hmm (3)
     4  end      686.000 s  conf 0.53  silence+spectral-change (2)
     5  start    687.200 s  conf 1.00  silence+spectral-change+hmm (3)
     6  end     1561.400 s  conf 0.59  silence+spectral-change (2)
```

Three tracks, 4:43, 6:42 and 14:34. The 266 the HMM found on its own are the subject of
one of the findings below.

### The split VRipr does not have

VRipr's detectors take a file path and decode it with Symphonia. VCW's take frames,
because §22 asks for live analysis while the record is still turning and there is no
file to open. So `features::Windows` is the only thing that touches audio: it turns
capture bytes into `Frame { rms, flatness }`, and `silence`, `spectral` and `hmm` see
nothing else. The same extractor serves both passes, which is what makes the live answer
and the refine answer comparable rather than merely similar.

`Windows` carries a frame that straddles two calls instead of dropping it. A meter can
drop three bytes and be wrong by nothing anyone can hear; an extractor cannot, because a
dropped orphan sample shifts the window alignment for the rest of the side and moves
every boundary after it. That was a real defect, found by a test that pushed the same
audio in ragged chunks and in one go and demanded the same frames
(`frames_do_not_depend_on_how_the_audio_arrives`).

### Positions are frames, and that is where VRipr and VCW part company

VRipr works in seconds as `f64`. VCW works in frames, and `Region::seconds` exists for
printing. The difference shows up in exactly one place and it is worth the paragraph:
`merge_gaps` bridges gaps *shorter than* a limit, and VRipr computes the gap from
`index as f64 * window_secs`, so a gap of exactly eight 100 ms windows comes out as
0.7999999999999993 and gets bridged. VCW computes 0.8 and leaves it.

That single boundary condition accounts for most of the corpus disagreement. Forcing it
the other way was tried: the level detector rises from 99.52% to 99.86% and the HMM
*falls* from 98.01% to 96.22%, because VRipr's own answer depends on how the error
happened to accumulate in each snippet. Neither comparison reproduces it; only
reproducing the accumulation would, and that means giving up the frame arithmetic that
makes the live pass and the refine pass agree. A gap of exactly `min_silence_secs` is a
boundary, which is what the setting says it is.

### The adaptive floor can only find the groove if there is enough groove

`adaptive_floor` is VRipr's interpolated 3rd-percentile estimate, and the percentile is
the whole story: the estimate lands in the inter-track groove only if the groove is
*more* than 3% of the side. A tightly cut side is under 1% groove, and then the third
percentile sits inside the quietest music. Two tests were written against the wrong
premise before this was measured - they asked for a floor near the groove on traces that
were exactly 3% groove, and got -30.2 dB and -23.3 dB instead. The limitation is now
documented on the function, and it is why `adaptive_margin_db` is as wide as 12 dB.

### Contrast is a median, because a padded boundary straddles the transition

Boundary confidence comes from the contrast between the music before a boundary and the
gap after it, over five windows each way. Taking the mean failed a 50 dB test case at
44.8 dB, and the reason is not noise: the padding puts a window or two of the *other*
side inside the look-back range, and a vinyl pop drags a mean up on its own. A max/min
would fix both and bias every score upward, which is the one direction a confidence must
not be wrong in. The median fixes both and biases nothing.

### Smoothing costs the flatness detector two or three windows of position

`spectral::smooth` is VRipr's ±3-window rolling mean, 700 ms at the default window, and
it moves a boundary by two or three windows - pinned at window 302 becoming 303 in
`smoothing_costs_the_boundary_a_few_windows_and_that_is_the_trade`. The same blur
corrupts the measurement of the thing being scored: reading flatness at the boundary
understates the gap-to-music separation by a factor of two, so the score stands back
`SKIP = SMOOTHING + 1` windows. And the order of the pipeline matters more than it looks:
a 200 ms tonal pop inside a 1.2 s flatness gap erases the gap through the rolling mean
entirely, so the transient filter runs *before* gap-fill, not after.

### The HMM posterior saturates, so it is a veto and never a score

This is the session's headline finding and it changed the design. Forward-backward
returns a posterior of **1.0 for every boundary Viterbi commits to** - across a 60 s
fade, and at only 3 dB of real contrast. It has to: the emissions are fitted to the
quietest 15% and the loudest 40% *of the data being classified*, and a Gaussian
log-likelihood is quadratic, so whatever the side contains, the two states end up
separated in the model that was built from them. A posterior used as a confidence would
report total certainty about a boundary that is not there.

So `confidence = min(level_confidence, posterior)`: the posterior can only ever veto.
And it has one exception, because a capture edge has no transition to measure - the first
boundary of a side that begins in music scored 0.0002, which is the correct posterior for
a state change that never happened and a useless confidence for a boundary that is
certainly real. Inside `EDGE_WINDOWS` of either end the veto is skipped, and
`hmm.posterior_applies` records which way it went, so the evidence says whether the veto
was in force rather than leaving a reader to infer it.

Three other HMM premises were disproved by probes before they became tests: with
indifferent emissions a chain drifts to *Music*, not to the biased start state, because
`min_sound > min_silence` makes music the stickier of the two; a featureless side comes
back as one Music region rather than none; and posteriors sum to 2.015 rather than 2.000
for two boundaries, because some paths cross more than twice.

### The HMM over-segments a real side, faithfully

266 boundaries on the side above, against six from each of the other two. It is not a
defect in the port - the parity harness puts the HMM at 98.01% of VRipr's own answers,
and VRipr's HMM does the same thing - it is what a self-fitted two-state model does to a
record with quiet passages in it: the quietest 15% of a real side *is* music, so the gap
state gets fitted to quiet music and then finds it everywhere.

This is the case the resolver exists for. Every one of those 264 unsupported boundaries
comes back at confidence 0.50 with `agreement() == 1`, and the six a second detector
seconded come back at 0.53 to 1.00. `vcw detect --min-sources 2` is the same filter at
the command line, and WP-13 should not promote a boundary no second detector saw.

### Agreement is counted, never multiplied

`resolve` clusters observations within half a second of each other, per edge, and takes
the **maximum** confidence in a cluster rather than a noisy-OR. Three detectors reading
one level series and each reporting 0.5 are not three independent witnesses; they are one
measurement counted three times, and a noisy-OR would turn it into 0.875. The position
goes in the safe direction - earliest start, latest end - so a padding error clips
silence rather than music. A boundary a person placed fixes the position of its cluster,
cannot be merged away, and two user boundaries close together stay two boundaries (§24).

### The live pass is the refine pass with less audio

Not an approximation of it. The live worker keeps the whole feature trace - 15,000
windows for a 25-minute side, about 200 KB - and re-runs the detector on every drain,
which costs well under a millisecond. So a provisional marker is the final answer
computed from the audio that has arrived so far, and
`the_live_pass_and_the_refine_pass_agree` proves the two produce identical regions and
boundaries to the frame when one is fed ragged 7,331-byte chunks and the other the lot.

A marker is published only once it cannot move, which is `min_silence + gap_fill +
pre + post` behind the analysed position: **1.2 s at the defaults**. A marker is never
retracted, and an adaptive live pass settles nothing at all - the threshold depends on
the whole side - which `Live::settled` reports honestly by returning 0.

The live pass is levels only: no FFT on a tap of the capture stream. The refine pass is
where the spectral extraction happens, once, with all three detectors reading its
frames. That buys a property worth having: a disagreement between two detectors on the
same side cannot be a disagreement about what they were looking at.

### Parity, measured

`crates/signal/tests/vripr_parity.rs`, `#[ignore]`d because it reads 294 MB from outside
the repo. `/data2/vripr_training` is 595 snippets VRipr cut from its own track tables -
16 s of mono 16 kHz audio centred on a boundary, peak-normalised, with a JSON sidecar
naming the kind. The reference is **VRipr's own detectors run over the same snippets**,
computed out of tree at `/data2/vcw-scratch/parity` from a verbatim copy of
`/data2/vripr/src/audio/mod.rs` and checked in as
`crates/signal/tests/fixtures/vripr_answers.jsonl`, so the reference outlives the other
repository.

| Detector | Fixed threshold | Adaptive | Snippets identical | Offset |
|---|---|---|---|---|
| `silence` | 99.52% | 97.63% | 98.66% / 95.29% | 0.000 s |
| `spectral-change` | 99.71% | 98.71% | 97.98% / 96.13% | 0.000 s |
| `hmm` | 98.01% | 98.01% | 93.95% | 0.000 s |

Every agreement is at the identical frame; the test asserts that, not a tolerance. The
HMM is the same under both thresholds because it never reads one - it fits its own. The
gate fails below 97% of VRipr's boundaries or 90% of snippets matching exactly.

### Agreement with the *labels* is low, and VRipr's is lower

The same harness scores both against what the sidecars say, and the numbers are
uncomfortable until you see the second column:

| Detector | VCW | VRipr | `mid` snippets left alone |
|---|---|---|---|
| `silence` | 17.1% | 16.3% | 99.5% |
| `spectral-change` | 11.3% | 10.8% | 99.5% |
| `hmm` | 30.9% | 30.7% | 82.7% |
| resolved, 2+ detectors | 12.3% | - | 99.5% |

VCW is a hair ahead of VRipr on every row, which is the only thing this comparison can
establish. The corpus is dominated by ambient and drone records whose tracks segue with
no silence at all, and the labels are the track table - which for those records came
from a release listing or from a person, not from any detector. The `.onnx` file sitting
in the corpus directory is the rest of the story: VRipr was training a learned detector
on this material precisely because its classical ones could not do it. §22 lists the
three that are required, and this is what they are worth on the hard cases; §22's
"expected track count, release durations, fingerprints, side topology" are the rest of
the answer, and they arrive with WP-12 and WP-13. The resolver already takes a boundary
from a release listing without a line of new code
(`a_boundary_from_a_release_listing_needs_no_new_code_to_be_heard`).

### Cost

A 26-minute side at 192 kHz: **12.8 s** for the refine pass in release, 15,658 windows,
one FFT each. That is 120x faster than the side plays, and it is dominated by the
extraction rather than the detectors. The live pass costs a 250 ms drain of a 2 s lossy
tap and a re-scan of at most 15,000 frames, and the capture it hangs off reports the
same zero dropped frames and zero overruns it did with only the meter attached
(`detection_costs_the_capture_nothing`).

### The CLI

`vcw detect <project>` runs the refine pass and prints the boundaries with their
provenance, confidence, agreement and - with `--evidence` - every measurement behind
them. `--threshold-db`, `--adaptive`, `--min-silence` and `--min-sound` reach the
detectors; `--min-sources` filters by agreement; `--json` gives a UI the same thing.
`vcw session --json` now renders `track-detected` as it happens, which is how the live
half is visible with nothing else running.

Nothing here writes. A boundary becomes a track in WP-13, and the tracks `vcw detect`
prints are labelled implied for that reason.

### Tests

496 in the workspace pass and 4 are `#[ignore]`d, the full gate is green, `cargo deny`
is clean and so is the doc leg. 77 of them are in `vcw-signal`'s lib covering the five
new modules, four are on the engine's detection path, three go through the shipped
binary, and one is the parity harness, which is `#[ignore]`d and was run.

### What is not verified

The parity figure is parity, not accuracy. Nothing here has been checked against a
boundary anyone confirmed by ear; the labelled corpus is VRipr's own reading of its own
records, and on the hardest third of it both implementations are mostly wrong together.

The live pass has only ever been fed the simulated source through the engine - uniform
noise, which has exactly one boundary. The live/refine agreement test uses real
synthesised material but drives `Live` directly rather than through a capture, so what
has never happened is a live pass over a real record with real gaps in it, on a real
device. That wants a turntable and a side, and it is the first thing to do with WP-11
when there is one to hand.

Adaptive mode has no live story worth the name, as above. And the HMM's over-segmentation
is reproduced rather than solved: the resolver makes it harmless, but §22's guided
detection - expected track count, release durations, side topology - is what would
actually fix it, and that is WP-12 and WP-13 work.

## Next up

**Where to pick up.** WP-11 is finished and gate-green but **not yet committed** - the
whole of it is in the working tree along with this file. Committing is the first thing
the next session does, and nothing about the work is half-done.

**`WP-12`, metadata, is next** and needs no device at all: a provider trait, Discogs,
MusicBrainz, genre normalisation, artwork, caching, rate limits, timeouts and
cancellation, at weight 9. Its exit criterion is that the application stays fully usable
with networking disabled, so the tests are fixture-backed and offline by construction,
and §39's rule that no credential is ever written into a project file is a constraint on
the design rather than a check at the end.

**`WP-13`, the vinyl data model and editing, is the alternative, and WP-11 just
unblocked it.** Detection publishes boundaries and deliberately writes no tracks, per
§23, so WP-13 is where a boundary becomes one - and where §24's locking, which `resolve`
already honours for any observation handed to it, finally has something to lock. It is
also what turns `SKIP FORWARD` and `SKIP BACK` from a fixed ten seconds into what §21
actually wants, the next boundary either way: the boundaries now exist, but nothing
stores them, so the transport has nothing to ask.

Two things WP-11 leaves on the table for whoever takes WP-13. The HMM's
over-segmentation is real and faithful, so **a boundary with `agreement() == 1` should
not become a track** without a person saying so; `vcw detect --min-sources 2` is the
same rule at the command line. And §22's guided detection - expected track count,
release durations, fingerprints, side topology - is the part of the requirement that is
*not* built, and it is what would actually fix the hard cases the corpus is full of. The
resolver takes such a boundary already; something has to produce one.

The plumbing is in place for whichever comes first. `Tee` takes any number of taps, the
meter uses one, and a playback monitor or a live waveform feed is a second `tap()` call
and a second thread, with no change to the capture path.

One thing WP-09 deliberately left undone and did not need: **the waveform is read, not
pushed.** The writer summarises every block as it commits, so the rows are there the
instant they land, but nothing publishes a waveform event and a UI would have to ask. It
is a WP-16 question about what the view wants, and cheap either way.

Still open on WP-03, WP-04 and now WP-10, and all for the same reason: **Windows and
macOS.** The device matrix is reported on one OS, the OS format verifier exists for
Linux/ALSA only, and playback's buffer negotiation has only ever met one backend. On the other two the verdict degrades to `Unconfirmed` rather than to a false
pass, which is the right failure, but neither work package can close on it.

WP-05 and WP-06 have the same shape of gap: the 90-minute soak and the kill suite have
both run on x86_64/ext4 and nowhere else. WP-09's two new indexes put the soak back in
scope on this machine, and **that re-run is done and passes**: commit max 102.6 ms
against the pre-index 102.3 ms, zero loss, every one of 6,220,938,240 bytes matched. The
x86_64/ext4 gap is closed for the new schema; the Pi 5 and Windows runs are what remain. The Pi 5 on SD and on NVMe, and Windows, are
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
  WP-04 at `95f1f52`, WP-05 at `358c44a`, WP-06 at `b2a517b` and WP-07 at `051a648`.
  WP-08 is committed at `75123ab`, and WP-09 at `ae9b6d8` and `807d097`. WP-09 is the
  first change since WP-02 to touch the schema, so `docs/SCHEMA.md` was regenerated with
  it; regenerate with `VCW_BLESS=1 cargo test -p vcw-project --test schema_doc` whenever
  the schema moves, or `the_committed_document_matches_the_schema` fails.
- **`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` is part of the gate.**
  It had never been run, and it found ten broken doc links across four crates that
  `clippy -D warnings` does not see: private items linked from public docs
  (`GAP_SHARE`, `sizing`), a link to a crate `vcw-signal` does not depend on, a file
  path written as an intra-doc link, a module and a function called `validate` needing
  `mod@` to disambiguate, and `crate::schema::markdown` for what is really
  `crate::doc::markdown`. It also found a real defect: the doc comment on
  `playback::choose` had been cut in half by a `const` inserted into the middle of it,
  so half the paragraph was documenting `TARGET_BUFFER_MILLIS` and `choose` began
  mid-sentence. Both halves are reunited. Run the doc leg with the others from now on.
- **`/data2/vcw-scratch/parity/`** is the A/B reference generator for WP-11: a scratch
  crate holding a verbatim copy of `/data2/vripr/src/audio/mod.rs`, run over
  `/data2/vripr_training` to produce `vripr-answers.jsonl`. Its output is checked in at
  `crates/signal/tests/fixtures/vripr_answers.jsonl`; the crate itself is not, so that
  nothing in the repository carries a second copy of VRipr's algorithm. Regenerate with
  `cargo run --release -- /data2/vripr_training vripr-answers.jsonl` from that directory
  if the reference ever needs rebuilding, and expect the parity floors in
  `tests/vripr_parity.rs` to need re-reading against the new figures.
- **`/data2/vcw-scratch/`** is the measurement bench for WP-09 and WP-10, none of it in
  the repository
  and all of it disposable. `realrip/` is a throwaway crate that pushes a headerless WAV
  through `persistence::Writer` at full tilt - there is no CLI path for that yet, and it
  is how the 26-minute side got into a project. `evict.py` is the two-line
  `posix_fadvise(DONTNEED)` cache-evictor every cold number was taken with; without it
  the readings are RAM readings. `side-a.vcw` is the 2.33 GiB real side, `soak90.log` the
  passing 90-minute run and `soak90-contended.log` the spoiled one. `play/` holds WP-10's
  six-second project and the `.raw` renders taken off it by hand; `m2/m2.vcw` is the
  twenty-second capture the M2 chain above was demonstrated on.
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
