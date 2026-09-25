# VCW - project status

**As of:** 2026-09-25
**Phase:** 1 has started - WP-01 is built. All five Phase 0 spikes returned verdicts on
their primary platform; gate G0 remains open on hardware coverage and on D3's
firmed-config soak, neither of which blocks foundation work.
**Branch:** `main` at `19dd459` plus the WP-01 scaffold in the working tree.

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

One honest gap: the soak ran the harness defaults (`synchronous=NORMAL`, interleaved),
not the D3-firmed `FULL` + per-channel. The short matrix says the firmed config should be
no worse, but *should be* is not *measured* - **D3 should not close until the firmed
config is soaked.**

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

## Next up

**`WP-02`, the `.vcw` schema.** The largest single lever in Phase 1: schema v1 as a
deliberate AUP4 superset, `sampleblocks` column-for-column, transactional migrations,
create/open/validate, integrity check. Everything it needs is in place - D1 locked and
validated against real AUP4 bytes, D2 locked, S2's block parameters measured, and a
30-project corpus to diff the `sampleblocks` shape against in CI.

Three measurement jobs stay queued and can run on the machine's own time: D3's
firmed-config soak (**D3 does not close until it runs**), S3's `cpu-matrix.sh`, and
S3's two R8 isolation soaks.

## Housekeeping

- All of Phase 0 is committed: the spikes, the CPAL 0.18 upgrade and the `.vcw` rename
  at `a28fd85`, the S3 IPC bench at `096a8a0`, and S4, S5 and the AUP4 delta at
  `19dd459`. The WP-01 scaffold is the current working-tree change.
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
