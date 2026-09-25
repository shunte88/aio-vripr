//! Playback engine, transport and audition (§21).
//!
//! Filled by WP-10. Reports its own path honestly for the same reason capture does:
//! monitoring through the OS mixer is not the same as monitoring bit-perfect, and
//! the UI has to be able to say which one the user is hearing.
