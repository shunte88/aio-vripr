# sqlite-capture-bench (spike S2)

Answers REQUIREMENTS.md §48: can SQLite absorb sustained 24/192 stereo capture while
analysis reads the same database, and what survives a crash?

Findings live in [`docs/spikes/S2-sqlite-capture.md`](../../docs/spikes/S2-sqlite-capture.md).

## Design

No audio hardware is involved. A synthetic source emits frames on a real-time deadline
into a bounded lock-free ring; if the ring is full it drops the chunk and counts it,
because capture must never wait for the database (§10). A writer thread drains the ring,
assembles immutable blocks and commits them in batched transactions. Reader threads
imitate the waveform and fingerprint workers against the same file.

The payload is a deterministic function of (frame, channel), so `verify` can prove the
stored bytes are the *right* bytes - not merely that a checksum matches.

The schema is the shape decided in D1: `sampleblocks` mirroring Audacity's AUP4 table
column for column, plus `capture_blocks` for the provenance AUP4 cannot hold.

## Commands

```sh
run         # one capture, human or --json report, exits non-zero on FAIL
sweep       # the §48 parameter matrix, one JSON object per combination
verify      # integrity_check + checksums + pattern + sequence gaps
crash-test  # SIGKILL mid-capture, N cycles, verify what survived
```

## Caveat that cost us once

**Do not benchmark against a tmpfs.** `/tmp` is RAM on many systems, which flattered the
first round of results by 3–6× at the tail. Point `--db` at the storage the real
application will use.
