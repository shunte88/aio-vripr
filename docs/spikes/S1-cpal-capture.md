# S1 — CPAL bit-perfect capture and playback

**Status:** Linux/x86_64 complete. Windows, Android and macOS outstanding.
**Date:** 2026-09-22 · **Requirement:** REQUIREMENTS §5, §8, §47 · **Plan:** PROJECT_PLAN §4 (S1), risk R2
**Artefact:** [`spikes/vinyl-audio-test`](../../spikes/vinyl-audio-test)

## Question

Can CPAL capture and play back bit-perfect audio on every target platform, with
device choice being — as the working assumption had it — "simply a matter of
selection from a device list"?

## Answer

**Yes on capture quality, no on the device list.** On Linux, CPAL reaches
genuine bit-perfect 24/192 capture, and the full §47 path works end to end. But
the device list CPAL publishes is not a description of the hardware, and taking
it at face value produces silent, undetectable resampling. That is a product
problem, not just an engineering one, and it is the main output of this spike.

Verified on this host (Linux 7.0.0-31, ALSA 1.2.15.3, CPAL 0.16.0, HDA Intel PCH
line input), **while the S2 90-minute soak was concurrently hammering the same
disk**:

```
negotiated        192000 Hz  2 ch  I32  (4 B/sample)
kernel hw_params  /proc/asound/card0/pcm0c/sub0/hw_params : S32_LE 192000 Hz 2 ch
elapsed 5.01 s · callbacks 112 · frames captured 960152 · frames dropped 0
blocks 21 · commit p50 4.0 ms / p99 24.3 ms · peak WAL 4.0 MiB
verify: integrity ok · checksum failures 0 · sequence gaps 0
PASS (bit-perfect: kernel confirms the negotiated format)
```

Playback returned all 960,152 frames in the stored format with no conversion.
Crash recovery, with a live device and `SIGKILL`:

```
cycle 1: killed at 7s, recovered 6.75s (lost 0.25s, budget 0.25s) integrity=ok checksums=0 gaps=0
cycle 2: killed at 7s, recovered 6.75s (lost 0.25s, budget 0.25s) integrity=ok checksums=0 gaps=0
cycle 3: killed at 7s, recovered 6.75s (lost 0.25s, budget 0.25s) integrity=ok checksums=0 gaps=0
```

Identical to S2's synthetic result, now with real audio: loss is exactly one
block, every time, as designed.

## Finding 1 — CPAL's device list describes the plug layer, not the hardware

`cpal-0.16.0/src/host/alsa/enumerate.rs` opens every sound card through
`plughw:`, not `hw:`, under a hardcoded `const USE_PLUGHW: bool = true`. The
upstream comment is candid about it:

> Using plughw adds the ALSA plug layer, which can do sample type conversion,
> sample rate convertion, ... It is convenient, but at the same time not
> suitable for pro-audio as it hides the actual device capabilities and perform
> audio manipulation under your feet

There is no public API to open a device by PCM id, so an application cannot ask
for `hw:` through CPAL. Three consequences, all observed:

1. **Advertised configs are fiction.** Every device reports 128–256 supported
   configurations and defaults to 44100 Hz F32, because that is what the plug
   layer will accept — not what the converter does.
2. **Some advertised rates fail at stream build.** Requesting the advertised
   maximum on the HDA card fails with
   `snd_pcm_hw_params_set_rate ... Invalid argument (22)`. The list offered a
   rate the hardware rejects.
3. **Conversion is invisible at the CPAL layer.** This is the serious one.

### The failure mode, caught on the first run

Capturing from the default (PipeWire) device, CPAL reported the request
*honoured exactly*: 48000 Hz, 2 channels, I32. The kernel disagreed:

```
negotiated        48000 Hz  2 ch  I32
/proc/asound/card2/pcm0c/sub0/hw_params : S16_LE 8000 Hz 1 ch
```

The hardware was running **8 kHz mono 16-bit**. PipeWire upsampled it to 48 kHz
stereo 32-bit, and CPAL reported a clean success. Nothing in the CPAL API
indicates anything happened. We would have stored four seconds of "48 kHz
24-bit stereo" that was upsampled telephone audio, checksummed it, and called it
bit-perfect.

**This is exactly the defect §8 exists to prevent, and no amount of care at the
CPAL layer detects it.**

### What we do about it

`vinyl-audio-test` cross-checks every capture against
`/proc/asound/card*/pcm*c/sub*/hw_params`, which reports what ALSA actually
negotiated with the hardware, and refuses to call a capture bit-perfect unless
the kernel agrees. That check is cheap, it is the only source of truth on Linux,
and **it must survive into the product** (WP-04), not stay in the spike.

For the device list itself there are three routes, and this needs a decision:

| | approach | cost | keeps CPAL? |
|---|---|---|---|
| A | Patch `USE_PLUGHW` to false in a vendored CPAL | trivial; loses devices that genuinely need the plug layer | yes |
| B | Upstream an API to open a device by PCM id, carry a patch until it lands | small, and the right long-term fix | yes |
| C | Use the `alsa` crate directly for Linux capture, CPAL elsewhere | a second backend to maintain, but full control | partly |

**Recommendation: B, with A as the interim.** The kernel cross-check makes
either safe, because a wrong choice becomes a visible failure rather than a
silent one. Equivalent verification is needed per platform — WASAPI exclusive
mode reports its negotiated format directly, and that path is untested here.

## Finding 2 — a CPAL bug that returns zero frames, and its one-line fix

On this host every ALSA capture produced **zero callbacks**, with the error
callback firing continuously:

```
A backend-specific error has occurred: get_htstamp `0.0` was earlier than
get_trigger_htstamp `110272.984807580`
```

Cause: CPAL probes whether the driver supplies usable timestamps *before*
starting the stream. Here `get_htstamp()` returns non-zero before start and
`0.0` once running, so the heuristic picks the htstamp path, and then every
callback fails its sanity check and delivers nothing. Format negotiation had
already succeeded perfectly — the stream was correctly configured and simply
produced no audio.

Moving the probe after `handle.start()` and also falling back when the timestamp
delta is negative fixes it completely; that is the difference between the zero-frame
runs and the clean 960,152-frame capture above. The patch lives in
`spikes/vendor/cpal` with a `[patch.crates-io]` entry in the workspace root.

**This should go upstream.** It is a small, well-understood fix, and carrying a
vendored CPAL indefinitely is a liability — it is also a second reason to want
route B above, since both changes touch the same backend.

## Finding 3 — the default input is the wrong default

CPAL's default input on this host is the PipeWire path at 44100 Hz F32. For a
vinyl capture application that default is actively harmful: it is the one
configuration guaranteed not to be bit-perfect. The product must default to the
widest *integer* format the hardware genuinely supports and make the trade
visible where it cannot.

`vinyl-audio-test` already biases this way — given no explicit request it ranks
I32 > I24 > I16 > F32 rather than taking CPAL's default — but the ranking is a
policy decision that belongs in WP-04 with the user able to override it.

## What this means for the plan

R2 was downgraded to Low likelihood on the working assumption that CPAL handles
bit-perfect capture across platforms. **That assumption holds for the audio path
and fails for device discovery.** The correction is narrow but real:

- Capture quality: CPAL delivers. Bytes arrive unconverted via
  `build_input_stream_raw` and reach SQLite untouched.
- Device selection: **not** simply picking from CPAL's list. The list is the
  plug layer's, and a per-platform verification step is mandatory.
- R2 should read: *"CPAL's device abstraction hides the hardware's real
  capabilities; bit-perfection must be verified against the OS, not assumed
  from the API."* Likelihood Low→**Medium**, impact unchanged, mitigation now
  evidenced rather than hoped for.

Nothing here changes the architecture. It adds one component — a per-platform
format verifier — and one decision (A/B/C above).

## Coverage of §47

| § | behaviour | state |
|---|---|---|
| 1 | list devices | done — `devices` |
| 2 | list supported formats | done — `devices -v`, `formats` (with the caveat in Finding 1) |
| 3 | open the requested stream | done — `capture` |
| 4 | create a SQLite project | done — via `capture-core`, shared with S2 |
| 5 | capture PCM into SQLite blocks | done — 24/192, zero drops |
| 6 | live peak/RMS | done — computed in the RT callback, published by atomics |
| 7 | capture diagnostics | done — full report, JSON or human |
| 8 | play captured audio from SQLite | done — no conversion, all frames |
| 9 | verify block checksums | done — `verify`, automatic after capture |
| 10 | simulate interruption and recovery | done — `crash-test`, SIGKILL, 3/3 exact |
| 11 | requested vs negotiated format | done — **and cross-checked against the kernel** |
| 12 | overruns, underruns, dropped frames | done |
| — | run on Linux, Windows, macOS | **Linux only.** See below. |

## Outstanding

- **Windows.** WASAPI exclusive mode is the bit-perfect path and is untested.
  The kernel cross-check has no direct equivalent; WASAPI reports the negotiated
  format itself, which needs its own verifier.
- **Android/AAudio** and **macOS/CoreAudio** — macOS has no hardware here at all
  (see [[test-hardware]]), so it stays unverified until that changes.
- **A real converter.** Everything above used the motherboard's line input. The
  HiFiBerry DAC+ADC Pro on the Pi 5 and the Tascam DA-3000 are the devices that
  matter, and neither has been through this yet.
- **`snd-aloop` bit-exactness.** Loading the loopback module needs root and has
  not been done. It is the only way to prove capture output equals playback
  input sample-for-sample rather than merely proving the format was not converted.
- **Exclusive/hog modes** per platform — the capture-mode matrix G0 asks for is
  not yet published.

## Running it

```sh
cargo run -p vinyl-audio-test -- devices --verbose
cargo run -p vinyl-audio-test -- capture --device "HDA Intel PCH" \
    --rate 192000 --channels 2 --format i32 --db ./capture.vripr --duration 30
cargo run -p vinyl-audio-test -- play --db ./capture.vripr
cargo run -p vinyl-audio-test -- verify ./capture.vripr
cargo run -p vinyl-audio-test -- crash-test --device "HDA Intel PCH" \
    --rate 192000 --channels 2 --format i32 --kill-after 7 --cycles 3
```

Add `--json` to any of them for machine-readable output.
