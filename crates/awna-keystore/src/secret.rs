use std::fmt;
use zeroize::Zeroizing;

/// A credential value, wiped on drop and redacted in debug output.
///
/// ```compile_fail
/// use awna_keystore::Secret;
/// Secret::new(String::new()).clone();
/// ```
///
/// ```compile_fail
/// use awna_keystore::Secret;
/// format!("{}", Secret::new(String::new()));
/// ```
pub struct Secret(Zeroizing<String>);

impl Secret {
    /// Takes ownership of a value without copying it.
    pub fn new(value: String) -> Secret {
        Secret(Zeroizing::new(value))
    }
    /// Borrows the credential's value.
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl From<Zeroizing<String>> for Secret {
    fn from(value: Zeroizing<String>) -> Self {
        Secret(value)
    }
}
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}
/// The longest accepted value, in bytes.
pub const MAX_SECRET: usize = 8192;
/// Whether a value has 1 to `min(max, MAX_SECRET)` bytes, all ASCII letters,
/// digits, or `. _ ~ : + / = -`. Other bytes, including whitespace, are refused.
pub fn storable(value: &Secret, max: usize) -> bool {
    let bytes = value.expose().as_bytes();
    !bytes.is_empty()
        && bytes.len() <= max.min(MAX_SECRET)
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || b"._~:+/=-".contains(b))
}
