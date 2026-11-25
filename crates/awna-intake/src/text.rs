use crate::ValidationError;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

pub(crate) fn forbidden(character: char, block: bool) -> bool {
    matches!(character, '\u{0}'..='\u{1f}') && !(block && matches!(character, '\n' | '\t'))
        || matches!(character, '\u{7f}'..='\u{9f}' | '\u{2028}' | '\u{2029}' | '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{ad}' | '\u{180e}' | '\u{200b}' | '\u{2060}'..='\u{2064}' | '\u{206a}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{e0000}'..='\u{e007f}' | '\u{fdd0}'..='\u{fdef}')
        || (character as u32 & 0xffff) >= 0xfffe
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
    if value.len() < min || value.len() > max || (min > 0 && value.trim().is_empty()) {
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
    check_line(field, value, 200)?;
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    if value.starts_with(['.', '-'])
        || value.ends_with(['.', ' '])
        || device
        || value.contains([
            '/', '\\', ':', '<', '>', '"', '|', '?', '*', '\u{200c}', '\u{200d}',
        ])
    {
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

fn check_ascii(field: &str, value: &str, max: usize, min: u8) -> Result<(), ValidationError> {
    check_line(field, value, max)?;
    if !value.bytes().all(|byte| (min..=0x7e).contains(&byte)) {
        return Err(ValidationError::new(field, "text must be printable ASCII"));
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
    |field, value: &str| check_ascii(field, value, 320, 0x21)
);
string_type!(
    Reference,
    "ref",
    "An opaque external reference, at most 500 bytes.",
    |field, value: &str| check_ascii(field, value, 500, 0x20)
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
    "An attachment basename with a safe extension, at most 200 bytes.",
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
        assert!(AttachmentName::new(format!("{}.txt", "a".repeat(196))).is_ok());
        assert!(AttachmentName::new(format!("{}.txt", "a".repeat(197))).is_err());
        assert!(AttachmentName::new(format!("a.{}", "a".repeat(16))).is_ok());
        assert!(AttachmentName::new(format!("a.{}", "a".repeat(17))).is_err());
    }
    #[test]
    fn string_newtypes_enforce_their_exact_byte_caps() {
        assert!(RequesterKey::new("a".repeat(320)).is_ok());
        assert!(RequesterKey::new(format!("{}x", "a".repeat(320))).is_err());
        assert!(Reference::new("a".repeat(500)).is_ok());
        assert!(Reference::new(format!("{}x", "a".repeat(500))).is_err());
    }
    #[test]
    fn invisible_ranges_and_noncharacters_have_exact_edges() {
        for (start, end) in [
            (0xad, 0xad),
            (0x180e, 0x180e),
            (0x200b, 0x200b),
            (0x2060, 0x2064),
            (0x206a, 0x206f),
            (0xfeff, 0xfeff),
            (0xfff9, 0xfffb),
            (0xe0000, 0xe007f),
            (0xfdd0, 0xfdef),
        ] {
            for point in [start, end] {
                let text = format!("a{}b", char::from_u32(point).unwrap());
                assert!(check_line("title", &text, 200).is_err(), "{point:x}");
                assert!(check_block("body", &text, 0, 200).is_err(), "{point:x}");
            }
            for point in [start - 1, end + 1] {
                // Adjacent bidi and noncharacter code points are also forbidden.
                if point == 0x2069 || point == 0xdffff {
                    continue;
                }
                let text = format!("a{}b", char::from_u32(point).unwrap());
                assert!(check_line("title", &text, 200).is_ok(), "{point:x}");
                assert!(check_block("body", &text, 0, 200).is_ok(), "{point:x}");
            }
        }
        for plane in 0..=16 {
            for low in [0xfffe, 0xffff] {
                let point = (plane << 16) | low;
                let text = format!("a{}b", char::from_u32(point).unwrap());
                assert!(check_line("title", &text, 200).is_err(), "{point:x}");
                assert!(check_block("body", &text, 0, 200).is_err(), "{point:x}");
            }
            let text = format!("a{}b", char::from_u32((plane << 16) | 0xfffd).unwrap());
            assert!(check_line("title", &text, 200).is_ok());
            assert!(check_block("body", &text, 0, 200).is_ok());
        }
        for text in ["👩\u{200d}💻", "a\u{200c}b"] {
            assert!(check_line("title", text, 200).is_ok());
            assert!(check_block("body", text, 0, 200).is_ok());
        }
    }
    #[test]
    fn opaque_keys_and_references_are_printable_ascii() {
        for point in 0..=0x7f {
            let text = format!("a{}b", char::from_u32(point).unwrap());
            assert_eq!(
                RequesterKey::new(&text).is_ok(),
                (0x21..=0x7e).contains(&point)
            );
            assert_eq!(
                Reference::new(&text).is_ok(),
                (0x20..=0x7e).contains(&point)
            );
        }
        assert!(RequesterKey::new("é").is_err());
        assert!(Reference::new("é").is_err());
        assert!(Reference::new("a b").is_ok());
        assert!(Reference::new(" a").is_err());
    }
    #[test]
    fn required_blocks_need_non_whitespace() {
        assert!(check_block("reason", " \n\t", 1, 4096).is_err());
        assert!(check_block("spec", "\u{a0}", 1, 32768).is_err());
        assert!(check_block("body", " \n\t", 0, 65536).is_ok());
    }
    #[test]
    fn attachment_names_are_portable_for_export() {
        for text in [
            "-a.txt",
            "a<.txt",
            "a>.txt",
            "a\".txt",
            "a|.txt",
            "a?.txt",
            "a*.txt",
            "a\u{200c}.txt",
            "a\u{200d}.txt",
            "CON.txt",
            "prn.txt",
            "Aux.a.txt",
            "NUL.txt",
            "a.txt.",
            "a.txt ",
        ] {
            assert!(AttachmentName::new(text).is_err(), "{text:?}");
        }
        for prefix in ["COM", "lpt"] {
            for n in 1..=9 {
                assert!(AttachmentName::new(format!("{prefix}{n}.txt")).is_err());
            }
        }
        for text in [
            "COM0.txt",
            "COM10.txt",
            "LPT0.txt",
            "LPT10.txt",
            "console.txt",
        ] {
            assert!(AttachmentName::new(text).is_ok());
        }
    }
}
