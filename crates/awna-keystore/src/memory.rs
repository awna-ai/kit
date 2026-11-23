use crate::{CredentialStore, Error, Item, MAX_SECRET, Secret, read_back, storable};
use std::{collections::HashMap, fmt, sync::Mutex};
use zeroize::Zeroizing;

/// A memory-only store that wipes values when replaced, removed, or dropped.
#[derive(Default)]
pub struct MemoryStore {
    values: Mutex<HashMap<(String, String), Zeroizing<String>>>,
}
impl MemoryStore {
    /// Creates an empty store without touching system storage.
    pub fn new() -> MemoryStore {
        Self::default()
    }
}
impl fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryStore")
            .field(
                "items",
                &self.values.lock().unwrap_or_else(|e| e.into_inner()).len(),
            )
            .finish()
    }
}
impl CredentialStore for MemoryStore {
    fn find(&self, item: &Item) -> Result<Option<Secret>, Error> {
        let values = self.values.lock().unwrap_or_else(|e| e.into_inner());
        Ok(values
            .get(&(item.service().into(), item.account().into()))
            .map(|v| Secret::new(v.to_string())))
    }
    fn exists(&self, item: &Item) -> Result<bool, Error> {
        Ok(self.find(item)?.is_some())
    }
    fn store(&self, item: &Item, value: &Secret) -> Result<(), Error> {
        if !storable(value, MAX_SECRET) {
            return Err(Error::NotStorable);
        }
        self.values
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                (item.service().into(), item.account().into()),
                Zeroizing::new(value.expose().to_owned()),
            );
        read_back(self, item, value)
    }
    fn delete(&self, item: &Item) -> Result<(), Error> {
        self.values
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(item.service().into(), item.account().into()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn poisoned_locks_are_recovered_for_all_operations() {
        let store = MemoryStore::new();
        let _ = std::panic::catch_unwind(|| {
            let _guard = store.values.lock().unwrap();
            panic!("invented failure");
        });
        let item = Item::new("invented", "invented", "Invented").unwrap();
        assert!(store.find(&item).unwrap().is_none());
        assert!(!store.exists(&item).unwrap());
        store
            .store(&item, &Secret::new("invented-token".into()))
            .unwrap();
        assert_eq!(
            store.find(&item).unwrap().unwrap().expose(),
            "invented-token"
        );
        assert_eq!(format!("{store:?}"), "MemoryStore { items: 1 }");
        store.delete(&item).unwrap();
        assert!(!store.exists(&item).unwrap());
    }
}
