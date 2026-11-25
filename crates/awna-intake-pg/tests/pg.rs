//! Scratch-database checks, explicitly enabled before a release.

use std::future::Future;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use awna_intake::{
    EventKind, Item, Limits, NewFile, NewItem, Note, Role, Status, Store, StoreError,
};
use awna_intake_pg::{PgError, PgStore, SCHEMA, connect_options};
use sqlx::{PgPool, postgres::PgPoolOptions};

const PREFIX: &str = "awna_intake_pg_t_";
const CONNECTIONS: u32 = 16;
static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Scratch {
    pool: PgPool,
    admin: PgPool,
    name: String,
    url: String,
}

fn seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn checked_name(name: &str) {
    assert!(!name.is_empty());
    assert!(
        name.bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    );
}

async fn drop_database(admin: &PgPool, name: &str) -> Result<(), sqlx::Error> {
    checked_name(name);
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {name}"
    )))
    .execute(admin)
    .await?;
    Ok(())
}

async fn sweep(admin: &PgPool) {
    let names: Vec<String> =
        sqlx::query_scalar("SELECT datname FROM pg_database WHERE starts_with(datname, $1)")
            .bind(PREFIX)
            .fetch_all(admin)
            .await
            .expect("list stale scratch databases");
    for name in names {
        let timestamp = name
            .strip_prefix(PREFIX)
            .and_then(|tail| tail.split('_').next())
            .and_then(|part| part.parse::<u64>().ok());
        if timestamp.is_some_and(|timestamp| seconds().saturating_sub(timestamp) > 7200)
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
            && drop_database(admin, &name).await.is_err()
        {
            eprintln!("could not drop a stale scratch database");
        }
    }
}

async fn with_database<F, Fut>(body: F)
where
    F: FnOnce(Scratch) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let url = std::env::var("AWNA_INTAKE_PG_TEST_ADMIN_URL")
        .ok()
        .filter(|value| !value.is_empty())
        .expect("needs AWNA_INTAKE_PG_TEST_ADMIN_URL");
    let options = connect_options(&url).expect("valid local test database target");
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options.clone())
        .await
        .unwrap_or_else(|_| panic!("connect to test database server"));
    sweep(&admin).await;
    let name = format!(
        "{PREFIX}{}_{}_{}",
        seconds(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    checked_name(&name);
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&admin)
        .await
        .expect("create scratch database");
    let pool_result = PgPoolOptions::new()
        .max_connections(CONNECTIONS)
        .acquire_timeout(Duration::from_secs(30))
        .connect_with(options.database(&name))
        .await;
    let pool = match pool_result {
        Ok(pool) => pool,
        Err(_) => {
            drop_database(&admin, &name)
                .await
                .expect("drop scratch after connection failure");
            panic!("connect to scratch database");
        }
    };
    let separator = if url.contains('?') { '&' } else { '?' };
    let scratch = Scratch {
        pool: pool.clone(),
        admin: admin.clone(),
        name: name.clone(),
        url: format!("{url}{separator}dbname={name}"),
    };
    let outcome = tokio::spawn(async move {
        sqlx::raw_sql(SCHEMA)
            .execute(&scratch.pool)
            .await
            .expect("apply schema");
        body(scratch).await;
    })
    .await;
    pool.close().await;
    drop_database(&admin, &name)
        .await
        .expect("drop scratch database");
    admin.close().await;
    if let Err(error) = outcome {
        if error.is_panic() {
            std::panic::resume_unwind(error.into_panic());
        }
        panic!("scratch test task was cancelled");
    }
}

fn new_item() -> NewItem {
    NewItem {
        source: "web-form".parse().unwrap(),
        requester: "someone@example.com".parse().unwrap(),
        title: "The total is wrong".into(),
        body: "Please check the total.".into(),
        snapshot: None,
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs AWNA_INTAKE_PG_TEST_ADMIN_URL"]
async fn store_conformance() {
    with_database(|scratch| async move {
        awna_intake::conformance::run(&PgStore::new(
            scratch.pool,
            awna_intake::conformance::limits(),
        ))
        .await;
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs AWNA_INTAKE_PG_TEST_ADMIN_URL"]
async fn schema_is_plain_create() {
    with_database(|scratch| async move {
        assert!(
            sqlx::raw_sql(SCHEMA).execute(&scratch.pool).await.is_err(),
            "applying the schema twice must fail"
        );
    })
    .await;
}

async fn wait_for_blocked_writers(scratch: &Scratch, count: i64) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let blocked: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity WHERE datname = $1 AND wait_event_type = 'Lock'",
        )
        .bind(&scratch.name)
        .fetch_one(&scratch.admin)
        .await
        .expect("observe blocked writers");
        if blocked >= count {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "writers did not all reach the held row"
        );
        tokio::task::yield_now().await;
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs AWNA_INTAKE_PG_TEST_ADMIN_URL"]
async fn competing_moves_serialize() {
    with_database(|scratch| async move {
        let store = Arc::new(PgStore::new(scratch.pool.clone(), Limits::default()));
        let item = store.submit(&new_item(), &[]).await.unwrap();
        let item = store.set_status(item.id, Status::Held, None).await.unwrap();
        let id = i64::try_from(item.id.get()).unwrap();
        let mut lock = scratch.pool.begin().await.unwrap();
        sqlx::query("SELECT id FROM intake_items WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *lock)
            .await
            .unwrap();
        let mut tasks = Vec::new();
        for to in [Status::Queued, Status::Declined] {
            let store = Arc::clone(&store);
            tasks.push(tokio::spawn(async move {
                store.set_status(item.id, to, None).await
            }));
        }
        wait_for_blocked_writers(&scratch, 2).await;
        lock.commit().await.unwrap();
        let mut successes = 0;
        let mut refusals = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(_) => successes += 1,
                Err(StoreError::IllegalMove(_)) => refusals += 1,
                Err(error) => panic!("unexpected competing move error: {error}"),
            }
        }
        assert_eq!((successes, refusals), (1, 1));
        let saved = store.get(item.id).await.unwrap();
        assert_eq!(saved.events.len(), item.events.len() + 1);
        assert!(matches!(saved.status, Status::Queued | Status::Declined));
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs AWNA_INTAKE_PG_TEST_ADMIN_URL"]
async fn concurrent_notes_stop_at_history_cap() {
    with_database(|scratch| async move {
        let store = Arc::new(PgStore::new(scratch.pool.clone(), Limits::default()));
        let item = store.submit(&new_item(), &[]).await.unwrap();
        let id = i64::try_from(item.id.get()).unwrap();
        let mut lock = scratch.pool.begin().await.unwrap();
        sqlx::query("SELECT id FROM intake_items WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *lock)
            .await
            .unwrap();
        let mut tasks = Vec::new();
        for _ in 0..300 {
            let store = Arc::clone(&store);
            tasks.push(tokio::spawn(async move {
                store
                    .add_note(
                        item.id,
                        &Note {
                            kind: EventKind::new("comment").unwrap(),
                            body: "Checked.".into(),
                        },
                    )
                    .await
            }));
        }
        // All available writer connections must reach the locked row before release.
        // Without a store row lock these writers instead read the same old event count.
        wait_for_blocked_writers(&scratch, i64::from(CONNECTIONS - 1)).await;
        lock.commit().await.unwrap();
        let mut successes = 0;
        let mut refusals = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(_) => successes += 1,
                Err(StoreError::Invalid(error)) => {
                    assert_eq!(error.field, "events");
                    assert_eq!(error.reason, "event history is full");
                    refusals += 1;
                }
                Err(error) => panic!("unexpected concurrent note error: {error}"),
            }
        }
        assert_eq!((successes, refusals), (255, 45));
        assert_eq!(store.get(item.id).await.unwrap().events.len(), 256);
        assert!(
            matches!(
                store.set_status(item.id, Status::Processed, None).await,
                Err(StoreError::Invalid(error)) if error.field == "to"
            ),
            "processed target validation precedes the history cap"
        );
        assert!(
            matches!(
                store.set_status(item.id, Status::Submitted, None).await,
                Err(StoreError::Invalid(error)) if error.field == "events"
            ),
            "the history cap precedes the illegal-move check"
        );
        let seq: Vec<i32> =
            sqlx::query_scalar("SELECT seq FROM intake_events WHERE item_id = $1 ORDER BY seq")
                .bind(id)
                .fetch_all(&scratch.pool)
                .await
                .unwrap();
        assert_eq!(seq, (0..=255).collect::<Vec<_>>());
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs AWNA_INTAKE_PG_TEST_ADMIN_URL"]
async fn inconsistent_stored_row_is_opaque() {
    with_database(|scratch| async move {
        let store = PgStore::new(scratch.pool.clone(), Limits::default());
        let item = store.submit(&new_item(), &[]).await.unwrap();
        sqlx::query("UPDATE intake_items SET status = $1, title = $2 WHERE id = $3")
            .bind("shipped")
            .bind("stored-SENTINEL-7f3a")
            .bind(i64::try_from(item.id.get()).unwrap())
            .execute(&scratch.pool)
            .await
            .unwrap();
        let error = store.get(item.id).await.unwrap_err();
        assert!(matches!(&error, StoreError::Backend(PgError::Corrupt(_))));
        for rendered in [error.to_string(), format!("{error:?}")] {
            assert!(!rendered.contains("SENTINEL"));
            assert!(!rendered.contains("shipped"));
        }
    })
    .await;
}

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "awna_intake_pg_files_{}_{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("create file fixture directory");
        Self(path)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cli(url: &str, arguments: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_awna-intake"));
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("TEST_DATABASE_URL", url)
        .args(["--db-env", "TEST_DATABASE_URL"])
        .args(arguments);
    command.output().expect("run command line")
}

fn cli_item(url: &str, arguments: &[&str]) -> Item {
    let output = cli(url, arguments);
    assert_eq!(
        output.status.code(),
        Some(0),
        "command should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Item::from_json(std::str::from_utf8(&output.stdout).unwrap()).unwrap()
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs AWNA_INTAKE_PG_TEST_ADMIN_URL"]
async fn command_line_round_trip() {
    with_database(|scratch| async move {
        let store = PgStore::new(scratch.pool.clone(), Limits::default());
        let item = store.submit(&new_item(), &[NewFile { name: "details.txt".parse().unwrap(), content_type: "text/plain".into(), role: Role::File, bytes: b"file bytes" }]).await.unwrap();
        let id = item.id.to_string();
        let pending = cli(&scratch.url, &["pending", "--json"]);
        assert_eq!(pending.status.code(), Some(0));
        let items: Vec<Item> = serde_json::from_slice(&pending.stdout).unwrap();
        assert_eq!(items, vec![item.clone()]);
        assert_eq!(cli_item(&scratch.url, &["get", &id, "--json"]), item);
        assert_eq!(cli_item(&scratch.url, &["set-status", &id, "held"]).status, Status::Held);
        let queued = cli_item(&scratch.url, &["set-status", &id, "queued", "--ref", "IN-1"]);
        assert_eq!(queued.status, Status::Queued);
        assert!(matches!(queued.events.last(), Some(awna_intake::Event::Status { reference: Some(reference), .. }) if reference.as_str() == "IN-1"));
        assert_eq!(cli(&scratch.url, &["set-status", &id, "declined"]).status.code(), Some(3));
        let files = Files::new();
        let body = files.0.join("note.txt");
        std::fs::write(&body, "A comment.").unwrap();
        let annotated = cli_item(&scratch.url, &["event", &id, "--kind", "comment", "--body-file", body.to_str().unwrap()]);
        assert_eq!(annotated.events.len(), queued.events.len() + 1);
        let link = files.0.join("link.txt");
        symlink(&body, &link).unwrap();
        assert_eq!(cli(&scratch.url, &["event", &id, "--kind", "comment", "--body-file", link.to_str().unwrap()]).status.code(), Some(1));
        let export = files.0.join("export");
        let output = cli(&scratch.url, &["export", &id, export.to_str().unwrap()]);
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(String::from_utf8(output.stdout).unwrap(), format!("{}\n", export.display()));
        assert_eq!(std::fs::metadata(&export).unwrap().permissions().mode() & 0o777, 0o700);
        for path in [export.join("item.json"), export.join("0-details.txt")] {
            assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(std::fs::read(export.join("0-details.txt")).unwrap(), b"file bytes");
        assert_eq!(Item::from_json(&std::fs::read_to_string(export.join("item.json")).unwrap()).unwrap(), annotated);
        assert_eq!(cli(&scratch.url, &["export", &id, export.to_str().unwrap()]).status.code(), Some(1));
        assert_eq!(cli(&scratch.url, &["get", "IN-999", "--json"]).status.code(), Some(3));
    }).await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs AWNA_INTAKE_PG_TEST_ADMIN_URL"]
async fn snapshot_float_stays_within_one_ulp() {
    with_database(|scratch| async move {
        let store = PgStore::new(scratch.pool, Limits::default());
        let submitted = 1.0715660391465826e-75_f64;
        let mut input = new_item();
        input.snapshot = Some(awna_intake::Snapshot {
            state: serde_json::json!({ "number": submitted }),
        });
        let item = store.submit(&input, &[]).await.unwrap();
        let saved = store.get(item.id).await.unwrap();
        let returned = saved.snapshot.unwrap().state["number"].as_f64().unwrap();
        assert!(submitted.to_bits().abs_diff(returned.to_bits()) <= 1);
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs AWNA_INTAKE_PG_TEST_ADMIN_URL"]
async fn corrupt_attachment_is_refused_and_failed_export_is_removed() {
    with_database(|scratch| async move {
        let store = PgStore::new(scratch.pool.clone(), Limits::default());
        let item = store
            .submit(
                &new_item(),
                &[NewFile {
                    name: "details.txt".parse().unwrap(),
                    content_type: "text/plain".into(),
                    role: Role::File,
                    bytes: b"file bytes",
                }],
            )
            .await
            .unwrap();
        let id = i64::try_from(item.id.get()).unwrap();
        sqlx::query("UPDATE intake_attachments SET bytes = 'zz' WHERE item_id = $1")
            .bind(id)
            .execute(&scratch.pool)
            .await
            .unwrap();
        assert!(matches!(
            store.read_file(item.id, 0).await,
            Err(StoreError::Backend(PgError::Corrupt(
                "attachment size differs from metadata"
            )))
        ));
        let files = Files::new();
        let directory = files.0.join("export");
        let result = cli(
            &scratch.url,
            &["export", &item.id.to_string(), directory.to_str().unwrap()],
        );
        assert_eq!(result.status.code(), Some(1));
        assert!(!directory.exists(), "failed export left a directory");
        std::fs::create_dir(&directory).unwrap();
        let result = cli(
            &scratch.url,
            &["export", &item.id.to_string(), directory.to_str().unwrap()],
        );
        assert_eq!(result.status.code(), Some(1));
        assert!(directory.is_dir());
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs AWNA_INTAKE_PG_TEST_ADMIN_URL"]
async fn pending_is_bounded_and_oldest_first() {
    with_database(|scratch| async move {
        let store = PgStore::new(scratch.pool, Limits::default());
        let mut submitted = Vec::new();
        for _ in 0..101 {
            submitted.push(store.submit(&new_item(), &[]).await.unwrap());
        }
        let result = cli(&scratch.url, &["pending", "--json"]);
        assert_eq!(result.status.code(), Some(0));
        let returned: Vec<Item> = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(returned.len(), 100);
        assert_eq!(returned, submitted[..100]);
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs AWNA_INTAKE_PG_TEST_ADMIN_URL"]
async fn note_time_never_goes_backwards() {
    with_database(|scratch| async move {
        let store = PgStore::new(scratch.pool.clone(), Limits::default());
        let item = store.submit(&new_item(), &[]).await.unwrap();
        let item = store.add_note(item.id, &Note { kind: EventKind::new("comment").unwrap(), body: "Earlier.".into() }).await.unwrap();
        let future = item.updated_at_ms + 3_600_000;
        let id = i64::try_from(item.id.get()).unwrap();
        sqlx::query("UPDATE intake_items SET updated_at_ms = $2 WHERE id = $1")
            .bind(id).bind(i64::try_from(future).unwrap()).execute(&scratch.pool).await.unwrap();
        sqlx::query("UPDATE intake_events SET event = jsonb_set(event::jsonb, '{atMs}', to_jsonb($2::bigint))::json WHERE item_id = $1 AND seq = 1")
            .bind(id).bind(i64::try_from(future).unwrap()).execute(&scratch.pool).await.unwrap();
        let updated = store.add_note(item.id, &Note { kind: EventKind::new("comment").unwrap(), body: "Checked.".into() }).await.unwrap();
        assert_eq!(updated.updated_at_ms, future);
        assert!(matches!(updated.events.last(), Some(awna_intake::Event::Note { at_ms, .. }) if *at_ms == future));
        assert_eq!(store.get(item.id).await.unwrap(), updated);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs AWNA_INTAKE_PG_TEST_ADMIN_URL"]
async fn event_count_must_match_history() {
    with_database(|scratch| async move {
        let store = PgStore::new(scratch.pool.clone(), Limits::default());
        let item = store.submit(&new_item(), &[]).await.unwrap();
        sqlx::query("UPDATE intake_items SET event_count = event_count + 1 WHERE id = $1")
            .bind(i64::try_from(item.id.get()).unwrap())
            .execute(&scratch.pool)
            .await
            .unwrap();
        assert!(matches!(
            store.get(item.id).await,
            Err(StoreError::Backend(PgError::Corrupt(
                "event count differs from history"
            )))
        ));
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs AWNA_INTAKE_PG_TEST_ADMIN_URL"]
async fn database_error_redacts_table_column_and_value() {
    with_database(|scratch| async move {
        let store = PgStore::new(scratch.pool.clone(), Limits::default());
        let item = store.submit(&new_item(), &[]).await.unwrap();
        let error = sqlx::query("INSERT INTO intake_events (item_id, seq, event) SELECT item_id, seq, event FROM intake_events WHERE item_id = $1")
            .bind(i64::try_from(item.id.get()).unwrap()).execute(&scratch.pool).await.unwrap_err();
        assert!(matches!(&error, sqlx::Error::Database(_)));
        let error = PgError::from(error);
        for text in [error.to_string(), format!("{error:?}")] {
            assert_eq!(text, "database error (SQLSTATE 23505)");
            for forbidden in ["intake_events", "item_id", "seq", "Key", "already exists"] {
                assert!(!text.contains(forbidden));
            }
        }
    }).await;
}
