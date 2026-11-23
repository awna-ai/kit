//! Install and control macOS user services. Describe a service with a typed unit and render it
//! as a launchd property list. Apply updates with a policy for running and disabled services,
//! bounded retries, and a health check. Use a recording manager to test changes without
//! contacting launchd.
//!
//! ```no_run
//! use awna_launch::{Launchctl, Policy, Run, Unit, apply};
//! use std::path::Path;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let account_home = Path::new("/var/tmp/example/home");
//! let manager = Launchctl::resolve(None, account_home, account_home, 501)?;
//! let agents = account_home.join("Library/LaunchAgents");
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
mod launchctl;
mod manager;
mod plist;
mod stand_in;
mod unit;

pub use apply::{
    Applied, ApplyError, Policy, Removed, Rollback, Tries, Wait, apply, plist_path, remove, retire,
};
pub use launchctl::{Launchctl, ResolveError};
pub use manager::{Manager, ManagerError, State, parse_disabled, parse_print};
pub use plist::{Platform, render};
pub use stand_in::{Call, StandIn};
pub use unit::{CalendarTime, Kind, Logs, Run, Unit, UnitError, valid_label};
