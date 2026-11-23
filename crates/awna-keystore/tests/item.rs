use awna_keystore::{Error, Item};

#[test]
fn item_accepts_bounded_identifiers_and_label_characters() {
    for size in [1, 64] {
        let id = "a".repeat(size);
        let item = Item::new(&id, &id, &id).unwrap();
        assert_eq!(item.service(), id);
        assert_eq!(item.account(), id);
        assert_eq!(item.label(), id);
        assert_eq!(item.clone(), item);
    }
    assert!(Item::new("a0.-", "a0.-", "aZ09 .,:()_-").is_ok());
}

#[test]
fn item_rejects_invalid_identifiers_and_labels() {
    for id in ["", &"a".repeat(65), "A", "a_", "a/", "é", "a ", "a\0"] {
        assert_eq!(Item::new(id, "a", "A"), Err(Error::InvalidItem));
        assert_eq!(Item::new("a", id, "A"), Err(Error::InvalidItem));
    }
    for label in [
        "",
        "\"",
        "\\",
        "\n",
        "\t",
        " A",
        "A ",
        &"a".repeat(65),
        "é",
        "A\0",
    ] {
        assert_eq!(Item::new("a", "a", label), Err(Error::InvalidItem));
    }
}
