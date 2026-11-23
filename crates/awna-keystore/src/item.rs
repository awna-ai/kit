use crate::Error;

/// A credential's service, account, and human-readable label.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Item {
    service: String,
    account: String,
    label: String,
}
impl Item {
    /// Validates service and account as 1–64 bytes of `a-z`, `0-9`, `.` or `-`.
    /// Labels are 1–64 bytes of ASCII letters, digits, spaces or `. , : ( ) _ -`,
    /// without a leading or trailing space. Returns [`Error::InvalidItem`] otherwise.
    pub fn new(service: &str, account: &str, label: &str) -> Result<Item, Error> {
        let identifier = |s: &str| {
            (1..=64).contains(&s.len())
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b".-".contains(&b))
        };
        let valid_label = (1..=64).contains(&label.len())
            && !label.starts_with(' ')
            && !label.ends_with(' ')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b" .,:()_-".contains(&b));
        if !identifier(service) || !identifier(account) || !valid_label {
            return Err(Error::InvalidItem);
        }
        Ok(Item {
            service: service.into(),
            account: account.into(),
            label: label.into(),
        })
    }
    /// The service under which the credential is filed.
    pub fn service(&self) -> &str {
        &self.service
    }
    /// The account under which the credential is filed.
    pub fn account(&self) -> &str {
        &self.account
    }
    /// The label shown in the password manager.
    pub fn label(&self) -> &str {
        &self.label
    }
}
