//! Transactional storage with validated, consistent reads.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{PgError, PgStore};
use awna_intake::{
    Attachment, Event, Filter, IllegalMove, Item, ItemId, Limits, NewFile, NewItem, Note,
    Processed, Reference, Status, Store, StoreError, ValidationError,
};
use sqlx::{PgConnection, Row, postgres::PgRow};

type Result<T> = std::result::Result<T, StoreError<PgError>>;

impl From<PgError> for StoreError<PgError> {
    fn from(error: PgError) -> Self {
        Self::Backend(error)
    }
}
fn corrupt(reason: &'static str) -> StoreError<PgError> {
    PgError::Corrupt(reason).into()
}

// JSON is text at the SQL boundary; no sqlx JSON feature participates in encoding.
macro_rules! decode_json {
    ($text:expr) => {
        serde_json::from_str(&$text).map_err(|_| corrupt("invalid JSON"))?
    };
}
macro_rules! encode_json {
    ($value:expr) => {
        serde_json::to_string($value).map_err(|_| corrupt("JSON encoding failed"))?
    };
}
fn db_id(id: ItemId) -> Result<i64> {
    i64::try_from(id.get()).map_err(|_| StoreError::NotFound(id))
}
fn stored_u64(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| corrupt("integer out of range"))
}
fn db_time(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| corrupt("timestamp out of range"))
}
fn stamp(previous: u64) -> Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let now = u64::try_from(millis).map_err(|_| corrupt("timestamp out of range"))?;
    let now = previous.max(now);
    db_time(now)?;
    Ok(now)
}
fn check_history(item: &Item) -> Result<()> {
    if item.events.len() >= 256 {
        return Err(StoreError::Invalid(ValidationError {
            field: "events".into(),
            reason: "event history is full",
        }));
    }
    Ok(())
}

/// Read all child rows in two queries for the entire selected page.
async fn hydrate(conn: &mut PgConnection, rows: Vec<PgRow>) -> Result<Vec<Item>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids = rows
        .iter()
        .map(|row| row.try_get::<i64, _>("id"))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(PgError::from)?;
    let event_rows = sqlx::query("SELECT item_id, seq, event::text AS event FROM intake_events WHERE item_id = ANY($1) ORDER BY item_id, seq")
        .bind(&ids).fetch_all(&mut *conn).await.map_err(PgError::from)?;
    let attachment_rows = sqlx::query("SELECT item_id, file_index, meta::text AS meta FROM intake_attachments WHERE item_id = ANY($1) ORDER BY item_id, file_index")
        .bind(&ids).fetch_all(&mut *conn).await.map_err(PgError::from)?;
    let mut events: BTreeMap<i64, Vec<Event>> = BTreeMap::new();
    for row in event_rows {
        let item_id = row.try_get("item_id").map_err(PgError::from)?;
        let entries = events.entry(item_id).or_default();
        let seq: i32 = row.try_get("seq").map_err(PgError::from)?;
        if usize::try_from(seq).ok() != Some(entries.len()) {
            return Err(corrupt("event sequence is not consecutive"));
        }
        entries.push(decode_json!(
            row.try_get::<String, _>("event").map_err(PgError::from)?
        ));
    }
    let mut attachments: BTreeMap<i64, Vec<Attachment>> = BTreeMap::new();
    for row in attachment_rows {
        let item_id = row.try_get("item_id").map_err(PgError::from)?;
        let entries = attachments.entry(item_id).or_default();
        let index: i32 = row.try_get("file_index").map_err(PgError::from)?;
        let meta: Attachment =
            decode_json!(row.try_get::<String, _>("meta").map_err(PgError::from)?);
        if usize::try_from(index).ok() != Some(entries.len())
            || u32::try_from(index).ok() != Some(meta.index)
        {
            return Err(corrupt("attachment index differs from metadata"));
        }
        entries.push(meta);
    }
    rows.into_iter()
        .map(|row| {
            let number: i64 = row.try_get("id").map_err(PgError::from)?;
            let id = ItemId::new(stored_u64(number)?).ok_or_else(|| corrupt("invalid item id"))?;
            let events = events.remove(&number).unwrap_or_default();
            let count: i32 = row.try_get("event_count").map_err(PgError::from)?;
            if usize::try_from(count).ok() != Some(events.len()) {
                return Err(corrupt("event count differs from history"));
            }
            let item = Item {
                id,
                source: row
                    .try_get::<String, _>("source")
                    .map_err(PgError::from)?
                    .parse()
                    .map_err(|_| corrupt("invalid source"))?,
                requester: row
                    .try_get::<String, _>("requester")
                    .map_err(PgError::from)?
                    .parse()
                    .map_err(|_| corrupt("invalid requester"))?,
                title: row.try_get("title").map_err(PgError::from)?,
                body: row.try_get("body").map_err(PgError::from)?,
                snapshot: decode_json!(
                    row.try_get::<Option<String>, _>("snapshot")
                        .map_err(PgError::from)?
                        .unwrap_or_else(|| "null".into())
                ),
                triage: decode_json!(
                    row.try_get::<Option<String>, _>("triage")
                        .map_err(PgError::from)?
                        .unwrap_or_else(|| "null".into())
                ),
                proposal: decode_json!(
                    row.try_get::<Option<String>, _>("proposal")
                        .map_err(PgError::from)?
                        .unwrap_or_else(|| "null".into())
                ),
                attachments: attachments.remove(&number).unwrap_or_default(),
                status: row
                    .try_get::<String, _>("status")
                    .map_err(PgError::from)?
                    .parse()
                    .map_err(|_| corrupt("invalid status"))?,
                events,
                created_at_ms: stored_u64(row.try_get("created_at_ms").map_err(PgError::from)?)?,
                updated_at_ms: stored_u64(row.try_get("updated_at_ms").map_err(PgError::from)?)?,
            };
            item.validate().map_err(|_| corrupt("invalid item"))?;
            Ok(item)
        })
        .collect()
}

async fn locked(conn: &mut PgConnection, id: ItemId) -> Result<Item> {
    let number = db_id(id)?;
    let row = sqlx::query("SELECT id, source, requester, status, title, body, snapshot::text AS snapshot, triage::text AS triage, proposal::text AS proposal, created_at_ms, updated_at_ms, event_count FROM intake_items WHERE id = $1 FOR UPDATE")
        .bind(number).fetch_optional(&mut *conn).await.map_err(PgError::from)?.ok_or(StoreError::NotFound(id))?;
    hydrate(conn, vec![row])
        .await?
        .pop()
        .ok_or_else(|| corrupt("missing item"))
}

async fn append(conn: &mut PgConnection, item: &mut Item, event: Event) -> Result<()> {
    let number = db_id(item.id)?;
    let seq = i32::try_from(item.events.len()).map_err(|_| corrupt("event index out of range"))?;
    item.updated_at_ms = event.at_ms();
    sqlx::query("INSERT INTO intake_events (item_id, seq, event) VALUES ($1, $2, $3::json)")
        .bind(number)
        .bind(seq)
        .bind(encode_json!(&event))
        .execute(&mut *conn)
        .await
        .map_err(PgError::from)?;
    item.events.push(event);
    sqlx::query("UPDATE intake_items SET status = $2, triage = $3::json, proposal = $4::json, updated_at_ms = $5, event_count = $6 WHERE id = $1")
        .bind(number).bind(item.status.as_str())
        .bind(item.triage.as_ref().map(serde_json::to_string).transpose().map_err(|_| corrupt("JSON encoding failed"))?)
        .bind(item.proposal.as_ref().map(serde_json::to_string).transpose().map_err(|_| corrupt("JSON encoding failed"))?)
        .bind(db_time(item.updated_at_ms)?).bind(seq + 1).execute(&mut *conn).await.map_err(PgError::from)?;
    Ok(())
}

impl Store for PgStore {
    type Error = PgError;
    fn limits(&self) -> &Limits {
        &self.limits
    }

    async fn submit(&self, input: &NewItem, files: &[NewFile<'_>]) -> Result<Item> {
        input.validate().map_err(StoreError::Invalid)?;
        self.limits.check(files).map_err(StoreError::Invalid)?;
        let now = stamp(0)?;
        let attachments = files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                Ok(Attachment {
                    index: u32::try_from(index)
                        .map_err(|_| corrupt("attachment index out of range"))?,
                    name: file.name.clone(),
                    content_type: file.content_type.clone(),
                    role: file.role,
                    size: u64::try_from(file.bytes.len())
                        .map_err(|_| corrupt("file size out of range"))?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        // Validate the complete item before opening a transaction, just as the reference store does.
        let mut item = Item {
            id: ItemId::new(1).expect("positive id"),
            source: input.source.clone(),
            requester: input.requester.clone(),
            title: input.title.clone(),
            body: input.body.clone(),
            snapshot: input.snapshot.clone(),
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
        item.validate().map_err(StoreError::Invalid)?;
        let mut tx = self.pool.begin().await.map_err(PgError::from)?;
        let number: i64 = sqlx::query("INSERT INTO intake_items (source, requester, status, title, body, snapshot, created_at_ms, updated_at_ms, event_count) VALUES ($1, $2, $3, $4, $5, $6::json, $7, $7, $8) RETURNING id")
            .bind(item.source.as_str()).bind(item.requester.as_str()).bind(item.status.as_str()).bind(&item.title).bind(&item.body)
            .bind(item.snapshot.as_ref().map(serde_json::to_string).transpose().map_err(|_| corrupt("JSON encoding failed"))?)
            .bind(db_time(now)?).bind(1_i32).fetch_one(&mut *tx).await.map_err(PgError::from)?.try_get("id").map_err(PgError::from)?;
        item.id = ItemId::new(stored_u64(number)?).ok_or_else(|| corrupt("invalid item id"))?;
        sqlx::query("INSERT INTO intake_events (item_id, seq, event) VALUES ($1, $2, $3::json)")
            .bind(number)
            .bind(0_i32)
            .bind(encode_json!(&item.events[0]))
            .execute(&mut *tx)
            .await
            .map_err(PgError::from)?;
        for (meta, file) in item.attachments.iter().zip(files) {
            sqlx::query("INSERT INTO intake_attachments (item_id, file_index, meta, bytes) VALUES ($1, $2, $3::json, $4)")
                .bind(number).bind(i32::try_from(meta.index).map_err(|_| corrupt("attachment index out of range"))?)
                .bind(encode_json!(meta)).bind(file.bytes).execute(&mut *tx).await.map_err(PgError::from)?;
        }
        tx.commit().await.map_err(PgError::from)?;
        Ok(item)
    }

    async fn get(&self, id: ItemId) -> Result<Item> {
        let number = db_id(id)?;
        let mut tx = self.pool.begin().await.map_err(PgError::from)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(PgError::from)?;
        let row = sqlx::query("SELECT id, source, requester, status, title, body, snapshot::text AS snapshot, triage::text AS triage, proposal::text AS proposal, created_at_ms, updated_at_ms, event_count FROM intake_items WHERE id = $1")
            .bind(number).fetch_optional(&mut *tx).await.map_err(PgError::from)?.ok_or(StoreError::NotFound(id))?;
        let item = hydrate(&mut tx, vec![row])
            .await?
            .pop()
            .ok_or_else(|| corrupt("missing item"))?;
        tx.commit().await.map_err(PgError::from)?;
        Ok(item)
    }

    async fn list(&self, filter: &Filter) -> Result<Vec<Item>> {
        filter.validate().map_err(StoreError::Invalid)?;
        let statuses: Vec<&str> = filter.statuses.iter().map(|s| s.as_str()).collect();
        let mut tx = self.pool.begin().await.map_err(PgError::from)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(PgError::from)?;
        let rows = sqlx::query("SELECT id, source, requester, status, title, body, snapshot::text AS snapshot, triage::text AS triage, proposal::text AS proposal, created_at_ms, updated_at_ms, event_count FROM intake_items WHERE (cardinality($1::text[]) = 0 OR status = ANY($1)) AND ($2::text IS NULL OR source = $2) AND ($3::text IS NULL OR requester = $3) ORDER BY id LIMIT $4")
            .bind(&statuses).bind(filter.source.as_ref().map(|v| v.as_str())).bind(filter.requester.as_ref().map(|v| v.as_str()))
            .bind(filter.limit.map(i64::from)).fetch_all(&mut *tx).await.map_err(PgError::from)?;
        let items = hydrate(&mut tx, rows).await?;
        tx.commit().await.map_err(PgError::from)?;
        Ok(items)
    }

    async fn record_processing(&self, id: ItemId, processed: &Processed) -> Result<Item> {
        db_id(id)?;
        let mut tx = self.pool.begin().await.map_err(PgError::from)?;
        let mut item = locked(&mut tx, id).await?;
        processed.validate().map_err(StoreError::Invalid)?;
        check_history(&item)?;
        let from = item.status;
        if from != Status::Submitted {
            return Err(StoreError::IllegalMove(IllegalMove {
                from,
                to: Status::Processed,
            }));
        }
        let now = stamp(item.updated_at_ms)?;
        item.triage = Some(processed.triage.clone());
        item.proposal = Some(processed.proposal.clone());
        item.status = Status::Processed;
        append(
            &mut tx,
            &mut item,
            Event::Status {
                at_ms: now,
                from: Some(from),
                to: Status::Processed,
                reference: None,
            },
        )
        .await?;
        tx.commit().await.map_err(PgError::from)?;
        Ok(item)
    }

    async fn set_status(
        &self,
        id: ItemId,
        to: Status,
        reference: Option<&Reference>,
    ) -> Result<Item> {
        db_id(id)?;
        let mut tx = self.pool.begin().await.map_err(PgError::from)?;
        let mut item = locked(&mut tx, id).await?;
        if to == Status::Processed {
            return Err(StoreError::Invalid(ValidationError {
                field: "to".into(),
                reason: "only record_processing moves an item to processed",
            }));
        }
        check_history(&item)?;
        let from = item.status;
        from.check_move(to).map_err(StoreError::IllegalMove)?;
        let now = stamp(item.updated_at_ms)?;
        item.status = to;
        append(
            &mut tx,
            &mut item,
            Event::Status {
                at_ms: now,
                from: Some(from),
                to,
                reference: reference.cloned(),
            },
        )
        .await?;
        tx.commit().await.map_err(PgError::from)?;
        Ok(item)
    }

    async fn add_note(&self, id: ItemId, note: &Note) -> Result<Item> {
        db_id(id)?;
        let mut tx = self.pool.begin().await.map_err(PgError::from)?;
        let mut item = locked(&mut tx, id).await?;
        note.validate().map_err(StoreError::Invalid)?;
        check_history(&item)?;
        let now = stamp(item.updated_at_ms)?;
        append(
            &mut tx,
            &mut item,
            Event::Note {
                at_ms: now,
                kind: note.kind.clone(),
                body: note.body.clone(),
            },
        )
        .await?;
        tx.commit().await.map_err(PgError::from)?;
        Ok(item)
    }

    async fn read_file(&self, id: ItemId, index: u32) -> Result<Vec<u8>> {
        let number = db_id(id)?;
        let mut tx = self.pool.begin().await.map_err(PgError::from)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(PgError::from)?;
        sqlx::query("SELECT id FROM intake_items WHERE id = $1")
            .bind(number)
            .fetch_optional(&mut *tx)
            .await
            .map_err(PgError::from)?
            .ok_or(StoreError::NotFound(id))?;
        // Check and load bytes from the same snapshot, using only SELECT privileges.
        let row = sqlx::query("SELECT octet_length(bytes) AS size, meta::text AS meta FROM intake_attachments WHERE item_id = $1 AND file_index::bigint = $2")
            .bind(number).bind(i64::from(index)).fetch_optional(&mut *tx).await.map_err(PgError::from)?
            .ok_or(StoreError::NoFile { id, index })?;
        let meta: Attachment =
            decode_json!(row.try_get::<String, _>("meta").map_err(PgError::from)?);
        let size: i32 = row.try_get("size").map_err(PgError::from)?;
        if meta.index != index {
            return Err(corrupt("attachment index differs from metadata"));
        }
        if u64::try_from(size).ok() != Some(meta.size) {
            return Err(corrupt("attachment size differs from metadata"));
        }
        let bytes = sqlx::query_scalar(
            "SELECT bytes FROM intake_attachments WHERE item_id = $1 AND file_index::bigint = $2",
        )
        .bind(number)
        .bind(i64::from(index))
        .fetch_one(&mut *tx)
        .await
        .map_err(PgError::from)?;
        tx.commit().await.map_err(PgError::from)?;
        Ok(bytes)
    }
}
