//! Schema, migrations, open/create/validate, integrity checking (§12, §16, §49).
//!
//! Filled by WP-02. `rusqlite` with the `bundled` feature (D2): synchronous and
//! predictable, no async runtime anywhere near the writer thread, and one SQLite
//! build across every platform instead of whatever the OS shipped.
//!
//! Two habits inherited from S5, both cheap and both load-bearing. Open read paths
//! with `mode=ro`, never `immutable=1` - the latter makes SQLite ignore the `-wal`
//! sidecar and silently serve a stale database. And dispatch on `user_version`
//! rather than on the magic or the extension, because AUP3 and AUP4 share an
//! `application_id`.

/// The SQLite library version this binary is linked against.
///
/// Bundled, so it is a property of the build rather than of the machine - which is
/// the point of bundling it, and worth printing when a project file misbehaves.
pub fn runtime_version() -> &'static str {
    rusqlite::version()
}

/// SQLite `application_id` for a `.vcw` project: ASCII `"VCW\0"`.
///
/// Distinct from Audacity's `0x41554459` (`"AUDY"`), which both AUP3 and AUP4 use.
pub const APPLICATION_ID: i32 = 0x5643_5700;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_is_bundled_and_recent() {
        let v = runtime_version();
        let major: u32 = v
            .split('.')
            .next()
            .and_then(|s| s.parse().ok())
            .expect("version");
        assert!(major >= 3, "unexpected SQLite version {v}");
    }

    #[test]
    fn application_id_is_not_audacitys() {
        assert_ne!(APPLICATION_ID as u32, 0x4155_4459);
        assert_eq!(&(APPLICATION_ID as u32).to_be_bytes(), b"VCW\0");
    }
}
