pub use crate::text::AttachmentName;
use crate::{ValidationError, text::check_line};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fmt};

/// The purpose of an attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// An ordinary submitted file.
    File,
    /// The single screenshot submitted with an item.
    Screenshot,
}

pub(crate) fn check_content_type(field: &str, value: &str) -> Result<(), ValidationError> {
    check_line(field, value, 127)?;
    let valid_part = |part: &str| {
        !part.is_empty()
            && part.as_bytes()[0].is_ascii_alphanumeric()
            && part.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".+-".contains(&byte)
            })
            && !part.as_bytes()[0].is_ascii_uppercase()
    };
    if !value
        .split_once('/')
        .is_some_and(|(kind, subtype)| valid_part(kind) && valid_part(subtype))
    {
        return Err(ValidationError::new(field, "invalid content type"));
    }
    Ok(())
}
pub(crate) fn check_screenshot(
    field: &str,
    role: Role,
    content_type: &str,
) -> Result<(), ValidationError> {
    if role == Role::Screenshot && !matches!(content_type, "image/png" | "image/jpeg") {
        return Err(ValidationError::new(
            field,
            "screenshot must be PNG or JPEG",
        ));
    }
    Ok(())
}

pub(crate) fn check_unique_name(
    names: &mut HashSet<String>,
    field: &str,
    name: &AttachmentName,
) -> Result<(), ValidationError> {
    if !names.insert(name.as_str().to_ascii_lowercase()) {
        return Err(ValidationError::new(field, "duplicate attachment name"));
    }
    Ok(())
}

/// Stored attachment metadata; the bytes are held separately.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawAttachment")]
pub struct Attachment {
    /// Its zero-based submission index.
    pub index: u32,
    /// The submitted basename.
    pub name: AttachmentName,
    /// A lower-case content type, at most 127 bytes.
    pub content_type: String,
    /// Its purpose.
    pub role: Role,
    /// Its nonzero byte size.
    pub size: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawAttachment {
    index: u32,
    name: AttachmentName,
    content_type: String,
    role: Role,
    size: u64,
}
impl TryFrom<RawAttachment> for Attachment {
    type Error = ValidationError;
    fn try_from(raw: RawAttachment) -> Result<Self, Self::Error> {
        let value = Self {
            index: raw.index,
            name: raw.name,
            content_type: raw.content_type,
            role: raw.role,
            size: raw.size,
        };
        value.validate()?;
        Ok(value)
    }
}
impl Attachment {
    /// Checks every field against the contract.
    pub fn validate(&self) -> Result<(), ValidationError> {
        check_content_type("contentType", &self.content_type)?;
        if self.size == 0 {
            return Err(ValidationError::new("size", "file is empty"));
        }
        Ok(())
    }
}
/// A submitted file borrowed by the store, without any I/O.
#[derive(Clone)]
pub struct NewFile<'a> {
    /// The submitted basename.
    pub name: AttachmentName,
    /// The exact content type to match against the allowlist.
    pub content_type: String,
    /// Its purpose.
    pub role: Role,
    /// The submitted bytes.
    pub bytes: &'a [u8],
}
impl fmt::Debug for NewFile<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewFile")
            .field("name", &self.name)
            .field("content_type", &self.content_type)
            .field("role", &self.role)
            .field("byte_length", &self.bytes.len())
            .finish()
    }
}
impl NewFile<'_> {
    /// Checks the metadata and refuses an empty file.
    pub fn validate(&self) -> Result<(), ValidationError> {
        check_content_type("contentType", &self.content_type)?;
        if self.bytes.is_empty() {
            return Err(ValidationError::new("bytes", "file is empty"));
        }
        Ok(())
    }
}
/// A lower-case content type and its lower-case filename extensions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawAllowedType")]
pub struct AllowedType {
    /// The exact lower-case content type.
    pub content_type: String,
    /// The allowed lower-case ASCII alphanumeric extensions.
    pub extensions: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawAllowedType {
    content_type: String,
    extensions: Vec<String>,
}
impl TryFrom<RawAllowedType> for AllowedType {
    type Error = ValidationError;
    fn try_from(raw: RawAllowedType) -> Result<Self, Self::Error> {
        let value = Self {
            content_type: raw.content_type,
            extensions: raw.extensions,
        };
        value.validate()?;
        Ok(value)
    }
}
impl AllowedType {
    /// Checks every field against the contract.
    pub fn validate(&self) -> Result<(), ValidationError> {
        check_content_type("contentType", &self.content_type)?;
        for (index, extension) in self.extensions.iter().enumerate() {
            if extension.is_empty()
                || extension.len() > 16
                || !extension
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            {
                return Err(ValidationError::new(
                    format!("extensions[{index}]"),
                    "invalid extension",
                ));
            }
        }
        Ok(())
    }
}
/// Attachment count, size and content-type limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawLimits")]
pub struct Limits {
    /// Maximum attachment count, including the screenshot.
    pub max_files: u32,
    /// Maximum bytes per file.
    pub max_file_bytes: u64,
    /// Maximum total bytes per item.
    pub max_item_bytes: u64,
    /// The exact content-type and extension allowlist.
    pub allowed: Vec<AllowedType>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawLimits {
    max_files: u32,
    max_file_bytes: u64,
    max_item_bytes: u64,
    allowed: Vec<AllowedType>,
}
impl TryFrom<RawLimits> for Limits {
    type Error = ValidationError;
    fn try_from(raw: RawLimits) -> Result<Self, Self::Error> {
        let value = Self {
            max_files: raw.max_files,
            max_file_bytes: raw.max_file_bytes,
            max_item_bytes: raw.max_item_bytes,
            allowed: raw.allowed,
        };
        value.validate()?;
        Ok(value)
    }
}
impl Limits {
    /// Checks every field against the contract.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.max_files == 0 {
            return Err(ValidationError::new(
                "maxFiles",
                "file count must be positive",
            ));
        }
        if self.max_file_bytes == 0 {
            return Err(ValidationError::new(
                "maxFileBytes",
                "file size limit must be positive",
            ));
        }
        if self.max_item_bytes < self.max_file_bytes {
            return Err(ValidationError::new(
                "maxItemBytes",
                "item limit is below file limit",
            ));
        }
        let mut types = HashSet::new();
        for (index, allowed) in self.allowed.iter().enumerate() {
            if !types.insert(&allowed.content_type) {
                return Err(ValidationError::new(
                    format!("allowed[{index}].contentType"),
                    "duplicate content type",
                ));
            }
            allowed
                .validate()
                .map_err(|error| error.prefix(&format!("allowed[{index}]")))?;
        }
        Ok(())
    }
}
impl Default for Limits {
    fn default() -> Self {
        let allowed = [
            ("image/png", &["png"][..]),
            ("image/jpeg", &["jpg", "jpeg"][..]),
            ("image/gif", &["gif"][..]),
            ("image/webp", &["webp"][..]),
            ("image/heic", &["heic"][..]),
            ("application/pdf", &["pdf"][..]),
            ("text/plain", &["txt", "log"][..]),
            ("text/markdown", &["md"][..]),
            ("text/csv", &["csv"][..]),
            ("application/json", &["json"][..]),
            (
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                &["docx"][..],
            ),
            (
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                &["xlsx"][..],
            ),
            (
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
                &["pptx"][..],
            ),
        ]
        .into_iter()
        .map(|(content_type, extensions)| AllowedType {
            content_type: content_type.into(),
            extensions: extensions
                .iter()
                .map(|extension| (*extension).into())
                .collect(),
        })
        .collect();
        Self {
            max_files: 20,
            max_file_bytes: 104_857_600,
            max_item_bytes: 262_144_000,
            allowed,
        }
    }
}
impl Limits {
    /// Refuses files that exceed count or byte limits or break the content-type, extension or screenshot rules.
    pub fn check(&self, files: &[NewFile<'_>]) -> Result<(), ValidationError> {
        self.validate()?;
        if files.len() as u64 > u64::from(self.max_files) {
            return Err(ValidationError::new("files", "too many files"));
        }
        for (index, file) in files.iter().enumerate() {
            if file.bytes.is_empty() || file.bytes.len() as u64 > self.max_file_bytes {
                return Err(ValidationError::new(
                    format!("files[{index}].bytes"),
                    "file size is out of range",
                ));
            }
        }
        let mut total = 0_u64;
        for file in files {
            total = total
                .checked_add(file.bytes.len() as u64)
                .filter(|sum| *sum <= self.max_item_bytes)
                .ok_or_else(|| ValidationError::new("files", "total file size exceeds limit"))?;
        }
        let types = files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                self.allowed
                    .iter()
                    .find(|allowed| allowed.content_type == file.content_type)
                    .ok_or_else(|| {
                        ValidationError::new(
                            format!("files[{index}].contentType"),
                            "content type is not allowed",
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (index, (file, allowed)) in files.iter().zip(types).enumerate() {
            let extension = file
                .name
                .as_str()
                .rsplit_once('.')
                .map(|(_, extension)| extension.to_ascii_lowercase())
                .unwrap_or_default();
            if !allowed.extensions.contains(&extension) {
                return Err(ValidationError::new(
                    format!("files[{index}].name"),
                    "extension does not match content type",
                ));
            }
        }
        for (index, file) in files.iter().enumerate() {
            check_screenshot(
                &format!("files[{index}].role"),
                file.role,
                &file.content_type,
            )?;
        }
        let mut screenshot_seen = false;
        for (index, file) in files.iter().enumerate() {
            if file.role == Role::Screenshot {
                if screenshot_seen {
                    return Err(ValidationError::new(
                        format!("files[{index}].role"),
                        "more than one screenshot",
                    ));
                }
                screenshot_seen = true;
            }
        }
        let mut names = HashSet::new();
        for (index, file) in files.iter().enumerate() {
            check_unique_name(&mut names, &format!("files[{index}].name"), &file.name)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn limits() -> Limits {
        Limits {
            max_files: 3,
            max_file_bytes: 4,
            max_item_bytes: 6,
            ..Limits::default()
        }
    }
    fn file<'a>(name: &str, content_type: &str, role: Role, bytes: &'a [u8]) -> NewFile<'a> {
        NewFile {
            name: AttachmentName::new(name).unwrap(),
            content_type: content_type.into(),
            role,
            bytes,
        }
    }
    #[test]
    fn count_limit_and_zero_files() {
        let limits = limits();
        let files = ["a.txt", "b.txt", "c.txt", "d.txt"]
            .map(|name| file(name, "text/plain", Role::File, b"a"));
        assert!(limits.check(&[]).is_ok());
        assert!(limits.check(&files[..3]).is_ok());
        assert_eq!(limits.check(&files).unwrap_err().field, "files");
    }
    #[test]
    fn individual_byte_limits_and_empty_file() {
        for bytes in [&b""[..], &b"12345"[..]] {
            assert_eq!(
                limits()
                    .check(&[file("a.txt", "text/plain", Role::File, bytes)])
                    .unwrap_err()
                    .field,
                "files[0].bytes"
            );
        }
        assert!(
            limits()
                .check(&[file("a.txt", "text/plain", Role::File, b"1234")])
                .is_ok()
        );
    }
    #[test]
    fn total_byte_limit() {
        let a = file("a.txt", "text/plain", Role::File, b"1234");
        assert!(
            limits()
                .check(&[a.clone(), file("b.txt", "text/plain", Role::File, b"12")])
                .is_ok()
        );
        assert_eq!(
            limits()
                .check(&[a, file("b.txt", "text/plain", Role::File, b"123")])
                .unwrap_err()
                .field,
            "files"
        );
    }
    #[test]
    fn types_and_extensions_must_match_exactly() {
        for (name, content_type, field) in [
            ("a.svg", "image/svg+xml", "contentType"),
            ("a.html", "text/plain", "name"),
            ("a.txt", "Text/plain", "contentType"),
        ] {
            assert_eq!(
                limits()
                    .check(&[file(name, content_type, Role::File, b"x")])
                    .unwrap_err()
                    .field,
                format!("files[0].{field}")
            );
        }
        assert!(
            limits()
                .check(&[file("a.TXT", "text/plain", Role::File, b"x")])
                .is_ok()
        );
    }
    #[test]
    fn screenshots_are_single_png_or_jpeg() {
        assert!(
            limits()
                .check(&[file("a.pdf", "application/pdf", Role::Screenshot, b"x")])
                .is_err()
        );
        let image = file("a.png", "image/png", Role::Screenshot, b"x");
        assert!(limits().check(std::slice::from_ref(&image)).is_ok());
        assert_eq!(
            limits().check(&[image.clone(), image]).unwrap_err().field,
            "files[1].role"
        );
        assert!(
            limits()
                .check(&[file("a.jpeg", "image/jpeg", Role::Screenshot, b"x")])
                .is_ok()
        );
    }
    #[test]
    fn refusal_categories_are_checked_in_contract_order_across_all_files() {
        let bad_extension = file("a.html", "text/plain", Role::File, b"x");
        let bad_type = file("b.svg", "image/svg+xml", Role::File, b"x");
        assert_eq!(
            limits()
                .check(&[bad_extension.clone(), bad_type])
                .unwrap_err()
                .field,
            "files[1].contentType"
        );
    }
    #[test]
    fn extension_refusals_precede_screenshot_role_refusals_across_all_files() {
        let bad_extension = file("a.html", "text/plain", Role::File, b"x");
        let bad_screenshot = file("a.pdf", "application/pdf", Role::Screenshot, b"x");
        assert_eq!(
            limits()
                .check(&[bad_screenshot, bad_extension])
                .unwrap_err()
                .field,
            "files[1].name"
        );
    }
}
