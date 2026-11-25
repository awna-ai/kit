use crate::ValidationError;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// The current stage of an intake item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Received, not yet processed.
    Submitted,
    /// Triaged and proposed, waiting to be routed.
    Processed,
    /// Accepted, with its work filed.
    Queued,
    /// Waiting for a person.
    Held,
    /// Being worked.
    InProgress,
    /// The work landed.
    Built,
    /// Released to the sender.
    Shipped,
    /// A person said no.
    Declined,
    /// Withdrawn.
    Cancelled,
}
impl Status {
    /// Every status, in contract order.
    pub const ALL: [Self; 9] = [
        Self::Submitted,
        Self::Processed,
        Self::Queued,
        Self::Held,
        Self::InProgress,
        Self::Built,
        Self::Shipped,
        Self::Declined,
        Self::Cancelled,
    ];
    /// Whether the status table permits this move.
    pub fn can_move_to(self, to: Self) -> bool {
        matches!(
            (self, to),
            (
                Self::Submitted,
                Self::Processed | Self::Held | Self::Cancelled
            ) | (Self::Processed, Self::Queued | Self::Held | Self::Cancelled)
                | (Self::Held, Self::Queued | Self::Declined | Self::Cancelled)
                | (Self::Queued, Self::InProgress | Self::Cancelled)
                | (
                    Self::InProgress,
                    Self::Queued | Self::Built | Self::Cancelled
                )
                | (Self::Built, Self::Shipped)
        )
    }
    /// Refuses a move outside the status table, including a move to itself.
    pub fn check_move(self, to: Self) -> Result<(), IllegalMove> {
        if self.can_move_to(to) {
            Ok(())
        } else {
            Err(IllegalMove { from: self, to })
        }
    }
    /// Whether no outgoing move exists.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Shipped | Self::Declined | Self::Cancelled)
    }
    /// The JSON spelling of this status.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Processed => "processed",
            Self::Queued => "queued",
            Self::Held => "held",
            Self::InProgress => "in_progress",
            Self::Built => "built",
            Self::Shipped => "shipped",
            Self::Declined => "declined",
            Self::Cancelled => "cancelled",
        }
    }
}
impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl FromStr for Status {
    type Err = ValidationError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == value)
            .ok_or_else(|| ValidationError::new("status", "unknown status"))
    }
}

/// A refused move between two statuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IllegalMove {
    /// The current status.
    pub from: Status,
    /// The requested status.
    pub to: Status,
}
impl fmt::Display for IllegalMove {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "illegal move from {} to {}", self.from, self.to)
    }
}
impl std::error::Error for IllegalMove {}
