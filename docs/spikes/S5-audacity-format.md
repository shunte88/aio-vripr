# S5 — Audacity project format probe

**Status:** complete for AUP3. AUP4 unverified — see [Open questions](#open-questions).
**Date:** 2026-09-22 · **Requirement:** REQUIREMENTS §12, §13 · **Plan:** PROJECT_PLAN §4 (S5), decision D1
**Artefact:** [`spikes/aup-format-probe/probe.py`](../../spikes/aup-format-probe/probe.py)

## Question

D1 commits us to a native `.vripr` that is an AUP4 *superset*, with Audacity
**import only** — we never write a file Audacity has to read. The spike had to
establish whether that import is actually buildable, because the project
document is an undocumented binary blob, not XML on disk.

## Method

The corpus is 25 real vinyl rips from `/data2/vinyl_rips`, 10 MB–774 MB,
produced by Audacity 3.7.x over the life of the existing workflow — not
synthetic fixtures. The decoder was derived from the file bytes alone; no
Audacity source was consulted, so nothing here inherits Audacity's GPL.

The acceptance test is deliberately unforgiving: **every byte of every document
must be consumed.** A tag-length grammar that is even slightly wrong
desynchronises within a few records and throws. Partial credit is not
available, so 25/25 clean parses is strong evidence the grammar is right rather
than merely plausible.

## Result

```
25 parsed, 0 failed
elements: project 25, effects 50, tags 25, tag 50, wavetrack 50,
          waveclip 86, sequence 86, waveblock 30774, envelope 86,
          labeltrack 24, label 213
dangling block references: 0        orphan sample blocks: 0
```

Documents reconstruct to readable XML, and every `waveblock/@blockid` in every
project resolves to a row in `sampleblocks` with nothing left over.

**The import is tractable.** D1 stands; no re-plan needed.

## The format

A `.aup3` is an SQLite database, `application_id` `0x41554459` (`"AUDY"` — note
it is *not* `AUD3`, so the magic does not encode the version), `user_version`
`0x03070000` = 3.7.0.0. Page size is 65536 on recent files and 4096 on older
ones; both occur in the corpus, so a reader must not assume either.

Tables: `project`, `autosave`, `sampleblocks`, `sqlite_sequence`. `autosave` is
empty in a cleanly closed project and carries the same blob shape for a session
that was not saved — that is where a crashed Audacity's work lives.

### Document encoding

`project.dict` maps small integer ids to element and attribute names; `project.doc`
is a flat record stream referencing them, so no name is ever repeated. All strings
are UTF-32LE and **all lengths are in bytes, not characters**.

```
dict := 00 04                                  two-byte prologue, invariant
        ( 0F id:u16 nbytes:u16 utf32le[nbytes] )*

doc  := record*
   01 id:u16                                   start element
   02 id:u16                                   end element
   03 id:u16 nbytes:u32 utf32le[nbytes]        attribute, string
   04 id:u16 value:i32                         attribute, 32-bit signed
   05 id:u16 value:u8                          attribute, byte / bool
   06 id:u16 value:u32                         attribute, 32-bit
   07 id:u16 value:u64                         attribute, 64-bit
   08 id:u16 value:u32                         attribute, 32-bit
   0A id:u16 value:f64 digits:i32              attribute, double + precision hint
   0C nbytes:u32 utf32le[nbytes]               character data
```

`0x06` and `0x08` carry identical payloads and are used interchangeably for the
same attribute — `sampleformat` appears under both within the corpus. They are
presumably distinct C++ overloads upstream; a reader can treat them alike.
The `digits` field on `0x0A` is a formatting hint; `0xFFFFFFFF` means default.

Tags `0x00`, `0x09`, `0x0B`, `0x0D`, `0x0E` never occur. **The importer must
reject them, not guess a width** — a wrong guess silently desynchronises the
stream and yields plausible garbage rather than an error.

### Audio storage

```sql
CREATE TABLE sampleblocks(
  blockid INTEGER PRIMARY KEY AUTOINCREMENT, sampleformat INTEGER,
  summin REAL, summax REAL, sumrms REAL,
  summary256 BLOB, summary64k BLOB, samples BLOB);
```

Blocks are **mono**: one channel, `maxsamples` = 262144 samples = 1 MiB at 4
bytes per sample. A stereo track's two channels interleave their block ids
(1,3,5,… and 2,4,6,…) in a single autoincrement sequence. This vindicates the
per-channel block layout S2 measured — the two decisions agree.

Summaries are `(min, max, rms)` f32 triplets: `summary256` covers 256 samples
per triplet (12288 bytes for a full block), `summary64k` covers 65536 (48
bytes). Overhead measured on a 774 MB project: 8 MiB of summaries against 708
MiB of samples, **1.1%**.

`sampleformat` is `(bytes_per_sample << 16) | type_code`:

| value | | meaning |
|---|---|---|
| `0x00020001` | 131073 | int16 |
| `0x00040001` | 262145 | int24, stored in 4 bytes |
| `0x0004000F` | 262159 | float32 |

**There is no 32-bit integer format.** REQUIREMENTS §8 asks for one, so this is
a genuine conflict and the reason D1 chose an AUP4 *superset* rather than
literal AUP4. We add a format code in unused numeric space; Audacity never sees
our files, so nothing breaks.

### Stereo

Stereo is **two sibling `wavetrack` elements**, the left carrying
`channel="0" linked="3"` and the right `channel="1" linked="0"`. It is not one
element with two channels. A clip-split project carries many `waveclip` /
`sequence` pairs per track — one corpus file has 19 clips per channel.

## Traps the importer must handle

1. **`wavetrack/@rate` lies.** 22 of the 25 projects declare `rate="48000.0"`
   on the track while `project/@rate` correctly says `192000.0` and the audio
   genuinely is 192 kHz. The three exceptions agree with the project. An
   importer that trusts the track attribute plays every affected rip at a
   quarter speed, and would do so *silently*. **Authoritative source is
   `project/@rate`**; treat `wavetrack/@rate` as advisory and log a warning
   when they disagree.
2. **Two tags for one type.** Accept `0x06` and `0x08` identically.
3. **Byte lengths, not character counts.** A UTF-32 length read as characters
   under-reads by 4× and desynchronises.
4. **Page size varies** — 4096 and 65536 both present.
5. **`autosave` may be populated** on a project Audacity did not close cleanly.
   Importing `project` alone silently discards that session's work; detect the
   case and tell the user rather than quietly losing it.

## Incidental finding, and it matters

**24 of 25 existing rips are stored as float32, not integer.** Only
`simples_test` is int24. The current Audacity workflow has been converting the
ADC's integer sample words to float and storing those — every one of these rips
is one lossy conversion away from what the converter actually produced.

That is a direct, evidenced argument for REQUIREMENTS §8's bit-perfect capture
path, and it reframes the existing library: these files are good masters but
they are not bit-perfect captures, and re-ripping is the only way to make them
so. Worth deciding deliberately rather than discovering later.

## Open questions

- **AUP4 is unverified.** Audacity 4.x is not installed here and the corpus is
  entirely 3.7.x. Everything above describes AUP3. The AUP4 delta — new
  `application_id`, `user_version`, added tables, new tag numbers — is still
  unknown, and the `0x00/0x09/0x0B/0x0D/0x0E` gaps are the obvious places for
  it to land. **Action:** obtain Audacity 4.x, save a project, re-run the probe.
  Until then, treat "AUP4 superset" in D1 as a design intent validated only
  against AUP3.
- `envelope` / `numpoints` appear in every project but are always empty here;
  the point encoding is unexercised by this corpus.
- The `0x00 0x04` dict prologue is invariant across all 25 files. Its meaning is
  unconfirmed — plausibly a format-version marker. The decoder asserts it rather
  than interpreting it.

## Test fixtures — a problem with a clean answer

The corpus cannot be committed: the smallest project is 271 MB. CI still needs
genuine Audacity bytes, not something we wrote ourselves, or the import test
only proves our encoder agrees with our decoder.

The grammar makes a surgical shrink possible without re-encoding anything.
Records are self-delimiting, so dropping all but the first few `waveblock`
records per channel is a byte-slice deletion, and correcting `numsamples` is a
fixed-width `u64` patched in place. Delete the corresponding `sampleblocks`
rows, `VACUUM`, and the result is a few-MB project whose every surviving byte
was written by Audacity. That belongs in WP-20, not here.

## Next

Port the grammar to Rust in the importer work package with the corpus as the
regression test: parse every file, require full byte consumption, require zero
dangling block references. The Python probe stays as the oracle the Rust
implementation is diffed against.

Concrete follow-ups:
1. Obtain Audacity 4.x, save projects covering each sample format, mono and
   stereo, multi-clip, with labels and metadata; re-run the probe. This is the
   only thing blocking S5 closure.
2. Build the fixture shrinker described above (WP-20).
3. Find or make a project with a non-empty `envelope`, and one with a populated
   `autosave`, so both paths are exercised.
