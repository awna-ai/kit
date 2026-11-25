//! A Postgres store for validated intake items and its host-applied schema.
//!
//! Apply [`SCHEMA`] through the host application's migration before using [`PgStore`].
//! Database URLs accepted by [`connect_options`] target only loopback or Unix sockets.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

use awna_intake::Limits;

mod store;
mod url;
pub use url::{UrlError, connect_options};

/// The schema, applied byte-for-byte by the host application's migration.
pub const SCHEMA: &str = include_str!("../schema.sql");

/// A Postgres store with submission limits chosen by its host application.
pub struct PgStore {
    pub(crate) pool: sqlx::PgPool,
    pub(crate) limits: Limits,
}

impl PgStore {
    /// Use an existing pool and the given attachment limits.
    pub fn new(pool: sqlx::PgPool, limits: Limits) -> Self {
        Self { pool, limits }
    }

    /// Borrow the pool owned by this store.
    pub fn pool(&self) -> &sqlx::PgPool {
        &self.pool
    }
}

/// An opaque database failure or a stored row that breaks the item contract.
pub enum PgError {
    /// An underlying database failure; its text is never displayed or debugged.
    Sqlx(sqlx::Error),
    /// Stored data failed validation for a fixed reason.
    Corrupt(&'static str),
}

impl From<sqlx::Error> for PgError {
    fn from(error: sqlx::Error) -> Self {
        Self::Sqlx(error)
    }
}

impl fmt::Display for PgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Corrupt(reason) => write!(f, "stored data failed validation: {reason}"),
            Self::Sqlx(sqlx::Error::Database(error)) => match error.code() {
                Some(code) => write!(f, "database error (SQLSTATE {code})"),
                None => f.write_str("database error"),
            },
            Self::Sqlx(sqlx::Error::Io(_) | sqlx::Error::Tls(_)) => {
                f.write_str("database connection failed")
            }
            Self::Sqlx(sqlx::Error::PoolTimedOut) => f.write_str("database pool timed out"),
            Self::Sqlx(_) => f.write_str("database error"),
        }
    }
}

impl fmt::Debug for PgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Error for PgError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlx(error) => Some(error),
            Self::Corrupt(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catch_all_database_errors_are_opaque() {
        for source in [
            sqlx::Error::RowNotFound,
            sqlx::Error::Protocol("pw-SENTINEL-7f3a".into()),
        ] {
            let error = PgError::from(source);
            assert_eq!(error.to_string(), "database error");
            assert_eq!(format!("{error:?}"), "database error");
        }
    }

    #[test]
    fn database_errors_redact_their_sources() {
        let error = PgError::from(sqlx::Error::Io(std::io::Error::other("pw-SENTINEL-7f3a")));
        assert_eq!(error.to_string(), "database connection failed");
        assert_eq!(format!("{error:?}"), "database connection failed");
        assert!(error.source().is_some());
        assert_eq!(
            PgError::from(sqlx::Error::PoolTimedOut).to_string(),
            "database pool timed out"
        );
    }
}
