//! Credentials held in the system password store or in zeroized memory.
//!
//! [`system()`] opens this computer's store: the macOS keychain through `/usr/bin/security`, the Windows Credential
//! Manager, or the Linux Secret Service. [`MemoryStore`] keeps values in memory only. Every store refuses a value
//! [`storable`] rejects and reads each write back before it reports success.
//!
//! # Limits
//!
//! - The macOS `security` program runs with no timeout, so a locked keychain can keep a call waiting on its prompt.
//! - Values are wiped where this crate holds them, but the standard library's output buffer and the Windows and
//!   Linux stores may keep copies this crate cannot reach.
//! - `find` returns any non-empty text; a caller that needs the token rule checks [`storable`] itself.
//! - The Windows store has no label; the Linux store needs a running Secret Service, else [`Error::NoStore`].
//! - The Linux store sets an item's label when first creating it; later stores keep the old label.
//! - Items written through `/usr/bin/security` trust that program: any process of the same user can read them
//!   with `security find-generic-password -w` without a prompt, as with any item that program writes.
//! - Writing to `security` can raise SIGPIPE if it exits early. Rust programs ignore SIGPIPE; a host whose
//!   `main` is not Rust should ignore it too.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod item;
#[cfg(any(windows, target_os = "linux", test))]
mod keyring;
mod memory;
mod secret;
#[cfg(unix)]
mod security;

pub use item::Item;
pub use memory::MemoryStore;
pub use secret::{MAX_SECRET, Secret, storable};
#[cfg(unix)]
pub use security::SecurityCli;

use std::fmt;

/// A failure that never includes credential values or raw store errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A service, account, or label is invalid.
    InvalidItem,
    /// A value was refused before any operation.
    NotStorable,
    /// No supported system store is available.
    NoStore,
    /// The stored value is empty or is not UTF-8.
    Unreadable {
        /// The service/account identifier.
        item: String,
    },
    /// A write could not be confirmed by reading it back.
    ReadBack {
        /// The service/account identifier.
        item: String,
    },
    /// An operation failed with a sanitized reason.
    Failed {
        /// The service/account identifier.
        item: String,
        /// The operation attempted.
        op: &'static str,
        /// A fixed description or numeric exit code.
        why: String,
    },
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidItem => f.write_str("The credential item is invalid."),
            Self::NotStorable => f.write_str("The credential value cannot be stored."),
            Self::NoStore => f.write_str("No supported password store is available."),
            Self::Unreadable { item } => write!(f, "Could not find readable text for {item}."),
            Self::ReadBack { item } => {
                write!(f, "Could not confirm the store operation for {item}.")
            }
            Self::Failed { item, op, why } => write!(f, "Could not {op} {item}: {why}."),
        }
    }
}
impl std::error::Error for Error {}

fn item_name(item: &Item) -> String {
    format!("{}/{}", item.service(), item.account())
}

/// A password store whose writes are validated and read back before success.
pub trait CredentialStore: Send + Sync {
    /// Returns the value, or `None` when the item is missing.
    fn find(&self, item: &Item) -> Result<Option<Secret>, Error>;
    /// Reports whether the item is filed in the store.
    fn exists(&self, item: &Item) -> Result<bool, Error>;
    /// Validates, writes, and reads back the value, succeeding only on an exact match.
    fn store(&self, item: &Item, value: &Secret) -> Result<(), Error>;
    /// Removes an item; a missing item is not an error.
    fn delete(&self, item: &Item) -> Result<(), Error>;
}

fn read_back(store: &dyn CredentialStore, item: &Item, value: &Secret) -> Result<(), Error> {
    match store.find(item)? {
        Some(found) if found.expose() == value.expose() => Ok(()),
        _ => Err(Error::ReadBack {
            item: item_name(item),
        }),
    }
}

/// Opens this computer's system password store, or reports that none is available.
/// Never falls back to a file.
pub fn system() -> Result<Box<dyn CredentialStore>, Error> {
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(SecurityCli::new()))
    }
    #[cfg(any(windows, target_os = "linux"))]
    {
        Ok(Box::new(keyring::KeyringStore::open()?))
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        Err(Error::NoStore)
    }
}
