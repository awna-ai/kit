//! The storage boundary and its filtering and error types.

use std::error::Error;
use std::fmt;
use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::{
    IllegalMove, Item, ItemId, Limits, NewFile, NewItem, Note, Processed, Reference, RequesterKey,
    Source, Status, ValidationError,
};

/// A storage implementation owns locking, persistence and attachment bytes.
pub trait Store {
    /// The implementation's backend error.
    type Error: Error + Send + Sync + 'static;

    /// The limits applied before a submission is stored.
    fn limits(&self) -> &Limits;

    /// Validate the item and files before storing anything, returning `Invalid` on failure.
    /// Assign the next id and attachment indices in submission order; return a submitted
    /// item with one status event from `None` to `Submitted`, no reference,
    /// and both timestamps set to now. Timestamps
    /// never go backwards within an item.
    fn submit(
        &self,
        item: &NewItem,
        files: &[NewFile<'_>],
    ) -> impl Future<Output = Result<Item, StoreError<Self::Error>>> + Send;

    /// Return the item, or `NotFound(id)` when it does not exist.
    fn get(&self, id: ItemId)
    -> impl Future<Output = Result<Item, StoreError<Self::Error>>> + Send;

    /// Match every set filter field, ordered by ascending id, with at most `limit` items.
    fn list(
        &self,
        filter: &Filter,
    ) -> impl Future<Output = Result<Vec<Item>, StoreError<Self::Error>>> + Send;

    /// Validate the output first, returning `Invalid` on failure. Require submitted status
    /// or return `IllegalMove`; atomically store both outputs, move to processed, append
    /// its status event and update the timestamp without going backwards.
    fn record_processing(
        &self,
        id: ItemId,
        processed: &Processed,
    ) -> impl Future<Output = Result<Item, StoreError<Self::Error>>> + Send;

    /// Refuse processed with `Invalid`, field `to`, reason
    /// "only record_processing moves an item to processed".
    /// Otherwise atomically check the move and write it, or return `IllegalMove`.
    /// Append a status event with the reference and update time without going backwards.
    fn set_status(
        &self,
        id: ItemId,
        to: Status,
        reference: Option<&Reference>,
    ) -> impl Future<Output = Result<Item, StoreError<Self::Error>>> + Send;

    /// Validate the note first (`Invalid` on failure), then append it in any status,
    /// including terminal statuses. Update the timestamp without going backwards.
    fn add_note(
        &self,
        id: ItemId,
        note: &Note,
    ) -> impl Future<Output = Result<Item, StoreError<Self::Error>>> + Send;

    /// Return the stored bytes, `NotFound(id)` for no item, or `NoFile` for no index.
    fn read_file(
        &self,
        id: ItemId,
        index: u32,
    ) -> impl Future<Output = Result<Vec<u8>, StoreError<Self::Error>>> + Send;
}

/// A storage failure, with validation and transition failures separated from backend errors.
#[derive(Debug)]
pub enum StoreError<E> {
    /// No item exists with this id.
    NotFound(ItemId),
    /// The item exists but has no attachment at this index.
    NoFile {
        /// The item's id.
        id: ItemId,
        /// The missing attachment's index.
        index: u32,
    },
    /// The requested status move is illegal.
    IllegalMove(IllegalMove),
    /// Input failed validation before being stored.
    Invalid(ValidationError),
    /// An implementation-specific storage failure.
    Backend(E),
}

impl<E: fmt::Display> fmt::Display for StoreError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(formatter, "item {id} not found"),
            Self::NoFile { id, index } => write!(formatter, "item {id} has no file {index}"),
            Self::IllegalMove(error) => error.fmt(formatter),
            Self::Invalid(error) => write!(formatter, "invalid input: {error}"),
            Self::Backend(error) => write!(formatter, "storage failed: {error}"),
        }
    }
}

impl<E: Error + 'static> Error for StoreError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Backend(error) => Some(error),
            Self::IllegalMove(error) => Some(error),
            _ => None,
        }
    }
}

/// Every set field must match. An empty status list matches every status.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawFilter")]
pub struct Filter {
    /// Accepted statuses, or empty for all statuses.
    pub statuses: Vec<Status>,
    /// Match this source when set.
    pub source: Option<Source>,
    /// Match this requester when set.
    #[serde(rename = "requesterKey")]
    pub requester: Option<RequesterKey>,
    /// Return at most this many items when set.
    pub limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawFilter {
    statuses: Vec<Status>,
    source: Option<Source>,
    #[serde(rename = "requesterKey")]
    requester: Option<RequesterKey>,
    limit: Option<u32>,
}

impl TryFrom<RawFilter> for Filter {
    type Error = ValidationError;

    fn try_from(raw: RawFilter) -> Result<Self, Self::Error> {
        let filter = Self {
            statuses: raw.statuses,
            source: raw.source,
            requester: raw.requester,
            limit: raw.limit,
        };
        filter.validate()?;
        Ok(filter)
    }
}

impl Filter {
    /// Match submitted and processed items that are waiting to be routed.
    pub fn pending() -> Self {
        Self {
            statuses: vec![Status::Submitted, Status::Processed],
            ..Self::default()
        }
    }

    /// Validate this filter; its typed fields have no additional cross-field restrictions.
    pub fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}
