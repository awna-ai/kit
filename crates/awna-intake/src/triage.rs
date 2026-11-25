use crate::{
    ValidationError,
    text::{check_block, check_line},
};
use serde::{Deserialize, Serialize};

/// The kind of request inferred during triage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TriageKind {
    /// Something behaves incorrectly.
    Bug,
    /// A small adjustment to existing behavior.
    Tweak,
    /// New behavior is requested.
    Feature,
    /// The request cannot yet be classified.
    Unclear,
}
/// The estimated size of the proposed work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TriageSize {
    /// A small body of work.
    Small,
    /// A large body of work.
    Large,
}
/// A validated assessment of a submitted request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawTriage")]
pub struct Triage {
    /// The classification.
    pub kind: TriageKind,
    /// The estimated size.
    pub size: TriageSize,
    /// Whether the request is clear enough to act on.
    pub clear: bool,
    /// A line of at most 280 bytes.
    pub summary: String,
    /// A block of at most 16,384 bytes.
    pub details: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawTriage {
    kind: TriageKind,
    size: TriageSize,
    clear: bool,
    summary: String,
    details: String,
}
impl TryFrom<RawTriage> for Triage {
    type Error = ValidationError;
    fn try_from(raw: RawTriage) -> Result<Self, Self::Error> {
        let value = Self {
            kind: raw.kind,
            size: raw.size,
            clear: raw.clear,
            summary: raw.summary,
            details: raw.details,
        };
        value.validate()?;
        Ok(value)
    }
}
impl Triage {
    /// Checks every field against the contract.
    pub fn validate(&self) -> Result<(), ValidationError> {
        check_line("summary", &self.summary, 280)?;
        check_block("details", &self.details, 0, 16_384)
    }
}
