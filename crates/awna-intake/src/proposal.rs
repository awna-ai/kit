use crate::{
    Reference, ValidationError,
    text::{check_block, check_line},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// The outcome proposed after triage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProposalKind {
    /// One or more tickets should be filed.
    Tickets,
    /// No work should be filed.
    NoAction,
    /// A question needs an answer first.
    Question,
}
/// The generic destination list for a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TicketList {
    /// Ready work.
    #[serde(rename = "To Do")]
    ToDo,
    /// Work deferred or blocked.
    Backlog,
}
/// A dependency on earlier proposed work or an existing external reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Blocker {
    /// The zero-based index of an earlier ticket in the same proposal.
    Ticket(u32),
    /// An opaque reference to existing work.
    External(Reference),
}
/// A proposed unit of work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawTicket")]
pub struct Ticket {
    /// A line of at most 200 bytes.
    pub title: String,
    /// A nonempty block of at most 32,768 bytes.
    pub spec: String,
    /// The destination list.
    pub list: TicketList,
    /// At most 20 distinct dependencies.
    pub blocked_by: Vec<Blocker>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawTicket {
    title: String,
    spec: String,
    list: TicketList,
    blocked_by: Vec<Blocker>,
}
impl TryFrom<RawTicket> for Ticket {
    type Error = ValidationError;
    fn try_from(raw: RawTicket) -> Result<Self, Self::Error> {
        let value = Self {
            title: raw.title,
            spec: raw.spec,
            list: raw.list,
            blocked_by: raw.blocked_by,
        };
        value.validate()?;
        Ok(value)
    }
}
impl Ticket {
    /// Checks every field against the contract.
    pub fn validate(&self) -> Result<(), ValidationError> {
        check_line("title", &self.title, 200)?;
        check_block("spec", &self.spec, 1, 32_768)?;
        if self.blocked_by.len() > 20 {
            return Err(ValidationError::new("blockedBy", "too many blockers"));
        }
        let mut seen = HashSet::new();
        for blocker in &self.blocked_by {
            if !seen.insert(blocker) {
                return Err(ValidationError::new("blockedBy", "duplicate blocker"));
            }
            if matches!(blocker, Blocker::Ticket(_)) && self.list != TicketList::Backlog {
                return Err(ValidationError::new(
                    "list",
                    "ticket blockers require Backlog",
                ));
            }
        }
        Ok(())
    }
}
/// Validated proposed work or a reason not to file work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawProposal")]
pub struct Proposal {
    /// The proposed outcome.
    pub kind: ProposalKind,
    /// One to 20 tickets for tickets, otherwise empty.
    pub tickets: Vec<Ticket>,
    /// A nonempty explanation of at most 4,096 bytes.
    pub reason: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawProposal {
    kind: ProposalKind,
    tickets: Vec<Ticket>,
    reason: String,
}
impl TryFrom<RawProposal> for Proposal {
    type Error = ValidationError;
    fn try_from(raw: RawProposal) -> Result<Self, Self::Error> {
        let value = Self {
            kind: raw.kind,
            tickets: raw.tickets,
            reason: raw.reason,
        };
        value.validate()?;
        Ok(value)
    }
}
impl Proposal {
    /// Checks every field against the contract.
    pub fn validate(&self) -> Result<(), ValidationError> {
        check_block("reason", &self.reason, 1, 4_096)?;
        let valid_count = match self.kind {
            ProposalKind::Tickets => (1..=20).contains(&self.tickets.len()),
            ProposalKind::NoAction | ProposalKind::Question => self.tickets.is_empty(),
        };
        if !valid_count {
            return Err(ValidationError::new(
                "tickets",
                "ticket count does not match proposal kind",
            ));
        }
        for (index, ticket) in self.tickets.iter().enumerate() {
            ticket
                .validate()
                .map_err(|error| error.prefix(&format!("tickets[{index}]")))?;
            for (blocker_index, blocker) in ticket.blocked_by.iter().enumerate() {
                if let Blocker::Ticket(earlier) = blocker
                    && u64::from(*earlier) >= index as u64
                {
                    return Err(ValidationError::new(
                        format!("tickets[{index}].blockedBy[{blocker_index}]"),
                        "ticket blocker must refer to an earlier ticket",
                    ));
                }
            }
        }
        Ok(())
    }
}
