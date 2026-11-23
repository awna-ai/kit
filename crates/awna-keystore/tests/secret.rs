use awna_keystore::{MAX_SECRET, Secret, storable};

#[test]
fn debug_redacts_the_value() {
    let secret = Secret::new("invented-token".into());
    assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
    assert!(!format!("{secret:?}").contains(secret.expose()));
    let wrapped = zeroize::Zeroizing::new("other-token".to_owned());
    assert_eq!(Secret::from(wrapped).expose(), "other-token");
}

#[test]
fn storable_accepts_only_bounded_token_bytes() {
    for value in ["aZ09", ".", "_", "~", ":", "+", "/", "=", "-"] {
        assert!(storable(&Secret::new(value.into()), MAX_SECRET));
    }
    assert!(storable(&Secret::new("a".repeat(MAX_SECRET)), MAX_SECRET));
    assert!(storable(&Secret::new("abc".into()), 3));
    for value in ["", " ", "\"", "'", "\\", "$", "\n", "\0", "é"] {
        assert!(!storable(&Secret::new(value.into()), MAX_SECRET));
    }
    assert!(!storable(
        &Secret::new("a".repeat(MAX_SECRET + 1)),
        usize::MAX
    ));
    assert!(!storable(&Secret::new("abcd".into()), 3));
    assert!(!storable(&Secret::new("a".into()), 0));
}
