use awna_keystore::{CredentialStore, Error, Item, MemoryStore, Secret};
fn item() -> Item {
    Item::new("awna-keystore.test", "invented", "Invented").unwrap()
}

#[test]
fn missing_items_are_absent_and_deleting_them_succeeds() {
    let store = MemoryStore::new();
    assert!(store.find(&item()).unwrap().is_none());
    assert!(!store.exists(&item()).unwrap());
    assert_eq!(store.delete(&item()), Ok(()));
}

#[test]
fn store_reads_back_replaces_and_deletes_values() {
    let store = MemoryStore::new();
    for value in ["invented-token", "replacement-token"] {
        store.store(&item(), &Secret::new(value.into())).unwrap();
        assert_eq!(store.find(&item()).unwrap().unwrap().expose(), value);
        assert!(store.exists(&item()).unwrap());
        assert!(!format!("{store:?}").contains(value));
    }
    assert_eq!(format!("{store:?}"), "MemoryStore { items: 1 }");
    store.delete(&item()).unwrap();
    assert!(store.find(&item()).unwrap().is_none());
}

#[test]
fn service_and_account_separate_items_but_labels_do_not() {
    let store = MemoryStore::new();
    store
        .store(&item(), &Secret::new("invented-token".into()))
        .unwrap();
    for other in [
        Item::new("other", "invented", "Invented").unwrap(),
        Item::new("awna-keystore.test", "other", "Invented").unwrap(),
    ] {
        assert!(store.find(&other).unwrap().is_none());
    }
    let renamed = Item::new(item().service(), item().account(), "Other label").unwrap();
    assert_eq!(
        store.find(&renamed).unwrap().unwrap().expose(),
        "invented-token"
    );
}

#[test]
fn refused_values_write_nothing_and_preserve_existing_values() {
    let store = MemoryStore::new();
    let refused = Secret::new("invented value".into());
    assert_eq!(store.store(&item(), &refused), Err(Error::NotStorable));
    assert!(!store.exists(&item()).unwrap());
    store
        .store(&item(), &Secret::new("invented-token".into()))
        .unwrap();
    assert_eq!(store.store(&item(), &refused), Err(Error::NotStorable));
    assert_eq!(
        store.find(&item()).unwrap().unwrap().expose(),
        "invented-token"
    );
}
