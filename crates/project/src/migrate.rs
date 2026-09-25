//! Transactional, forward-only schema migrations (§16).
//!
//! Every migration runs inside its own transaction together with the bookkeeping
//! that records it, so a migration either happened or did not. There is no state in
//! which the DDL landed but `user_version` and `schema_migrations` disagree with it.
//!
//! Forward-only is a deliberate limitation. §16 asks that a newer application
//! upgrade an older project; it does not ask for downgrade, and a downgrade path
//! would have to decide what to do with data the older schema cannot hold. Opening
//! a project from the future is refused instead - see [`crate::Error::SchemaTooNew`].

use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::schema::SCHEMA_V1;

/// One step from schema version `version - 1` to `version`.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    /// The version this step produces.
    pub version: u32,
    /// What it does, recorded in `schema_migrations` and shown if it fails.
    pub description: &'static str,
    /// The DDL, executed as a batch.
    pub sql: &'static str,
}

/// The migrations that build the current schema, in ascending order.
///
/// Migration 1 *is* the schema: a fresh project is a migration run against an empty
/// database, so the create path and the upgrade path cannot drift apart.
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    description: "initial capture schema",
    sql: SCHEMA_V1,
}];

/// Identifies the build that applied a migration, for the audit row.
pub const APPLIED_BY: &str = concat!("vcw-project ", env!("CARGO_PKG_VERSION"));

/// The schema version recorded in the file.
pub fn current_version(conn: &Connection) -> rusqlite::Result<u32> {
    conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
        .map(|v| v as u32)
}

/// Migrations in `set` that have not been applied to `conn`.
pub fn pending<'a>(
    conn: &Connection,
    set: &'a [Migration],
) -> rusqlite::Result<Vec<&'a Migration>> {
    let at = current_version(conn)?;
    Ok(set.iter().filter(|m| m.version > at).collect())
}

/// Applies every pending migration in `set`, in order. Returns the versions applied.
///
/// Each step is one transaction. A failure rolls that step back and stops: later
/// migrations are not attempted, because they were written against a schema that
/// does not exist.
pub fn apply(conn: &mut Connection, set: &[Migration]) -> Result<Vec<u32>> {
    debug_assert!(
        set.windows(2).all(|w| w[0].version < w[1].version),
        "migrations must be ascending and unique"
    );

    let mut applied = Vec::new();
    for m in pending(conn, set)? {
        let tx = conn.transaction()?;
        let step = (|| -> rusqlite::Result<()> {
            tx.execute_batch(m.sql)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, description, applied_at, applied_by)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![m.version, m.description, crate::now(), APPLIED_BY],
            )?;
            // Not a bindable parameter: PRAGMA takes a literal.
            tx.pragma_update(None, "user_version", m.version)?;
            Ok(())
        })();

        match step {
            Ok(()) => {
                tx.commit()?;
                applied.push(m.version);
            }
            Err(source) => {
                // Explicit, though the Drop impl would also roll back.
                let _ = tx.rollback();
                return Err(Error::Migration {
                    version: m.version,
                    description: m.description.to_owned(),
                    source,
                });
            }
        }
    }
    Ok(applied)
}

/// The highest version `set` can produce.
pub fn target_version(set: &[Migration]) -> u32 {
    set.iter().map(|m| m.version).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_migration_set_reaches_the_declared_schema_version() {
        assert_eq!(target_version(MIGRATIONS), crate::schema::SCHEMA_VERSION);
    }

    #[test]
    fn migrations_are_ascending_and_unique() {
        assert!(MIGRATIONS.windows(2).all(|w| w[0].version < w[1].version));
        assert_eq!(MIGRATIONS.first().map(|m| m.version), Some(1));
    }
}
