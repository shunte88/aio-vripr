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

Capture has been confirmed bit-perfect end to end on this machine: 96 kHz / 2 ch / S32
requested and granted in exclusive mode over a direct hardware path, cross-checked
against what the kernel says the card is actually running. That cross-check is the
point - the audio API's report of its own success is not evidence, and on the same card
through a converting path the claim is correctly refused.

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

Add `--project take1.vcw` to record the session and its diagnostics counters into a
project file. The samples themselves are drained and discarded for now: the writer that
commits them is WP-05.

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
