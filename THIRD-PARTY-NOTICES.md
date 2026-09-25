# Third-Party Notices

VCW's own source code is licensed under the MIT License - see [LICENSE](LICENSE).

VCW binaries are **statically linked** Rust executables: the compiled artefacts embed
the object code of VCW's dependencies. Some of those dependencies carry licences whose
notice and source-availability terms apply to anyone redistributing those binaries.
This file records them.

A full machine-readable inventory of every dependency and its version is
[`Cargo.lock`](Cargo.lock); each release tag pins the exact versions used to build that
release's binaries. [`deny.toml`](deny.toml) encodes which licences are acceptable, and
CI fails the build on anything outside that set - so this file cannot silently fall out
of date with what is actually linked.

---

## Current state, as of the Phase 1 scaffold

The shipped crates under `crates/` link **permissive dependencies only**:
`MIT OR Apache-2.0` predominantly, with a smaller number under Apache-2.0,
BSD-2-Clause, BSD-3-Clause, ISC, Zlib, BSL-1.0, Unicode-3.0, CC0-1.0,
CDLA-Permissive-2.0 and Unlicense. These require attribution and nothing more; their
copyright notices are carried in their respective crate sources, referenced by
`Cargo.lock`.

Two dependencies are worth naming because they are load-bearing rather than incidental:

- **`cpal`** (Apache-2.0) - the audio host abstraction. It is an ordinary crates.io
  dependency, used unmodified. VCW briefly carried a patched fork; both defects behind
  that fork were fixed upstream in 0.18.2 and the fork was deleted.
- **`rusqlite`** with the `bundled` feature (MIT) - which compiles SQLite itself
  (public domain) into the binary. Bundling is deliberate: it removes platform SQLite
  variance from a file format we have to be able to recover.

The sections below describe obligations that arrive with **Phase 2**. They are written
now, while the decisions are fresh, rather than at the point of release.

---

## LGPL-2.1-or-later - `chromaprint-next` (Phase 2)

VCW will use [`chromaprint-next`](https://github.com/attilagyorffy/chromaprint-next) to
compute AcoustID audio fingerprints in-process (REQUIREMENTS §25). That crate is
licensed `MIT AND LGPL-2.1-or-later`:

- Most of the crate is MIT (Copyright (c) 2010-2016 Lukas Lalinsky for the original
  C/C++ Chromaprint; Copyright (c) 2026 Attila Györffy for the Rust port).
- Its resampler module (`src/audio/resample.rs`) is a port of FFmpeg's `av_resample`
  (Copyright (c) 2004 Michael Niedermayer) and is licensed **LGPL-2.1-or-later**.

The full text of the GNU Lesser General Public License, version 2.1, is included in
this repository as [LICENSE-LGPL-2.1](LICENSE-LGPL-2.1) and will ship alongside every
released binary that links the crate.

The crate is used **unmodified**, as an ordinary Cargo dependency. The dependency of
record is the published release on crates.io, not a local checkout or a fork - see
`docs/adr/0004-licence-and-toolchain.md`.

### Notice to users of VCW binaries containing LGPL code

Under section 6 of the LGPL you have the right to modify the LGPL portion and relink it
into VCW. VCW supports this as follows:

1. **Complete corresponding source.** The complete source of VCW is this repository,
   under the MIT licence. The complete source of the LGPL component is published on
   [crates.io](https://crates.io/crates/chromaprint-next) and at the upstream
   repository linked above. The exact version used by any release is recorded in
   `Cargo.lock` at that release's tag.
2. **Relinking.** Because the complete source of the "work that uses the Library" is
   available under the MIT licence, you can modify `chromaprint-next` and rebuild VCW
   yourself to produce a binary incorporating your modified version:

   ```toml
   # Cargo.toml
   [patch.crates-io]
   chromaprint-next = { path = "../my-modified-chromaprint-next" }
   ```

   ```sh
   cargo build --release
   ```

3. **No further restrictions.** VCW does not impose terms on the LGPL portion beyond
   those in LGPL-2.1, and the binaries are not obfuscated or licence-restricted in a
   way that would prevent reverse engineering for debugging your modifications.

---

## LGPL - MP3 and Ogg Vorbis encoders (Phase 2, and optional)

Decision D5 selects `mp3lame-encoder` (which links libmp3lame) and `vorbis_rs` for MP3
and Ogg Vorbis export. Both are LGPL and both extend the relink obligation above to
their own libraries.

Because these are *export conveniences* rather than core function - the archival
formats are WAV and FLAC, and FLAC's encoder (`flacenc`) is pure Rust and Apache-2.0 -
they are candidates for **optional cargo features**, so that a default build carries no
LGPL obligation at all. That choice is made at WP-14, not here, and whichever way it
goes this file records the outcome.

---

## Regenerating a complete per-crate listing

```sh
cargo install cargo-about
cargo about generate --format json
```

`cargo deny check licenses` is the cheaper day-to-day check and runs in CI on every
push.
