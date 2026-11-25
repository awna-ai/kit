//! An in-memory implementation of the store contract for tests and application mocks.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    Attachment, Event, Filter, IllegalMove, Item, ItemId, Limits, NewFile, NewItem, Note,
    Processed, Reference, Status, Store, StoreError, ValidationError,
};

type Result<T> = std::result::Result<T, StoreError<Infallible>>;

/// A synchronized store that keeps attachment bytes and items in memory.
pub struct MemoryStore {
    limits: Limits,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    entries: BTreeMap<ItemId, Entry>,
    last_id: u64,
    last_stamp: u64,
}

struct Entry {
    item: Item,
    files: Vec<Vec<u8>>,
}

impl MemoryStore {
    /// Create an empty store using these submission limits.
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            state: Mutex::new(State::default()),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}

impl State {
    fn stamp(&mut self) -> u64 {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis());
        let now = u64::try_from(millis).unwrap_or(u64::MAX);
        self.last_stamp = self.last_stamp.max(now);
        self.last_stamp
    }

    fn entry_mut(&mut self, id: ItemId) -> Result<&mut Entry> {
        self.entries.get_mut(&id).ok_or(StoreError::NotFound(id))
    }
}

impl Store for MemoryStore {
    type Error = Infallible;

    fn limits(&self) -> &Limits {
        &self.limits
    }

    async fn submit(&self, item: &NewItem, files: &[NewFile<'_>]) -> Result<Item> {
        item.validate().map_err(StoreError::Invalid)?;
        self.limits.check(files).map_err(StoreError::Invalid)?;
        let mut state = self.state();
        let next = state
            .last_id
            .checked_add(1)
            .and_then(ItemId::new)
            .ok_or_else(|| {
                StoreError::Invalid(ValidationError {
                    field: "id".into(),
                    reason: "item ids are exhausted",
                })
            })?;
        let now = state.stamp();
        let attachments = files
            .iter()
            .enumerate()
            .map(|(index, file)| Attachment {
                index: index as u32,
                name: file.name.clone(),
                content_type: file.content_type.clone(),
                role: file.role,
                size: file.bytes.len() as u64,
            })
            .collect();
        let stored = Item {
            id: next,
            source: item.source.clone(),
            requester: item.requester.clone(),
            title: item.title.clone(),
            body: item.body.clone(),
            snapshot: item.snapshot.clone(),
            attachments,
            triage: None,
            proposal: None,
            status: Status::Submitted,
            events: vec![Event::Status {
                at_ms: now,
                from: None,
                to: Status::Submitted,
                reference: None,
            }],
            created_at_ms: now,
            updated_at_ms: now,
        };
        stored.validate().map_err(StoreError::Invalid)?;
        state.entries.insert(
            next,
            Entry {
                item: stored.clone(),
                files: files.iter().map(|file| file.bytes.to_vec()).collect(),
            },
        );
        state.last_id = next.get();
        Ok(stored)
    }

    async fn get(&self, id: ItemId) -> Result<Item> {
        self.state()
            .entries
            .get(&id)
            .map(|entry| entry.item.clone())
            .ok_or(StoreError::NotFound(id))
    }

    async fn list(&self, filter: &Filter) -> Result<Vec<Item>> {
        filter.validate().map_err(StoreError::Invalid)?;
        Ok(self
            .state()
            .entries
            .values()
            .map(|entry| &entry.item)
            .filter(|item| {
                (filter.statuses.is_empty() || filter.statuses.contains(&item.status))
                    && filter
                        .source
                        .as_ref()
                        .is_none_or(|source| source == &item.source)
                    && filter
                        .requester
                        .as_ref()
                        .is_none_or(|requester| requester == &item.requester)
            })
            .take(filter.limit.map_or(usize::MAX, |limit| limit as usize))
            .cloned()
            .collect())
    }

    async fn record_processing(&self, id: ItemId, processed: &Processed) -> Result<Item> {
        processed.validate().map_err(StoreError::Invalid)?;
        let mut state = self.state();
        let from = state.entry_mut(id)?.item.status;
        if from != Status::Submitted {
            return Err(StoreError::IllegalMove(IllegalMove {
                from,
                to: Status::Processed,
            }));
        }
        let now = state.stamp();
        let item = &mut state.entry_mut(id)?.item;
        item.triage = Some(processed.triage.clone());
        item.proposal = Some(processed.proposal.clone());
        item.status = Status::Processed;
        item.events.push(Event::Status {
            at_ms: now,
            from: Some(from),
            to: Status::Processed,
            reference: None,
        });
        item.updated_at_ms = now;
        Ok(item.clone())
    }

    async fn set_status(
        &self,
        id: ItemId,
        to: Status,
        reference: Option<&Reference>,
    ) -> Result<Item> {
        let mut state = self.state();
        let from = state.entry_mut(id)?.item.status;
        if to == Status::Processed {
            return Err(StoreError::Invalid(ValidationError {
                field: "to".into(),
                reason: "only record_processing moves an item to processed",
            }));
        }
        from.check_move(to).map_err(StoreError::IllegalMove)?;
        let now = state.stamp();
        let item = &mut state.entry_mut(id)?.item;
        item.status = to;
        item.events.push(Event::Status {
            at_ms: now,
            from: Some(from),
            to,
            reference: reference.cloned(),
        });
        item.updated_at_ms = now;
        Ok(item.clone())
    }

    async fn add_note(&self, id: ItemId, note: &Note) -> Result<Item> {
        note.validate().map_err(StoreError::Invalid)?;
        let mut state = self.state();
        state.entry_mut(id)?;
        let now = state.stamp();
        let item = &mut state.entry_mut(id)?.item;
        item.events.push(Event::Note {
            at_ms: now,
            kind: note.kind.clone(),
            body: note.body.clone(),
        });
        item.updated_at_ms = now;
        Ok(item.clone())
    }

    async fn read_file(&self, id: ItemId, index: u32) -> Result<Vec<u8>> {
        let state = self.state();
        let entry = state.entries.get(&id).ok_or(StoreError::NotFound(id))?;
        entry
            .files
            .get(index as usize)
            .cloned()
            .ok_or(StoreError::NoFile { id, index })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_stamps_never_go_backwards() {
        let mut state = State {
            last_stamp: u64::MAX,
            ..State::default()
        };
        assert_eq!(state.stamp(), u64::MAX);
        assert_eq!(state.stamp(), u64::MAX);
    }

    #[test]
    fn a_poisoned_lock_is_recovered() {
        let store = MemoryStore::default();
        std::thread::scope(|scope| {
            let thread = scope.spawn(|| {
                let _state = store.state();
                panic!("poison the test lock");
            });
            assert!(thread.join().is_err());
        });
        assert!(store.state().entries.is_empty());
    }
}
