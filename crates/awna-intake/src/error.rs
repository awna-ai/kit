use std::fmt;

/// A field that failed validation, without reflecting its untrusted value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// The JSON path to the invalid field.
    pub field: String,
    /// A fixed explanation that never includes the offending value.
    pub reason: &'static str,
}

impl ValidationError {
    pub(crate) fn new(field: impl Into<String>, reason: &'static str) -> Self {
        Self {
            field: field.into(),
            reason,
        }
    }

    pub(crate) fn prefix(mut self, prefix: &str) -> Self {
        self.field = format!("{prefix}.{}", self.field);
        self
    }
}
impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.reason)
    }
}
impl std::error::Error for ValidationError {}
