//! Validated intake items and their store and processor contracts, with no I/O.
//!
//! ```
//! use awna_intake::{MemoryStore, NewItem, Source, RequesterKey, Status, Store};
//! # fn block_on<F: std::future::Future>(future: F) -> F::Output {
//! #     let mut future = std::pin::pin!(future);
//! #     let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
//! #     loop { if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut cx) { return value; } }
//! # }
//! let store = MemoryStore::default();
//! let new = NewItem {
//!     source: Source::new("web-form")?,
//!     requester: RequesterKey::new("someone@example.com")?,
//!     title: "The total on the summary page is wrong".into(),
//!     body: "The displayed total differs from the rows.".into(),
//!     snapshot: None,
//! };
//! let item = block_on(store.submit(&new, &[]))?;
//! let held = block_on(store.set_status(item.id, Status::Held, None))?;
//! let queued = block_on(store.set_status(held.id, Status::Queued, None))?;
//! assert_eq!(queued.status, Status::Queued);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod attachment;
/// A reusable suite for store implementations.
pub mod conformance;
mod error;
mod item;
mod memory;
mod processor;
mod proposal;
mod status;
mod store;
mod text;
mod triage;

pub use attachment::*;
pub use error::*;
pub use item::*;
pub use memory::*;
pub use processor::*;
pub use proposal::*;
pub use status::*;
pub use store::*;
pub use triage::*;

/// The version of the serialized intake contract.
pub const CONTRACT_VERSION: &str = "v1";
