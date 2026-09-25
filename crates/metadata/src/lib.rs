//! Release metadata providers.
//!
//! Requirements: §28 (providers), §32 (genres), §40 (caching, rate limits, timeouts).
//!
//! The provider trait is the point of this crate: §40 requires the application to be
//! fully usable with networking disabled, so every provider is fixture-backed in test
//! and every call is cancellable and time-boxed in production.

pub mod artwork;
pub mod discogs;
pub mod genres;
pub mod musicbrainz;
