# VCW - The Vinyl Capture Workstation

Capture, identify, edit, catalogue and export vinyl recordings. Bit-perfect from the
converter to the file, in one self-contained application.

VCW is the successor to [VRipr](https://github.com/shunte88/vripr), which analysed rips
Audacity had already made. VCW does the recording itself: the capture path, the project
store and the export are all its own, and Audacity becomes something it reads rather than
something it depends on.

- **Requirements:** [REQUIREMENTS.md](REQUIREMENTS.md)
- **Delivery plan:** [PROJECT_PLAN.md](PROJECT_PLAN.md)
- **Project format:** [docs/SCHEMA.md](docs/SCHEMA.md) (generated from the schema)
- **Spike findings:** [docs/spikes/](docs/spikes/)

## Status

Pre-0.1. Phase 0 - proving the foundations before building on them.

| Spike | Question | State |
|---|---|---|
| S1 | Does CPAL give us bit-perfect capture and playback per platform? | [Linux yes](docs/spikes/S1-cpal-capture.md), on stock CPAL 0.18; other platforms outstanding |
| S2 | Can SQLite absorb sustained 24/192 capture, and survive a kill? | [yes on x86_64/SSD](docs/spikes/S2-sqlite-capture.md), incl. a 90-min soak; other platforms outstanding |
| S3 | Will Tauri IPC carry the meter and waveform rates? | [yes, with 12.5× headroom](docs/spikes/S3-tauri-ipc.md), incl. a 30-min soak - the constraint is main-thread *rendering*, not IPC |
| S4 | Does `chromaprint-next` fingerprint from a stream? | [yes, bit-identically](docs/spikes/S4-chromaprint-streaming.md); region boundaries need no accuracy |
| S5 | Is the Audacity project format readable? | [yes, AUP3 *and* AUP4](docs/spikes/S5-audacity-format.md) - 30/30 corpus projects parsed to the last byte; the AUP3->AUP4 audio layer is byte-identical across five matched pairs |

Full snapshot - what is proven, what is assumed, what is waiting on hardware or a
decision - in [docs/STATUS.md](docs/STATUS.md).

Phase 1 has started. The workspace scaffold (WP-01) is in place: ten `vcw-*` crates
under `crates/`, a four-target CI matrix, and the licence and toolchain gates. Schema v1
(WP-02) is built and [documented](docs/SCHEMA.md); device enumeration (WP-03) and
capture (WP-04) are built on Linux x86_64, with Windows and macOS still unverified.
The persistence writer (WP-05) is built, so a capture now lands in the project as
audio rather than only as a row - proven by a 90-minute 24/192 soak on ext4 at a
real-time factor of 1.00001, with zero loss, a 4.81 MiB write-ahead log, and all
6,220,949,760 sample bytes read back and matched against what the source must have
produced for that frame and channel.

Recovery (WP-06) is built, which closes the first milestone: **it records.** A capture
process killed outright leaves a project that `vcw recover` finishes the way the writer
would have - the blocks decide the length rather than the row that never got updated,
the end time comes from the last block that was actually committed rather than from the
clock, and audio stranded past a gap is refused rather than quietly dropped. The proof
is a suite that spawns a real capture, `SIGKILL`s it at a random point, recovers it,
and then recomputes every stored sample from the frame index in its own block: 94 kills
this session, every one recovered exactly, none of them losing more than the single
250 ms block that had not been committed yet.

The engine (WP-07) is built, and with it the transport: §11's state machine is a
*typestate*, so an invalid transition is not rejected at runtime but has no method to
call - `Idle` cannot stop, `Stopped` cannot record, and a `Paused` that has been
resumed no longer exists. What drives it is a command in and an event out, nothing
else, which is what lets `vcw session side-a.vcw --script "arm,record,sleep 30,stop"`
record a side with no UI compiled at all. That is the architectural rule in §2 being
tested rather than asserted: a core that could only be driven from the interface would
have leaked into it.

The meters (WP-08) read through that same transport. Peak, RMS, a peak-hold needle and
a clip latch, per channel, at 50 Hz, live from the moment the transport is armed rather
than from the moment it records - because the workflow sets the level before the needle
goes down and there is nothing to set it against otherwise:

```sh
vcw session side-a.vcw --script "arm,sleep 10,record,sleep 1200,stop" --meters
```

Two things about that are worth stating, because both were decisions. Full scale is
**asymmetric**: the largest positive 16-bit code decodes to 0.99997 and the most negative
to exactly -1.0, so a clip detector comparing `abs() >= 1.0` never fires on integer audio
at all, and VCW tests each end against its own limit. And the fan-out that feeds the
meter is **lossy on purpose** - a meter worker that falls behind drops what it cannot
hold and counts it, because a stalled consumer must never cost a recorded frame. The
capture path is unchanged by it: the audio callback still does three atomics and one
memcpy.

The waveform (WP-09) is the first thing to read a capture back rather than write one.
The pyramid is built as the audio lands - the writer summarises every 250 ms block as it
commits it - and the reader picks its own rung from the span and the width, so the
picture is always exactly as wide as it was asked for and never costs more than the zoom
implies.

Getting there found something in the storage layer that no amount of care in the renderer
would have fixed. A sample block at 24/192 is 192 KB of audio, so with a 64 KiB page it
occupies pages of its own, and reading the twelve bytes of summary beside it still costs
a page fault. A 26-minute side is 12,528 blocks: **784 MiB of reads to obtain 150 KB of
triplets, and 3.77 seconds** for a drawing that has to feel instant. Two covering indexes
put the coarse rungs somewhere the audio is not, for 1.4% of the file, and the same
drawing is **17 ms**. The query names them explicitly, because SQLite left to itself
prefers the primary key and produces the slow plan, and a test asserts on the *query
plan* rather than on a stopwatch - the difference it guards is two hundred fold, and a
timing test for it would still be flaky.

Both indexes are maintained on every commit, so the writer was re-measured rather than
assumed: a 90-minute real-time 24/192 soak with the new schema gives a worst commit of
**102.6 ms against the 102.3 ms** the schema without them gave, over 21,601 commits,
with zero loss and all 6,220,938,240 bytes matched. The visible cost is 1.4% of the file
and 15% more write-ahead log.

The numbers are from a real record, not a generated signal: a 26-minute 192 kHz stereo
side pushed through the writer onto ext4, 300,627,479 frames in 2.33 GiB, with the page
cache evicted before every read. Whole side at 4000 columns 17 ms; an eight-minute span
at 1920 columns 74 ms; ten seconds 5.7 ms; the individual samples 6 ms. Worst case
anywhere in the sweep 304 ms.

Capture has been confirmed bit-perfect end to end on this machine: 96 kHz / 2 ch / S32
requested and granted in exclusive mode over a direct hardware path, cross-checked
against what the kernel says the card is actually running. That cross-check is the
point - the audio API's report of its own success is not evidence, and on the same card
through a converting path the claim is correctly refused.

Playback (WP-10) closes the second milestone: **it plays back.** Capture, waveform,
playback and seek all run headless:

```sh
vcw play side-a.vcw --capture 1                          # the whole capture
vcw play side-a.vcw --start 65 --end 130                  # a region
vcw play side-a.vcw --track 3                             # one track
vcw play side-a.vcw --boundary 65.4                       # 3 s either side of a boundary
vcw play side-a.vcw --render out.raw --start 0 --end 5     # no device needed
vcw play side-a.vcw --script "play, sleep 2, seek 15, skip-back, stop"
```

There is **no resampler**, and that is a feature
([ADR-0006](docs/adr/0006-playback-rate-policy.md)). A capture plays at its own rate or not
at all, and a device that cannot do 192 kHz is told so by name rather than handed a
silently converted side. The format preference is the opposite of capture's: capture
takes the best the device offers because a better capture is strictly better, while
playback takes whatever matches the bytes on disk, because anything else is a conversion
and a converted path is not bit-perfect however good it sounds. It says which it was.

Seeking is the part that had to be designed rather than written. The queue between the
reader and the audio callback is not a byte ring but **chunks tagged with an epoch**: a
seek bumps the epoch and the callback discards everything that no longer matches,
unplayed, instead of the listener hearing out the second of old audio a ring would still
be holding. Two useful things fall out of it for nothing - the reported position is the
frame the callback has in its hand rather than a guess with the buffer subtracted, and
running out of audio at the end of a side is distinguishable from running out in the
middle of one, which is the difference between "finished" and "your machine is too busy".

It is measured both ways. Byte-for-byte in CI with no sound card, because `--render`
drives the same queue and the same callback synchronously, so playing two seconds and
then seeking to four must produce exactly the first two seconds followed by everything
from the fourth: nothing repeated, nothing missing. And on a real device, where a seek
reaches the converter in a **median 19.8 ms**, one chunk. Getting there cost two findings
that only a device could have produced: a queue shallower than one hardware buffer
underruns on every single callback while every counter except that one reports health,
and a seek needs the reader to be holding an empty buffer at the moment it lands or it
costs a buffer of silence however fast everything else is.

## Layout

```
crates/          the product - one crate per REQUIREMENTS §6 group, §6's leaves
                 as modules. See docs/adr/0003-workspace-layout.md
spikes/          Phase 0 evidence, a separate workspace, excluded from the product
docs/SCHEMA.md   the .vcw schema, generated - do not edit by hand
docs/adr/        architecture decision records
docs/spikes/     the S1-S5 write-ups
```

Every source file opens with a header block: the file name, the product line, the
copyright, a one-line statement of what the file is for, and the MIT text. It is in
whatever comment syntax the language uses - `/* */` for Rust, TypeScript and HTML, `#`
for shell and Python, below the shebang where there is one. The purpose line repeats
the file's own first doc line (`//!` in Rust), which is where the real explanation
lives.

## Building

```sh
cargo build --workspace     # the product
cargo test --workspace
cargo run -p vcw-cli -- doctor
```

`vcw` is the headless driver - §4.5 requires the whole workflow to be drivable without a
UI. Today it can answer what this machine will record, and record from it:

```sh
cargo run -p vcw-cli -- devices --which input --hardware
cargo run -p vcw-cli -- formats "hw:CARD=0,DEV=0" --which input --confirm
cargo run -p vcw-cli -- capture "hw:CARD=0,DEV=0" --rate 96000 --seconds 10
```

`devices` lists what the host advertises; `--hardware` keeps only the direct paths that
could be bit-perfect. `formats` shows the §8 configurations one device offers, and
`--confirm` opens it once per configuration to find out which of them are real - an
advertisement is not a promise, and on an ALSA plug device most of them are not.

`capture` opens the device, reports what was asked for beside what was granted, reads
the format back from the operating system, and says whether the result can honestly be
called bit-perfect. It refuses the claim rather than guessing:

```
  negotiated  96000 Hz, 2 ch, S32, exclusive, direct hardware, buffer backend default
  os says     confirmed by /proc/asound/card0/pcm0c/sub0/hw_params: S32_LE 96000 Hz 2 ch
  counters    0 overruns, 0 underruns, 0 dropped frames, 0 stream errors
  verdict     bit-perfect, confirmed against the OS
```

Add `--project take1.vcw` to write the audio, the session and its diagnostics counters
into a project file. Without `--project` the samples are drained and discarded, because
there is nowhere to put them.

`soak` runs the same writer from a generated source for as long as you like, then reads
every byte back and checks it against the value the source must have produced for that
frame and channel:

```sh
vcw soak side-a-soak.vcw --rate 192000 --format s24 --minutes 90
```

That is how the storage path is measured on a machine before it is trusted with a
record. It reports commit latency percentiles against the block budget, the peak
write-ahead log, and whether anything was lost - and exits non-zero if it was.

`waveform` draws a capture at the terminal, which is how the pyramid is checked without
a UI:

```sh
vcw waveform side-a.vcw --pixels 160 --rows 21
vcw waveform side-a.vcw --start 300 --end 400 --pixels 1920 --json
```

It says which rung it read and how long the read took, so the claim above is verifiable
on any machine:

```
  capture    1, 192000 Hz, 2 ch, 1565.768 s
  span       0.000 s to 1565.768 s, 160 column(s) of 1878921.7 frame(s)
  level      block (48000 frame(s) a triplet), read in 17.600 ms
  channel 0  peak 0.9332
```

Add `--rebuild` to recompute the summaries from the stored audio first. It writes only
summaries, never samples, and only for blocks VCW recorded - an imported Audacity
project cannot be rewritten by a redraw.

Requires a Rust toolchain at 1.90 or newer and, on Linux, `libasound2-dev`. SQLite is
compiled in, so there is no system SQLite to match.

The spikes build separately:

```sh
cd spikes && cargo build --workspace
```

## Licence

MIT - see [LICENSE](LICENSE). Third-party obligations, including the LGPL relink
instructions that arrive with Phase 2 fingerprinting, are recorded in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md); `cargo deny` enforces the licence
allowlist in CI.
