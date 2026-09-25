# ADR-0001: Native project format, extension, and Audacity compatibility

**Status:** Accepted 2026-09-22. Validated against real AUP4 bytes 2026-09-24.
**Decision:** D1 · **Requirements:** §12, §16, §49 · **Evidence:** S2, S5

## Context

§12 requires a single-file SQLite project "following the architectural principles of
Audacity's AUP3/AUP4 project storage", self-contained and movable, with the extension
to be chosen before implementation. It leaves three questions open: what we call the
file, how close to Audacity's schema we sit, and whether we interoperate in one
direction or both.

VRipr, the predecessor, had no project format at all - it drove Audacity over a pipe.
So there is no legacy to carry, and the corpus of 25 real `.aup3` vinyl rips in
`/data2/vinyl_rips` is the only existing project data that matters.

## Decision

**Extension `.vcw`.** One file, self-identifying through SQLite's `application_id`
(ASCII `VCW\0`, `0x56435700`) and `user_version`. A reader never has to trust the
extension, which is the mistake an Audacity reader makes if it dispatches on `.aup3`
versus `.aup4` - both files carry `application_id` `0x41554459`, and only
`user_version` separates them.

**The schema is a deliberate superset of AUP4's.** `sampleblocks` column-for-column
identical, the same summary pyramids (256:1 and 64k:1), blocks immutable once written.
Our own tables sit alongside for disc and side topology, identification evidence,
export settings and recovery state, none of which Audacity has an equivalent for.

**Audacity interop is import-only, both versions.** We read `.aup3` and `.aup4`. We do
not write either. Export to Audacity is declined.

## Why a superset rather than a clone

Because §8 requires 32-bit integer capture and Audacity cannot represent it. S5
enumerated the complete set of `sampleformat` codes in the corpus - `0x00020001`
(int16), `0x00040001` (int24), `0x0004000F` (float32) - and there is no fourth. A clone
would have to either drop a required capture format or invent a code inside Audacity's
namespace, which is worse than being openly different.

The superset also lets us store what Audacity does not: per-block checksums for §15
recovery, capture diagnostics counters that §10 requires persisted, and boundary
provenance that §24 requires distinguishable.

## Why import-only

Writing `.aup4` would mean maintaining byte-level compatibility with a format that
belongs to someone else and changes when they change it, in exchange for a workflow -
round-tripping a capture back into Audacity - that the application exists to replace.
It also inverts the risk: a bad import is a failed import, a bad export is a corrupted
file in someone else's tool.

## Evidence this rests on

S5 decoded both versions from file bytes alone and parsed all 30 corpus projects to the
last byte. The findings that bear directly on this decision:

- `sampleblocks` is **column-for-column identical** between AUP3 and AUP4, and the
  AUP3 -> AUP4 conversion leaves it **byte-identical** across five matched pairs
  spanning both sample rates, both sample formats and both page sizes. The superset
  claim therefore rests on measurement, not on the 3.x schema plus an assumption.
- AUP4 adds exactly one table (`project_history`) and one record type (a
  length-prefixed binary blob, used only for an editor screenshot). Neither touches
  audio.
- `project/@rate` is a stored editor preference, not the sample rate; `wavetrack/@rate`
  is authoritative. This reverses our own 2026-09-22 conclusion. See
  [`docs/spikes/S5-audacity-format.md`](../spikes/S5-audacity-format.md).

## Consequences

- WP-02 builds the schema against a real `.aup4`, diffed in CI, rather than against a
  description of one. Built 2026-09-25; the result is documented in
  [`docs/SCHEMA.md`](../SCHEMA.md), generated from the DDL.
- WP-20 builds the importer. The corpus is its regression set and needs a fixture
  shrinker, since 451 MB to 4.7 GB projects are not committable.
- The clean-room constraint (risk R13) holds: the document grammar was derived by
  observing bytes, with no Audacity source consulted. Any extension to it must be
  derived the same way.
- We inherit no obligation to track Audacity's releases. A future AUP5 is an import
  problem, discovered by `user_version`, and the importer refuses cleanly on a version
  it does not know rather than guessing.
