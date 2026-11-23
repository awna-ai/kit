use crate::{CredentialStore, Error, Item, MAX_SECRET, Secret, item_name, read_back, storable};
use std::sync::Arc;
use zeroize::Zeroize;

#[derive(Clone, Copy)]
enum Flavor {
    #[cfg(any(windows, test))]
    Windows,
    #[cfg(any(target_os = "linux", test))]
    Linux,
    #[cfg(test)]
    Plain,
}

pub(crate) struct KeyringStore {
    store: Arc<keyring_core::CredentialStore>,
    flavor: Flavor,
}
#[cfg(any(target_os = "linux", test))]
fn local_session_bus(address: Option<&std::ffi::OsStr>) -> bool {
    let Some(address) = address else {
        return true;
    };
    let Some(address) = address.to_str() else {
        return false;
    };
    let mut entries = address
        .split(';')
        .filter(|entry| !entry.is_empty())
        .peekable();
    entries.peek().is_some() && entries.all(|entry| entry.starts_with("unix:"))
}
fn modifiers(flavor: Flavor, item: &Item) -> Vec<(&'static str, String)> {
    // Windows has no label modifier.
    let _ = item;
    match flavor {
        #[cfg(any(windows, test))]
        Flavor::Windows => vec![("persistence", "Local".into())],
        #[cfg(any(target_os = "linux", test))]
        Flavor::Linux => vec![("label", item.label().into())],
        #[cfg(test)]
        Flavor::Plain => Vec::new(),
    }
}
impl KeyringStore {
    #[cfg(windows)]
    pub(crate) fn open() -> Result<Self, Error> {
        Ok(Self {
            store: windows_native_keyring_store::Store::new().map_err(|_| Error::NoStore)?,
            flavor: Flavor::Windows,
        })
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn open() -> Result<Self, Error> {
        let address = std::env::var_os("DBUS_SESSION_BUS_ADDRESS");
        if !local_session_bus(address.as_deref()) {
            return Err(Error::NoStore);
        }
        Ok(Self {
            store: zbus_secret_service_keyring_store::Store::new().map_err(|_| Error::NoStore)?,
            flavor: Flavor::Linux,
        })
    }
    fn entry(&self, item: &Item, op: &'static str) -> Result<keyring_core::Entry, Error> {
        let pairs = modifiers(self.flavor, item);
        let map = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        self.store
            .build(
                item.service(),
                item.account(),
                if pairs.is_empty() { None } else { Some(&map) },
            )
            .map_err(|error| map_error(error, item, op))
    }
}
fn map_error(error: keyring_core::Error, item: &Item, op: &'static str) -> Error {
    use keyring_core::Error as E;
    let why = match error {
        E::BadEncoding(mut bytes) | E::BadDataFormat(mut bytes, _) => {
            bytes.zeroize();
            return Error::Unreadable {
                item: item_name(item),
            };
        }
        E::PlatformFailure(_) => "platform failure",
        E::NoStorageAccess(_) => "no storage access",
        E::BadStoreFormat(_) => "bad store format",
        E::TooLong(_, _) => "too long",
        E::Invalid(_, _) => "invalid",
        E::Ambiguous(_) => "ambiguous",
        E::NoDefaultStore => "no default store",
        E::NotSupportedByStore(_) => "not supported",
        _ => "store error",
    };
    Error::Failed {
        item: item_name(item),
        op,
        why: why.into(),
    }
}
impl CredentialStore for KeyringStore {
    fn find(&self, item: &Item) -> Result<Option<Secret>, Error> {
        match self.entry(item, "find")?.get_password() {
            Ok(value) if value.is_empty() => Err(Error::Unreadable {
                item: item_name(item),
            }),
            Ok(value) => Ok(Some(Secret::new(value))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(error) => Err(map_error(error, item, "find")),
        }
    }
    fn exists(&self, item: &Item) -> Result<bool, Error> {
        match self.entry(item, "exists")?.get_attributes() {
            Ok(_) => Ok(true),
            Err(keyring_core::Error::NoEntry) => Ok(false),
            Err(error) => Err(map_error(error, item, "exists")),
        }
    }
    fn store(&self, item: &Item, value: &Secret) -> Result<(), Error> {
        if !storable(value, MAX_SECRET) {
            return Err(Error::NotStorable);
        }
        self.entry(item, "store")?
            .set_password(value.expose())
            .map_err(|error| map_error(error, item, "store"))?;
        read_back(self, item, value)
    }
    fn delete(&self, item: &Item) -> Result<(), Error> {
        match self.entry(item, "delete")?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(error) => Err(map_error(error, item, "delete")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keyring_core::Error as StoreError;
    use keyring_core::api::{CredentialApi, CredentialStoreApi};
    use std::collections::HashMap;

    #[derive(Clone, Copy)]
    struct FixedReadStore(Option<&'static [u8]>);

    impl CredentialStoreApi for FixedReadStore {
        fn vendor(&self) -> String {
            "invented".into()
        }
        fn id(&self) -> String {
            "invented".into()
        }
        fn build(
            &self,
            _: &str,
            _: &str,
            _: Option<&HashMap<&str, &str>>,
        ) -> keyring_core::Result<keyring_core::Entry> {
            Ok(keyring_core::Entry::new_with_credential(Arc::new(*self)))
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }
    impl CredentialApi for FixedReadStore {
        fn set_secret(&self, _: &[u8]) -> keyring_core::Result<()> {
            Ok(())
        }
        fn get_secret(&self) -> keyring_core::Result<Vec<u8>> {
            self.0.map(<[u8]>::to_vec).ok_or(StoreError::NoEntry)
        }
        fn delete_credential(&self) -> keyring_core::Result<()> {
            Ok(())
        }
        fn get_credential(
            &self,
        ) -> keyring_core::Result<Option<Arc<keyring_core::api::Credential>>> {
            self.get_secret()?;
            Ok(None)
        }
        fn get_specifiers(&self) -> Option<(String, String)> {
            None
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }
    fn store() -> KeyringStore {
        KeyringStore {
            store: keyring_core::mock::Store::new().unwrap(),
            flavor: Flavor::Plain,
        }
    }
    fn item() -> Item {
        Item::new("awna-keystore.test", "invented", "Invented label").unwrap()
    }
    fn inject(store: &KeyringStore, error: StoreError) {
        let entry = store.entry(&item(), "find").unwrap();
        entry
            .as_any()
            .downcast_ref::<keyring_core::mock::Cred>()
            .unwrap()
            .set_error(error);
    }
    #[test]
    fn session_bus_addresses_allow_only_local_unix_entries() {
        assert!(local_session_bus(None));
        for address in [
            "unix:path=/run/user/1/bus",
            "unix:abstract=x;unix:path=/y",
            ";unix:path=/x;",
        ] {
            assert!(local_session_bus(Some(address.as_ref())), "{address}");
        }
        for address in [
            "tcp:host=example.invalid,port=1",
            "unix:path=/x;tcp:host=y",
            "unixexec:path=/bin/true",
            "nonce-tcp:",
            "",
            ";",
        ] {
            assert!(!local_session_bus(Some(address.as_ref())), "{address}");
        }
    }
    #[test]
    #[cfg(any(unix, windows))]
    fn session_bus_addresses_reject_non_utf8() {
        #[cfg(unix)]
        let address = {
            use std::os::unix::ffi::OsStringExt;
            std::ffi::OsString::from_vec(b"unix:path=/x\xff".to_vec())
        };
        #[cfg(windows)]
        let address = {
            use std::os::windows::ffi::OsStringExt;
            let mut units: Vec<u16> = "unix:path=/x".encode_utf16().collect();
            units.push(0xd800);
            std::ffi::OsString::from_wide(&units)
        };
        assert!(!local_session_bus(Some(&address)));
    }
    #[test]
    fn linux_modifiers_reach_the_store() {
        modifiers_reach_the_store(Flavor::Linux);
    }
    #[test]
    fn windows_modifiers_reach_the_store() {
        modifiers_reach_the_store(Flavor::Windows);
    }
    fn modifiers_reach_the_store(flavor: Flavor) {
        let store = KeyringStore {
            store: keyring_core::mock::Store::new().unwrap(),
            flavor,
        };
        assert_eq!(
            store.find(&item()).unwrap_err(),
            Error::Failed {
                item: item_name(&item()),
                op: "find",
                why: "not supported".into(),
            }
        );
    }
    #[test]
    fn writes_must_read_back_a_value() {
        assert_read_back_failure(None);
    }
    #[test]
    fn writes_must_read_back_the_same_value() {
        assert_read_back_failure(Some(b"invented-other"));
    }
    fn assert_read_back_failure(value: Option<&'static [u8]>) {
        let store = KeyringStore {
            store: Arc::new(FixedReadStore(value)),
            flavor: Flavor::Plain,
        };
        assert_eq!(
            store.store(&item(), &Secret::new("invented-token".into())),
            Err(Error::ReadBack {
                item: item_name(&item()),
            })
        );
    }
    #[test]
    fn missing_is_absent_and_delete_is_idempotent() {
        let store = store();
        assert!(matches!(store.find(&item()), Ok(None)));
        assert_eq!(store.exists(&item()), Ok(false));
        assert_eq!(store.delete(&item()), Ok(()));
    }
    #[test]
    fn writes_round_trip_replace_and_delete() {
        let store = store();
        for value in ["invented-token", "other-token"] {
            assert_eq!(store.store(&item(), &Secret::new(value.into())), Ok(()));
            assert_eq!(store.find(&item()).unwrap().unwrap().expose(), value);
            assert_eq!(store.exists(&item()), Ok(true));
        }
        assert_eq!(store.delete(&item()), Ok(()));
        assert!(matches!(store.find(&item()), Ok(None)));
    }
    #[test]
    fn service_and_account_keep_items_apart() {
        let store = store();
        assert_eq!(
            store.store(&item(), &Secret::new("invented-token".into())),
            Ok(())
        );
        for other in [
            Item::new("other", "invented", "Invented").unwrap(),
            Item::new("awna-keystore.test", "other", "Invented").unwrap(),
        ] {
            assert!(matches!(store.find(&other), Ok(None)));
        }
    }
    #[test]
    fn refused_values_write_nothing() {
        let store = store();
        assert_eq!(
            store.store(&item(), &Secret::new("invented value".into())),
            Err(Error::NotStorable)
        );
        assert!(matches!(store.find(&item()), Ok(None)));
    }
    #[test]
    fn encoding_errors_and_empty_values_are_unreadable_without_leaking() {
        let store = store();
        for error in [
            StoreError::BadEncoding(b"invented-token".to_vec()),
            StoreError::BadDataFormat(
                b"invented-token".to_vec(),
                std::io::Error::other("invented-token").into(),
            ),
        ] {
            inject(&store, error);
            let error = store.find(&item()).unwrap_err();
            assert_eq!(
                error,
                Error::Unreadable {
                    item: "awna-keystore.test/invented".into()
                }
            );
            assert!(!format!("{error:?} {error}").contains("invented-token"));
        }
        store
            .entry(&item(), "find")
            .unwrap()
            .set_password("")
            .unwrap();
        assert!(matches!(store.find(&item()), Err(Error::Unreadable { .. })));
    }
    #[test]
    fn platform_errors_never_expose_their_message() {
        let store = store();
        inject(
            &store,
            StoreError::PlatformFailure(std::io::Error::other("invented-token").into()),
        );
        let error = store.find(&item()).unwrap_err();
        assert_eq!(
            error,
            Error::Failed {
                item: "awna-keystore.test/invented".into(),
                op: "find",
                why: "platform failure".into()
            }
        );
        assert!(!format!("{error:?} {error}").contains("invented-token"));
    }
    #[test]
    fn storage_access_errors_are_sanitized() {
        let store = store();
        inject(
            &store,
            StoreError::NoStorageAccess(std::io::Error::other("invented-token").into()),
        );
        assert_eq!(
            store.exists(&item()),
            Err(Error::Failed {
                item: "awna-keystore.test/invented".into(),
                op: "exists",
                why: "no storage access".into()
            })
        );
    }
    #[test]
    fn linux_modifiers_use_each_items_own_label() {
        let first = item();
        let second = Item::new("other", "invented", "Other label").unwrap();
        assert_eq!(
            modifiers(Flavor::Linux, &first),
            vec![("label", "Invented label".into())]
        );
        assert_eq!(
            modifiers(Flavor::Linux, &second),
            vec![("label", "Other label".into())]
        );
    }
    #[test]
    fn windows_modifiers_keep_credentials_local() {
        assert_eq!(
            modifiers(Flavor::Windows, &item()),
            vec![("persistence", "Local".into())]
        );
    }
    #[test]
    fn plain_modifiers_are_absent() {
        assert!(modifiers(Flavor::Plain, &item()).is_empty());
    }
    #[test]
    fn remaining_error_kinds_use_fixed_phrases_and_operation_names() {
        let store = store();
        for (error, why) in [
            (
                StoreError::BadStoreFormat("invented-token".into()),
                "bad store format",
            ),
            (StoreError::TooLong("invented-token".into(), 1), "too long"),
            (
                StoreError::Invalid("invented-token".into(), "invented-token".into()),
                "invalid",
            ),
            (StoreError::Ambiguous(Vec::new()), "ambiguous"),
            (StoreError::NoDefaultStore, "no default store"),
            (
                StoreError::NotSupportedByStore("invented-token".into()),
                "not supported",
            ),
        ] {
            inject(&store, error);
            let error = store.delete(&item()).unwrap_err();
            assert_eq!(
                error,
                Error::Failed {
                    item: "awna-keystore.test/invented".into(),
                    op: "delete",
                    why: why.into()
                }
            );
            assert!(!format!("{error:?} {error}").contains("invented-token"));
        }
        inject(
            &store,
            StoreError::PlatformFailure(std::io::Error::other("invented-token").into()),
        );
        assert_eq!(
            store.store(&item(), &Secret::new("other-token".into())),
            Err(Error::Failed {
                item: "awna-keystore.test/invented".into(),
                op: "store",
                why: "platform failure".into()
            })
        );
        assert!(matches!(store.find(&item()), Ok(None)));
    }
}
