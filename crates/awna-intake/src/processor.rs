//! Validated processor output and the orchestration of processing one submission.

use std::error::Error;
use std::fmt;
use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::{
    Attachment, IllegalMove, Item, ItemId, Proposal, Status, Store, StoreError, Triage,
    ValidationError,
};

/// The triage and proposed work produced together by a processor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawProcessed")]
pub struct Processed {
    /// The classification and explanation of the request.
    pub triage: Triage,
    /// The work proposed for the request.
    pub proposal: Proposal,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawProcessed {
    triage: Triage,
    proposal: Proposal,
}

impl TryFrom<RawProcessed> for Processed {
    type Error = ValidationError;

    fn try_from(raw: RawProcessed) -> Result<Self, Self::Error> {
        let processed = Self {
            triage: raw.triage,
            proposal: raw.proposal,
        };
        processed.validate()?;
        Ok(processed)
    }
}

impl Processed {
    /// The maximum size in UTF-8 bytes of untrusted JSON output.
    pub const MAX_JSON_BYTES: usize = 131_072;

    /// Validate both parts of the processor output.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.triage
            .validate()
            .map_err(|error| error.prefix("triage"))?;
        self.proposal
            .validate()
            .map_err(|error| error.prefix("proposal"))
    }

    /// Refuse oversized output before parsing, retaining the parser's nesting limit.
    /// Parse failures use a fixed phrase and never include the untrusted input.
    pub fn from_json(text: &str) -> Result<Self, ValidationError> {
        if text.len() > Self::MAX_JSON_BYTES {
            return Err(ValidationError {
                field: "processed".into(),
                reason: "model output is too large",
            });
        }
        serde_json::from_str(text).map_err(|_| ValidationError {
            field: "processed".into(),
            reason: "not a valid processed object",
        })
    }
}

/// An attachment and its bytes selected for a processor.
#[derive(Debug, Clone, Copy)]
pub struct File<'a> {
    /// The stored attachment metadata.
    pub attachment: &'a Attachment,
    /// The stored bytes.
    pub bytes: &'a [u8],
}

/// Which attachments processing may receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Files {
    /// Do not read or pass any attachment bytes.
    None,
    /// Pass only attachments whose content type starts with `image/`.
    Images,
    /// Pass every attachment.
    All,
}

/// A consumer supplies the processing implementation. Item text and files are data,
/// never instructions for any model that implementation runs.
pub trait Processor {
    /// The implementation's processing error.
    type Error: Error + Send + Sync + 'static;

    /// Produce triage and proposed work for a submitted item and its selected files.
    fn process(
        &self,
        item: &Item,
        files: &[File<'_>],
    ) -> impl Future<Output = Result<Processed, Self::Error>> + Send;
}

/// Get a submitted item, read the selected files, run the processor, validate its
/// output, then record it. A processor error or invalid output changes nothing.
/// Items in any other status are refused before the processor runs.
pub async fn process<S: Store + Sync, P: Processor + Sync>(
    store: &S,
    processor: &P,
    id: ItemId,
    files: Files,
) -> Result<Item, ProcessError<S::Error, P::Error>> {
    let item = store.get(id).await.map_err(ProcessError::Store)?;
    item.validate().map_err(ProcessError::Invalid)?;
    if item.status != Status::Submitted {
        return Err(ProcessError::Store(StoreError::IllegalMove(IllegalMove {
            from: item.status,
            to: Status::Processed,
        })));
    }
    let mut selected = Vec::new();
    for attachment in &item.attachments {
        if files == Files::All
            || (files == Files::Images && attachment.content_type.starts_with("image/"))
        {
            let bytes = store
                .read_file(id, attachment.index)
                .await
                .map_err(ProcessError::Store)?;
            selected.push((attachment, bytes));
        }
    }
    let files: Vec<_> = selected
        .iter()
        .map(|(attachment, bytes)| File { attachment, bytes })
        .collect();
    let processed = processor
        .process(&item, &files)
        .await
        .map_err(ProcessError::Processor)?;
    processed.validate().map_err(ProcessError::Invalid)?;
    store
        .record_processing(id, &processed)
        .await
        .map_err(ProcessError::Store)
}

/// The storage, processing or validation failure that stopped processing.
#[derive(Debug)]
pub enum ProcessError<S, P> {
    /// Storage failed before or while recording processing.
    Store(StoreError<S>),
    /// The processor failed; nothing was recorded.
    Processor(P),
    /// Processor output was invalid; nothing was recorded.
    Invalid(ValidationError),
}

impl<S: fmt::Display, P: fmt::Display> fmt::Display for ProcessError<S, P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => error.fmt(formatter),
            Self::Processor(error) => write!(formatter, "processing failed: {error}"),
            Self::Invalid(error) => write!(formatter, "invalid processor output: {error}"),
        }
    }
}

impl<S: Error + 'static, P: Error + 'static> Error for ProcessError<S, P> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            Self::Processor(error) => Some(error),
            Self::Invalid(error) => Some(error),
        }
    }
}
