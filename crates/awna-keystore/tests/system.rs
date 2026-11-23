#[cfg(target_os = "macos")]
#[test]
fn system_constructs_without_touching_the_keychain() {
    assert!(awna_keystore::system().is_ok());
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
#[test]
fn unsupported_unix_reports_no_store() {
    assert!(matches!(
        awna_keystore::system(),
        Err(awna_keystore::Error::NoStore)
    ));
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "writes to this computer's keychain; run by hand"]
fn live_system_round_trip() {
    use awna_keystore::{Item, MAX_SECRET, Secret, system};
    let store = system().unwrap();
    let item = Item::new(
        "awna-keystore.live-test",
        "awna-keystore",
        "awna-keystore live test",
    )
    .unwrap();
    // Attempt cleanup even if an assertion fails after writing.
    struct Cleanup<'a>(&'a dyn awna_keystore::CredentialStore, &'a Item);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            let _ = self.0.delete(self.1);
        }
    }
    let _cleanup = Cleanup(&*store, &item);
    store.delete(&item).unwrap();
    store
        .store(&item, &Secret::new("invented-live-value".into()))
        .unwrap();
    assert_eq!(
        store.find(&item).unwrap().unwrap().expose(),
        "invented-live-value"
    );
    assert!(store.exists(&item).unwrap());
    store.delete(&item).unwrap();
    assert!(!store.exists(&item).unwrap());
    let maximum = Secret::new("a".repeat(MAX_SECRET));
    store.store(&item, &maximum).unwrap();
    assert_eq!(
        store.find(&item).unwrap().unwrap().expose(),
        maximum.expose()
    );
    store.delete(&item).unwrap();
    assert!(!store.exists(&item).unwrap());
}
