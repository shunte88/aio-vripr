# VCW - The Vinyl Capture Workstation

Capture, identify, edit, catalogue and export vinyl recordings. Bit-perfect from the
converter to the file, in one self-contained application.

VCW is the successor to [VRipr](https://github.com/shunte88/vripr), which analysed rips
Audacity had already made. VCW does the recording itself: the capture path, the project
store and the export are all its own, and Audacity becomes something it reads rather than
something it depends on.

- **Requirements:** [REQUIREMENTS.md](REQUIREMENTS.md)
- **Delivery plan:** [PROJECT_PLAN.md](PROJECT_PLAN.md)
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
under `crates/`, a four-target CI matrix, and the licence and toolchain gates.

## Layout

```
crates/          the product - one crate per REQUIREMENTS §6 group, §6's leaves
                 as modules. See docs/adr/0003-workspace-layout.md
spikes/          Phase 0 evidence, a separate workspace, excluded from the product
docs/adr/        architecture decision records
docs/spikes/     the S1-S5 write-ups
```

## Building

```sh
cargo build --workspace     # the product
cargo test --workspace
cargo run -p vcw-cli -- doctor
```

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
