use crate::ValidationError;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

fn forbidden(character: char, block: bool) -> bool {
    matches!(character, '\u{0}'..='\u{1f}') && !(block && matches!(character, '\n' | '\t'))
        || matches!(character, '\u{7f}'..='\u{9f}' | '\u{2028}' | '\u{2029}' | '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

pub(crate) fn check_line(field: &str, value: &str, max: usize) -> Result<(), ValidationError> {
    if value.is_empty() || value.len() > max {
        return Err(ValidationError::new(field, "line length is out of range"));
    }
    if value.trim() != value {
        return Err(ValidationError::new(
            field,
            "line has surrounding whitespace",
        ));
    }
    if value.chars().any(|character| forbidden(character, false)) {
        return Err(ValidationError::new(
            field,
            "text contains a forbidden character",
        ));
    }
    Ok(())
}

pub(crate) fn check_block(
    field: &str,
    value: &str,
    min: usize,
    max: usize,
) -> Result<(), ValidationError> {
    if value.len() < min || value.len() > max {
        return Err(ValidationError::new(field, "block length is out of range"));
    }
    if value.chars().any(|character| forbidden(character, true)) {
        return Err(ValidationError::new(
            field,
            "text contains a forbidden character",
        ));
    }
    Ok(())
}

pub(crate) fn check_slug(field: &str, value: &str) -> Result<(), ValidationError> {
    if value.is_empty()
        || value.len() > 32
        || !value.as_bytes()[0].is_ascii_lowercase()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || value.ends_with('-')
        || value.contains("--")
    {
        return Err(ValidationError::new(field, "invalid slug"));
    }
    Ok(())
}

fn check_event_kind(field: &str, value: &str) -> Result<(), ValidationError> {
    check_slug(field, value)?;
    if value == "status" {
        return Err(ValidationError::new(field, "status is reserved"));
    }
    Ok(())
}
fn check_attachment_name(field: &str, value: &str) -> Result<(), ValidationError> {
    check_line(field, value, 255)?;
    if value.starts_with('.') || value.contains(['/', '\\', ':']) {
        return Err(ValidationError::new(field, "invalid attachment name"));
    }
    let extension = value
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .unwrap_or("");
    if extension.is_empty()
        || extension.len() > 16
        || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(ValidationError::new(field, "invalid attachment extension"));
    }
    Ok(())
}

macro_rules! string_type {
    ($name:ident, $field:literal, $description:literal, $check:expr) => {
        #[doc = $description]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);
        impl $name {
            /// Builds a value after checking its contract rules.
            pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
                let value = value.into();
                ($check)($field, &value)?;
                Ok(Self(value))
            }
            /// Borrows the validated text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl FromStr for $name {
            type Err = ValidationError;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }
        impl TryFrom<String> for $name {
            type Error = ValidationError;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}
string_type!(Source, "source", "An opaque source slug.", check_slug);
string_type!(
    RequesterKey,
    "requesterKey",
    "An opaque requester key, at most 320 bytes.",
    |field, value: &str| check_line(field, value, 320)
);
string_type!(
    Reference,
    "ref",
    "An opaque external reference, at most 500 bytes.",
    |field, value: &str| check_line(field, value, 500)
);
string_type!(
    EventKind,
    "kind",
    "A note kind slug, excluding the reserved status kind.",
    check_event_kind
);
string_type!(
    AttachmentName,
    "name",
    "An attachment basename with a safe extension, at most 255 bytes.",
    check_attachment_name
);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forbidden_character_edges_are_rejected() {
        for character in [
            '\u{1f}', '\u{7f}', '\u{80}', '\u{9f}', '\u{2028}', '\u{2029}', '\u{202a}', '\u{202e}',
            '\u{2066}', '\u{2069}', '\u{61c}', '\u{200e}', '\u{200f}',
        ] {
            let value = format!("a{character}b");
            assert!(check_line("line", &value, 100).is_err(), "{character:?}");
            assert!(
                check_block("block", &value, 0, 100).is_err(),
                "{character:?}"
            );
        }
    }
    #[test]
    fn lines_refuse_whitespace_and_blocks_allow_only_newline_and_tab() {
        for value in ["", " x", "x ", "\u{a0}x", "x\u{a0}", "a\nb", "a\tb", "a\rb"] {
            assert!(check_line("line", value, 100).is_err(), "{value:?}");
        }
        assert!(check_block("block", "a\nb\tc", 0, 100).is_ok());
        assert!(check_block("block", "a\rb", 0, 100).is_err());
        assert!(check_block("block", "", 0, 100).is_ok());
        assert!(check_block("block", "", 1, 100).is_err());
    }
    #[test]
    fn lengths_count_utf8_bytes() {
        assert!(check_line("line", "éé", 4).is_ok());
        assert!(check_line("line", "ééx", 4).is_err());
        assert!(check_block("block", "éé", 4, 4).is_ok());
        assert!(check_block("block", "ééx", 0, 4).is_err());
        assert!(check_block("block", "é", 3, 4).is_err());
    }
    #[test]
    fn slugs_have_the_fixed_lowercase_shape() {
        for value in ["", "A", "1a", "a-", "a--b", "a_b", "a.b", "é", "a b"] {
            assert!(Source::new(value).is_err(), "{value:?}");
        }
        assert!(Source::new("a1-b").is_ok());
        assert!(Source::new("a".repeat(32)).is_ok());
        assert!(Source::new("a".repeat(33)).is_err());
        assert!(EventKind::new("status").is_err());
        assert!(EventKind::new("comment").is_ok());
    }
    #[test]
    fn attachment_names_have_safe_basenames_and_extensions() {
        for value in [
            ".",
            "..",
            ".a.txt",
            "a/b.txt",
            "a\\b.txt",
            "a:b.txt",
            "a",
            "a.",
            "a.long-extension",
            "a.é",
            "a.txt ",
        ] {
            assert!(AttachmentName::new(value).is_err(), "{value:?}");
        }
        assert!(AttachmentName::new("a.b.TXT").is_ok());
        assert!(AttachmentName::new(format!("{}.txt", "a".repeat(251))).is_ok());
        assert!(AttachmentName::new(format!("{}.txt", "a".repeat(252))).is_err());
        assert!(AttachmentName::new(format!("a.{}", "a".repeat(16))).is_ok());
        assert!(AttachmentName::new(format!("a.{}", "a".repeat(17))).is_err());
    }
    #[test]
    fn string_newtypes_enforce_their_exact_byte_caps() {
        assert!(RequesterKey::new("é".repeat(160)).is_ok());
        assert!(RequesterKey::new(format!("{}x", "é".repeat(160))).is_err());
        assert!(Reference::new("é".repeat(250)).is_ok());
        assert!(Reference::new(format!("{}x", "é".repeat(250))).is_err());
    }
}
