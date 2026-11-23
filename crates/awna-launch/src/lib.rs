//! Install and control user services: launchd agents on macOS and systemd user units on Linux.
//! Describe a service with a typed unit. Apply updates with a policy for running and disabled
//! services, bounded retries, and a health check. Use a recording manager to test changes without
//! contacting the service manager.
//!
//! ```no_run
//! use awna_launch::{Launchctl, Policy, Run, Unit, apply};
//! use std::path::Path;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let home = Path::new("/var/tmp/example/home");
//! // Use the account's password-database entry for account_home, never $HOME.
//! let account_home = Path::new("/var/tmp/example/home");
//! let manager = Launchctl::resolve(None, home, account_home, 501)?;
//! let agents = home.join("Library/LaunchAgents");
//! let unit = Unit::new(
//!     "com.example.alpha",
//!     vec!["/opt/example/bin/alpha".into(), "serve".into()],
//!     Run::KeepAlive,
//! );
//! apply(&manager, &agents, &unit, &Policy::default(), &mut || true)?;
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod apply;
mod command;
mod launchctl;
mod manager;
mod plist;
mod stand_in;
mod systemctl;
mod systemd;
mod unit;

pub use apply::{
    Applied, ApplyError, Policy, Removed, Rollback, Tries, Wait, apply, plist_path, remove, retire,
    unit_path,
};
pub use launchctl::Launchctl;
pub use manager::{Manager, ManagerError, ResolveError, State, parse_disabled, parse_print};
pub use plist::{Platform, render};
pub use stand_in::{Call, StandIn};
pub use systemctl::{LINGER_DIR, Systemctl, lingering, parse_masked, parse_show};
pub use unit::{CalendarTime, Kind, Logs, Run, Unit, UnitError, valid_label};
