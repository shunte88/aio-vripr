# VCW — The Vinyl Capture Workstation

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

Pre-0.1. Phase 0 — proving the foundations before building on them.

| Spike | Question | State |
|---|---|---|
| S1 | Does CPAL give us bit-perfect capture and playback per platform? | [Linux yes](docs/spikes/S1-cpal-capture.md), on stock CPAL 0.18; other platforms outstanding |
| S2 | Can SQLite absorb sustained 24/192 capture, and survive a kill? | [yes on x86_64/SSD](docs/spikes/S2-sqlite-capture.md), incl. a 90-min soak; other platforms outstanding |
| S3 | Will Tauri IPC carry the meter and waveform rates? | not started |
| S4 | Does `chromaprint-next` fingerprint from a stream? | not started |
| S5 | Is the Audacity project format readable? | [AUP3 yes](docs/spikes/S5-audacity-format.md), AUP4 unmeasured |

Full snapshot — what is proven, what is assumed, what is waiting on hardware or a
decision — in [docs/STATUS.md](docs/STATUS.md).

## Licence

MIT — see [LICENSE](LICENSE).
