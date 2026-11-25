use crate::attachment::{check_screenshot, check_unique_name};
pub use crate::text::{EventKind, Reference, RequesterKey, Source};
use crate::{
    Attachment, Proposal, Role, Status, Triage, ValidationError,
    text::{check_block, check_line, forbidden},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fmt, str::FromStr};

/// A positive intake item number, displayed as `IN-<n>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ItemId(u64);
impl ItemId {
    /// Builds an id, refusing zero.
    pub fn new(value: u64) -> Option<Self> {
        (value > 0).then_some(Self(value))
    }
    /// Returns the numeric part.
    pub fn get(self) -> u64 {
        self.0
    }
}
impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IN-{}", self.0)
    }
}
impl FromStr for ItemId {
    type Err = ValidationError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = || ValidationError::new("id", "invalid item id");
        let digits = value.strip_prefix("IN-").ok_or_else(invalid)?;
        if digits.is_empty()
            || digits.starts_with('0')
            || !digits.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(invalid());
        }
        let number = digits.parse::<u64>().map_err(|_| invalid())?;
        Self::new(number).ok_or_else(invalid)
    }
}
impl TryFrom<String> for ItemId {
    type Error = ValidationError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}
impl From<ItemId> for String {
    fn from(value: ItemId) -> Self {
        value.to_string()
    }
}

/// A bounded JSON snapshot of the sender state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawSnapshot")]
pub struct Snapshot {
    /// Any JSON, at most 1,048,576 compact bytes and 32 array/object levels.
    pub state: serde_json::Value,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawSnapshot {
    state: serde_json::Value,
}
impl TryFrom<RawSnapshot> for Snapshot {
    type Error = ValidationError;
    fn try_from(raw: RawSnapshot) -> Result<Self, Self::Error> {
        let value = Self { state: raw.state };
        value.validate()?;
        Ok(value)
    }
}
impl Snapshot {
    /// Replaces forbidden characters in state strings and keys with U+FFFD.
    /// On a collision, keeps the key that sorts first; size and nesting still need validation.
    pub fn lossy(mut state: serde_json::Value) -> Self {
        fn replace(text: &str) -> String {
            text.chars()
                .map(|c| if forbidden(c, true) { '\u{fffd}' } else { c })
                .collect()
        }
        let mut pending = vec![&mut state];
        while let Some(value) = pending.pop() {
            match value {
                serde_json::Value::String(text) => *text = replace(text),
                serde_json::Value::Array(values) => pending.extend(values.iter_mut()),
                serde_json::Value::Object(values) => {
                    let original = std::mem::take(values);
                    for (key, value) in original {
                        values.entry(replace(&key)).or_insert(value);
                    }
                    pending.extend(values.values_mut());
                }
                _ => {}
            }
        }
        Self { state }
    }

    /// Checks every field against the contract.
    pub fn validate(&self) -> Result<(), ValidationError> {
        let mut pending = vec![(&self.state, 0_usize)];
        while let Some((value, parents)) = pending.pop() {
            match value {
                serde_json::Value::Array(values) => {
                    let depth = parents + 1;
                    if depth > 32 {
                        return Err(ValidationError::new(
                            "state",
                            "snapshot nesting exceeds limit",
                        ));
                    }
                    pending.extend(values.iter().map(|value| (value, depth)));
                }
                serde_json::Value::Object(values) => {
                    let depth = parents + 1;
                    if depth > 32 {
                        return Err(ValidationError::new(
                            "state",
                            "snapshot nesting exceeds limit",
                        ));
                    }
                    for key in values.keys() {
                        check_snapshot_text(key)?;
                    }
                    pending.extend(values.values().map(|value| (value, depth)));
                }
                serde_json::Value::String(text) => check_snapshot_text(text)?,
                _ => {}
            }
        }
        let bytes = serde_json::to_vec(&self.state)
            .map_err(|_| ValidationError::new("state", "snapshot cannot be serialized"))?;
        if bytes.len() > 1_048_576 {
            return Err(ValidationError::new("state", "snapshot size exceeds limit"));
        }
        Ok(())
    }
}
fn check_snapshot_text(text: &str) -> Result<(), ValidationError> {
    if text.chars().any(|c| forbidden(c, true)) {
        return Err(ValidationError::new(
            "state",
            "snapshot text contains a forbidden character",
        ));
    }
    Ok(())
}

/// A new request before the store assigns its id, status and times.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawNewItem")]
pub struct NewItem {
    /// The opaque source slug.
    pub source: Source,
    /// The opaque requester key.
    #[serde(rename = "requesterKey")]
    pub requester: RequesterKey,
    /// A line of at most 200 bytes.
    pub title: String,
    /// A block of at most 65,536 bytes.
    pub body: String,
    /// Optional bounded sender state.
    #[serde(default)]
    pub snapshot: Option<Snapshot>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawNewItem {
    source: Source,
    #[serde(rename = "requesterKey")]
    requester: RequesterKey,
    title: String,
    body: String,
    #[serde(default)]
    snapshot: Option<Snapshot>,
}
impl TryFrom<RawNewItem> for NewItem {
    type Error = ValidationError;
    fn try_from(raw: RawNewItem) -> Result<Self, Self::Error> {
        let value = Self {
            source: raw.source,
            requester: raw.requester,
            title: raw.title,
            body: raw.body,
            snapshot: raw.snapshot,
        };
        value.validate()?;
        Ok(value)
    }
}
impl NewItem {
    /// Maximum untrusted JSON size in UTF-8 bytes.
    pub const MAX_JSON_BYTES: usize = 1_310_720;

    /// Parses untrusted JSON without reflecting its contents in errors.
    pub fn from_json(text: &str) -> Result<Self, ValidationError> {
        if text.len() > Self::MAX_JSON_BYTES {
            return Err(ValidationError::new("newItem", "input is too large"));
        }
        serde_json::from_str(text).map_err(|_| ValidationError::new("newItem", "not a valid item"))
    }

    /// Checks every field against the contract.
    pub fn validate(&self) -> Result<(), ValidationError> {
        check_line("title", &self.title, 200)?;
        check_block("body", &self.body, 0, 65_536)?;
        if let Some(snapshot) = &self.snapshot {
            snapshot
                .validate()
                .map_err(|error| error.prefix("snapshot"))?;
        }
        Ok(())
    }
}
/// A note the store timestamps when appended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawNote")]
pub struct Note {
    /// An event slug other than status.
    pub kind: EventKind,
    /// A block of at most 16,384 bytes.
    pub body: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawNote {
    kind: EventKind,
    body: String,
}
impl TryFrom<RawNote> for Note {
    type Error = ValidationError;
    fn try_from(raw: RawNote) -> Result<Self, Self::Error> {
        let value = Self {
            kind: raw.kind,
            body: raw.body,
        };
        value.validate()?;
        Ok(value)
    }
}
impl Note {
    /// Checks every field against the contract.
    pub fn validate(&self) -> Result<(), ValidationError> {
        check_block("body", &self.body, 0, 16_384)
    }
}
/// A timestamped change or note in the item history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    try_from = "RawEvent"
)]
pub enum Event {
    /// A status change, including the initial submission event.
    Status {
        /// Unix milliseconds for this event.
        at_ms: u64,
        /// The prior status, absent only on the initial event.
        from: Option<Status>,
        /// The new status.
        to: Status,
        /// An optional external reference.
        #[serde(rename = "ref")]
        reference: Option<Reference>,
    },
    /// A caller note that leaves status unchanged.
    Note {
        /// Unix milliseconds for this event.
        at_ms: u64,
        /// A note kind other than status.
        kind: EventKind,
        /// A block of at most 16,384 bytes.
        body: String,
    },
}
#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum RawEvent {
    Status {
        at_ms: u64,
        from: Option<Status>,
        to: Status,
        #[serde(rename = "ref")]
        reference: Option<Reference>,
    },
    Note {
        at_ms: u64,
        kind: EventKind,
        body: String,
    },
}
impl TryFrom<RawEvent> for Event {
    type Error = ValidationError;
    fn try_from(raw: RawEvent) -> Result<Self, Self::Error> {
        let event = match raw {
            RawEvent::Status {
                at_ms,
                from,
                to,
                reference,
            } => Self::Status {
                at_ms,
                from,
                to,
                reference,
            },
            RawEvent::Note { at_ms, kind, body } => Self::Note { at_ms, kind, body },
        };
        event.validate()?;
        Ok(event)
    }
}
impl Event {
    /// Checks a note body or a status event's individual move.
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Note { body, .. } => check_block("body", body, 0, 16_384),
            Self::Status {
                from: None,
                to: Status::Submitted,
                ..
            } => Ok(()),
            Self::Status {
                from: Some(from),
                to,
                ..
            } if from.can_move_to(*to) => Ok(()),
            Self::Status { .. } => Err(ValidationError::new("to", "illegal status event")),
        }
    }
    /// Returns the event's Unix millisecond timestamp.
    pub fn at_ms(&self) -> u64 {
        match self {
            Self::Status { at_ms, .. } | Self::Note { at_ms, .. } => *at_ms,
        }
    }
}
/// A complete stored request and its validated history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawItem")]
pub struct Item {
    /// The store-assigned positive id.
    pub id: ItemId,
    /// The opaque source slug.
    pub source: Source,
    /// The opaque requester key.
    #[serde(rename = "requesterKey")]
    pub requester: RequesterKey,
    /// A line of at most 200 bytes.
    pub title: String,
    /// A block of at most 65,536 bytes.
    pub body: String,
    /// Optional bounded sender state.
    #[serde(default)]
    pub snapshot: Option<Snapshot>,
    /// The attachment metadata in submission order.
    pub attachments: Vec<Attachment>,
    /// The stored triage, absent before processing.
    pub triage: Option<Triage>,
    /// The stored proposal, absent before processing.
    pub proposal: Option<Proposal>,
    /// The latest status.
    pub status: Status,
    /// A chronological history of 1 to 256 events.
    pub events: Vec<Event>,
    /// The first event time in Unix milliseconds.
    pub created_at_ms: u64,
    /// The most recent change time in Unix milliseconds.
    pub updated_at_ms: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawItem {
    id: ItemId,
    source: Source,
    #[serde(rename = "requesterKey")]
    requester: RequesterKey,
    title: String,
    body: String,
    #[serde(default)]
    snapshot: Option<Snapshot>,
    attachments: Vec<Attachment>,
    triage: Option<Triage>,
    proposal: Option<Proposal>,
    status: Status,
    events: Vec<Event>,
    created_at_ms: u64,
    updated_at_ms: u64,
}
impl TryFrom<RawItem> for Item {
    type Error = ValidationError;
    fn try_from(raw: RawItem) -> Result<Self, Self::Error> {
        let value = Self {
            id: raw.id,
            source: raw.source,
            requester: raw.requester,
            title: raw.title,
            body: raw.body,
            snapshot: raw.snapshot,
            attachments: raw.attachments,
            triage: raw.triage,
            proposal: raw.proposal,
            status: raw.status,
            events: raw.events,
            created_at_ms: raw.created_at_ms,
            updated_at_ms: raw.updated_at_ms,
        };
        value.validate()?;
        Ok(value)
    }
}
impl Item {
    /// Maximum untrusted JSON size in UTF-8 bytes.
    pub const MAX_JSON_BYTES: usize = 12_582_912;

    /// Parses untrusted JSON without reflecting its contents in errors.
    pub fn from_json(text: &str) -> Result<Self, ValidationError> {
        if text.len() > Self::MAX_JSON_BYTES {
            return Err(ValidationError::new("item", "input is too large"));
        }
        serde_json::from_str(text).map_err(|_| ValidationError::new("item", "not a valid item"))
    }

    /// Checks every field against the contract.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.events.len() > 256 {
            return Err(ValidationError::new("events", "event history is full"));
        }
        check_line("title", &self.title, 200)?;
        check_block("body", &self.body, 0, 65_536)?;
        if let Some(snapshot) = &self.snapshot {
            snapshot
                .validate()
                .map_err(|error| error.prefix("snapshot"))?;
        }
        if self.triage.is_some() != self.proposal.is_some() {
            return Err(ValidationError::new(
                "proposal",
                "triage and proposal must be present together",
            ));
        }
        if self.status == Status::Submitted && self.triage.is_some() {
            return Err(ValidationError::new(
                "triage",
                "submitted item cannot have processing results",
            ));
        }
        let was_processed = self.events.iter().any(|event| {
            matches!(
                event,
                Event::Status {
                    to: Status::Processed,
                    ..
                }
            )
        });
        if self.triage.is_some() != was_processed {
            return Err(ValidationError::new(
                "triage",
                "processing results must match processed status history",
            ));
        }
        if let Some(triage) = &self.triage {
            triage.validate().map_err(|error| error.prefix("triage"))?;
        }
        if let Some(proposal) = &self.proposal {
            proposal
                .validate()
                .map_err(|error| error.prefix("proposal"))?;
        }
        if self.attachments.len() > 1000 {
            return Err(ValidationError::new("attachments", "too many attachments"));
        }
        let mut total = 0_u64;
        let mut names = HashSet::new();
        let mut screenshots = 0;
        for (index, attachment) in self.attachments.iter().enumerate() {
            attachment
                .validate()
                .map_err(|error| error.prefix(&format!("attachments[{index}]")))?;
            check_unique_name(
                &mut names,
                &format!("attachments[{index}].name"),
                &attachment.name,
            )?;
            check_screenshot(
                &format!("attachments[{index}].role"),
                attachment.role,
                &attachment.content_type,
            )?;
            total = total.checked_add(attachment.size).ok_or_else(|| {
                ValidationError::new("attachments", "total attachment size overflows")
            })?;
            if attachment.index as u64 != index as u64 {
                return Err(ValidationError::new(
                    format!("attachments[{index}].index"),
                    "attachment indices are not consecutive",
                ));
            }
            if attachment.role == Role::Screenshot {
                screenshots += 1;
            }
        }
        if screenshots > 1 {
            return Err(ValidationError::new(
                "attachments",
                "more than one screenshot",
            ));
        }
        let Some(first) = self.events.first() else {
            return Err(ValidationError::new("events", "item has no events"));
        };
        if !matches!(
            first,
            Event::Status {
                from: None,
                to: Status::Submitted,
                ..
            }
        ) {
            return Err(ValidationError::new(
                "events[0]",
                "first event must record submission",
            ));
        }
        let mut previous_status = Status::Submitted;
        let mut previous_time = first.at_ms();
        for (index, event) in self.events.iter().enumerate() {
            event
                .validate()
                .map_err(|error| error.prefix(&format!("events[{index}]")))?;
            if event.at_ms() < previous_time {
                return Err(ValidationError::new(
                    format!("events[{index}].atMs"),
                    "event times decrease",
                ));
            }
            previous_time = event.at_ms();
            if index > 0
                && let Event::Status { from, to, .. } = event
            {
                if *from != Some(previous_status) || !previous_status.can_move_to(*to) {
                    return Err(ValidationError::new(
                        format!("events[{index}]"),
                        "status history is not continuous",
                    ));
                }
                previous_status = *to;
            }
        }
        if previous_status != self.status {
            return Err(ValidationError::new(
                "status",
                "status differs from last status event",
            ));
        }
        if self.created_at_ms != first.at_ms() {
            return Err(ValidationError::new(
                "createdAtMs",
                "creation time differs from first event",
            ));
        }
        if self.updated_at_ms < previous_time {
            return Err(ValidationError::new(
                "updatedAtMs",
                "update time precedes last event",
            ));
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_are_canonical_positive_decimal_strings() {
        for value in [
            "IN-0",
            "IN-01",
            "IN-+1",
            "in-1",
            "IN-18446744073709551616",
            "IN-",
            "IN- 1",
            "IN--1",
        ] {
            assert!(value.parse::<ItemId>().is_err(), "{value}");
        }
        for value in ["IN-1", "IN-18446744073709551615"] {
            assert_eq!(value.parse::<ItemId>().unwrap().to_string(), value);
        }
        assert!(ItemId::new(0).is_none());
        assert_eq!(ItemId::new(1).unwrap().get(), 1);
    }
}
