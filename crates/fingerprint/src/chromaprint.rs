//! Streaming fingerprinter over `chromaprint-next` (§25).
//!
//! Filled at Phase 2. The dependency of record is the published `chromaprint-next`
//! 0.1.0 from crates.io, not the local checkout - see ADR-0004. Linking it adds an
//! LGPL-2.1-or-later relink obligation to released binaries, which is why
//! `THIRD-PARTY-NOTICES.md` and `LICENSE-LGPL-2.1` are in the repository already.
