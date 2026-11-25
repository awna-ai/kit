//! Reusable behavioral checks for store implementations on an empty scratch store.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    Event, EventKind, Filter, Item, ItemId, Limits, NewFile, NewItem, Note, Processed, Proposal,
    ProposalKind, Reference, Role, Snapshot, Status, Store, StoreError, Ticket, TicketList, Triage,
    TriageKind, TriageSize,
};

/// The small attachment limits required by [`run`].
pub fn limits() -> Limits {
    Limits {
        max_files: 3,
        max_file_bytes: 1024,
        max_item_bytes: 2048,
        ..Limits::default()
    }
}

fn new_item() -> NewItem {
    NewItem {
        source: "web-form".parse().expect("valid source"),
        requester: "someone@example.com".parse().expect("valid requester"),
        title: "The total on the summary page is wrong".into(),
        body: "The entries add up to seven.".into(),
        snapshot: None,
    }
}

fn processed() -> Processed {
    Processed {
        triage: Triage {
            kind: TriageKind::Bug,
            size: TriageSize::Small,
            clear: true,
            summary: "The sum differs from the entries".into(),
            details: String::new(),
        },
        proposal: Proposal {
            kind: ProposalKind::Tickets,
            reason: "Correct the calculation.".into(),
            tickets: vec![Ticket {
                title: "Correct the total".into(),
                spec: "Sum the entries.".into(),
                list: TicketList::ToDo,
                blocked_by: vec![],
            }],
        },
    }
}

fn file<'a>(name: &str, content_type: &str, role: Role, bytes: &'a [u8]) -> NewFile<'a> {
    NewFile {
        name: name.parse().expect("valid file name"),
        content_type: content_type.into(),
        role,
        bytes,
    }
}

fn valid(item: Item) -> Item {
    item.validate().expect("every returned item must validate");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("current Unix time")
        .as_millis();
    for event in &item.events {
        assert!(
            now.abs_diff(u128::from(event.at_ms())) <= 86_400_000,
            "event time must be within one day of now"
        );
    }
    item
}

fn changed(before: &Item, after: Item) -> Item {
    let after = valid(after);
    assert!(
        after.updated_at_ms >= before.updated_at_ms,
        "item time must never decrease"
    );
    assert_eq!(after.id, before.id, "a change must preserve the item id");
    assert_eq!(
        after.created_at_ms, before.created_at_ms,
        "a change must preserve the creation time"
    );
    assert_eq!(
        after.events.last().expect("nonempty history").at_ms(),
        after.updated_at_ms,
        "a change must stamp its new event and update time together"
    );
    after
}

async fn assert_saved<S: Store + Sync>(store: &S, item: &Item) {
    assert_eq!(
        valid(store.get(item.id).await.expect("get after write")),
        *item,
        "get must return exactly the item returned by the write"
    );
}

async fn count<S: Store + Sync>(store: &S) -> usize {
    store
        .list(&Filter::default())
        .await
        .expect("list must succeed")
        .into_iter()
        .map(valid)
        .count()
}

async fn refused<S: Store + Sync>(store: &S, item: &NewItem, files: &[NewFile<'_>]) {
    let before = count(store).await;
    assert!(
        matches!(store.submit(item, files).await, Err(StoreError::Invalid(_))),
        "invalid submission must return Invalid"
    );
    assert_eq!(
        count(store).await,
        before,
        "invalid submission must store nothing"
    );
}

async fn at_status<S: Store + Sync>(store: &S, status: Status) -> Item {
    let mut item = valid(
        store
            .submit(&new_item(), &[])
            .await
            .expect("submit path item"),
    );
    assert_saved(store, &item).await;
    let path: &[Status] = match status {
        Status::Submitted => &[],
        Status::Processed => &[Status::Processed],
        Status::Held => &[Status::Held],
        Status::Queued => &[Status::Held, Status::Queued],
        Status::InProgress => &[Status::Held, Status::Queued, Status::InProgress],
        Status::Built => &[
            Status::Held,
            Status::Queued,
            Status::InProgress,
            Status::Built,
        ],
        Status::Shipped => &[
            Status::Held,
            Status::Queued,
            Status::InProgress,
            Status::Built,
            Status::Shipped,
        ],
        Status::Declined => &[Status::Held, Status::Declined],
        Status::Cancelled => &[Status::Cancelled],
    };
    for &to in path {
        let next = if to == Status::Processed {
            store
                .record_processing(item.id, &processed())
                .await
                .expect("processing path")
        } else {
            store
                .set_status(item.id, to, None)
                .await
                .expect("legal status path")
        };
        item = changed(&item, next);
        assert_saved(store, &item).await;
        if to != Status::Processed {
            assert!(
                matches!(
                    item.events.last(),
                    Some(Event::Status {
                        reference: None,
                        ..
                    })
                ),
                "None must store a null reference"
            );
        }
    }
    assert_eq!(item.status, status, "path must reach requested status");
    item
}

/// Check the contract against an empty store constructed with [`limits`].
///
/// Panics with the broken rule when any store operation violates the contract. Run
/// this against a throwaway store: it deliberately leaves many sample items behind.
pub async fn run<S: Store + Sync>(store: &S) {
    assert_eq!(
        store.limits(),
        &limits(),
        "store must use conformance limits"
    );
    assert_eq!(count(store).await, 0, "conformance store must start empty");
    let files = [
        file("screen.png", "image/png", Role::Screenshot, b"png"),
        file("details.txt", "text/plain", Role::File, b"details"),
    ];
    let first = valid(
        store
            .submit(&new_item(), &files)
            .await
            .expect("valid submit"),
    );
    assert_eq!(first.id.get(), 1, "ids must start at one");
    assert_eq!(first.status, Status::Submitted, "submit status");
    assert_eq!(
        first.attachments.len(),
        2,
        "store must retain attachment metadata"
    );
    for (index, (attachment, input)) in first.attachments.iter().zip(&files).enumerate() {
        assert_eq!(
            attachment.index as usize, index,
            "attachment submission order"
        );
        assert_eq!(attachment.name, input.name, "attachment name");
        assert_eq!(
            attachment.content_type, input.content_type,
            "attachment content type"
        );
        assert_eq!(attachment.role, input.role, "attachment role");
        assert_eq!(attachment.size, input.bytes.len() as u64, "attachment size");
        assert_eq!(
            store
                .read_file(first.id, attachment.index)
                .await
                .expect("read file"),
            input.bytes,
            "file bytes must be unchanged"
        );
    }
    assert_eq!(
        first.events,
        vec![Event::Status {
            at_ms: first.created_at_ms,
            from: None,
            to: Status::Submitted,
            reference: None
        }],
        "submission must have exactly its initial status event"
    );
    assert_eq!(
        first.created_at_ms, first.updated_at_ms,
        "submission timestamps must match"
    );
    assert_eq!(
        valid(store.get(first.id).await.expect("get submitted")),
        first,
        "get returns stored item"
    );

    let mut fidelity = new_item();
    fidelity.title = format!("{}😀", "é".repeat(98));
    fidelity.body = format!("{}😀e\u{301}\n\t ", "é".repeat(32763));
    let state = serde_json::json!({"nested":{"values":[7,1.25,0.1,-1,9007199254740993_u64,true,null,"é雪😀"]}});
    fidelity.snapshot = Some(Snapshot {
        state: state.clone(),
    });
    let binary = [0x00, 0xff, 0xc3, 0x28];
    let exact = valid(
        store
            .submit(
                &fidelity,
                &[file("Report-Final.TXT", "text/plain", Role::File, &binary)],
            )
            .await
            .expect("fidelity submit"),
    );
    assert_eq!(exact.source, fidelity.source, "source fidelity");
    assert_eq!(exact.requester, fidelity.requester, "requester fidelity");
    assert_eq!(exact.title, fidelity.title, "multi-byte title fidelity");
    assert_eq!(
        exact.body, fidelity.body,
        "block byte and whitespace fidelity"
    );
    assert_eq!(
        exact.snapshot.as_ref().expect("snapshot retained").state,
        state,
        "JSON value fidelity"
    );
    assert_eq!(
        exact.attachments[0].name.as_str(),
        "Report-Final.TXT",
        "attachment name case fidelity"
    );
    let encoded = serde_json::to_string(&state).expect("serialize snapshot");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&encoded).expect("parse snapshot"),
        state,
        "snapshot numbers must survive JSON without optional features"
    );
    assert_eq!(
        state["nested"]["values"][4].as_u64(),
        Some(9007199254740993)
    );
    assert_saved(store, &exact).await;
    assert_eq!(
        store.read_file(exact.id, 0).await.expect("binary fidelity"),
        binary,
        "non-UTF-8 bytes must be unchanged"
    );
    assert_eq!(
        store
            .read_file(first.id, 0)
            .await
            .expect("first file again"),
        b"png",
        "file keys must include item id"
    );

    let mut too_deep = new_item();
    let mut state = serde_json::Value::Null;
    for _ in 0..33 {
        state = serde_json::json!([state]);
    }
    too_deep.snapshot = Some(Snapshot { state });
    refused(store, &too_deep, &[]).await;
    let mut too_long = new_item();
    too_long.body = "x".repeat(65537);
    refused(store, &too_long, &[]).await;
    refused(
        store,
        &new_item(),
        &[
            file("same.txt", "text/plain", Role::File, b"x"),
            file("SAME.TXT", "text/plain", Role::File, b"y"),
        ],
    )
    .await;

    for title in ["", "bad\u{001f}title", "bad\u{202e}title"] {
        let mut item = new_item();
        item.title = title.into();
        refused(store, &item, &[]).await;
    }
    let bytes = vec![1; 1025];
    refused(
        store,
        &new_item(),
        &[file("large.txt", "text/plain", Role::File, &bytes)],
    )
    .await;
    let small = [1; 1024];
    refused(
        store,
        &new_item(),
        &[
            file("a.txt", "text/plain", Role::File, &small),
            file("b.txt", "text/plain", Role::File, &bytes),
        ],
    )
    .await;
    // This total exceeds the item cap while every file is individually within its cap.
    refused(
        store,
        &new_item(),
        &[
            file("a.txt", "text/plain", Role::File, &small),
            file("b.txt", "text/plain", Role::File, &small),
            file("c.txt", "text/plain", Role::File, b"x"),
        ],
    )
    .await;
    refused(
        store,
        &new_item(),
        &[
            file("a.txt", "text/plain", Role::File, b"x"),
            file("b.txt", "text/plain", Role::File, b"x"),
            file("c.txt", "text/plain", Role::File, b"x"),
            file("d.txt", "text/plain", Role::File, b"x"),
        ],
    )
    .await;
    for files in [
        vec![file("vector.svg", "image/svg+xml", Role::File, b"x")],
        vec![file("page.html", "text/plain", Role::File, b"x")],
        vec![file("page.pdf", "application/pdf", Role::Screenshot, b"x")],
        vec![
            file("one.png", "image/png", Role::Screenshot, b"x"),
            file("two.jpg", "image/jpeg", Role::Screenshot, b"x"),
        ],
        vec![file("empty.txt", "text/plain", Role::File, b"")],
    ] {
        refused(store, &new_item(), &files).await;
    }

    let output = processed();
    let completed = changed(
        &first,
        store
            .record_processing(first.id, &output)
            .await
            .expect("record processing"),
    );
    assert_saved(store, &completed).await;
    assert_eq!(
        completed.status,
        Status::Processed,
        "processing must move to processed"
    );
    assert_eq!(
        completed.triage.as_ref(),
        Some(&output.triage),
        "processing must retain triage"
    );
    assert_eq!(
        completed.proposal.as_ref(),
        Some(&output.proposal),
        "processing must retain proposal"
    );
    assert!(
        matches!(store.record_processing(first.id, &output).await,
        Err(StoreError::IllegalMove(error)) if error.from == Status::Processed && error.to == Status::Processed),
        "processing twice must be illegal"
    );
    let pending = at_status(store, Status::Submitted).await;
    let mut bad = processed();
    bad.proposal.tickets.clear();
    assert!(
        matches!(
            store.record_processing(pending.id, &bad).await,
            Err(StoreError::Invalid(_))
        ),
        "invalid processing must return Invalid"
    );
    assert_eq!(
        valid(store.get(pending.id).await.expect("unchanged item")),
        pending,
        "invalid processing must change nothing"
    );

    let reference: Reference = "work-19".parse().expect("valid reference");
    for from in Status::ALL {
        if from != Status::Submitted {
            let before = at_status(store, from).await;
            assert!(
                matches!(store.record_processing(before.id, &output).await,
                Err(StoreError::IllegalMove(error)) if error.from == from && error.to == Status::Processed),
                "record_processing from every non-submitted status is illegal"
            );
            assert_saved(store, &before).await;
        }
        for to in Status::ALL {
            let before = at_status(store, from).await;
            let result = store.set_status(before.id, to, Some(&reference)).await;
            if to == Status::Processed {
                assert!(
                    matches!(result, Err(StoreError::Invalid(ref error)) if error.field == "to" && error.reason == "only record_processing moves an item to processed"),
                    "set_status cannot move to processed"
                );
            } else if from.can_move_to(to) {
                let after = changed(&before, result.expect("table-legal move must succeed"));
                assert_saved(store, &after).await;
                assert_eq!(after.status, to, "move must set requested status");
                assert_eq!(
                    after.events.len(),
                    before.events.len() + 1,
                    "move appends one event"
                );
                assert!(
                    matches!(after.events.last(), Some(Event::Status { from: Some(actual_from), to: actual_to, reference: Some(actual_ref), .. }) if *actual_from == from && *actual_to == to && actual_ref == &reference),
                    "move event must record from, to and reference"
                );
            } else {
                assert!(
                    matches!(result, Err(StoreError::IllegalMove(ref error)) if error.from == from && error.to == to),
                    "table-illegal move must return IllegalMove"
                );
            }
            if to == Status::Processed || !from.can_move_to(to) {
                assert_eq!(
                    valid(store.get(before.id).await.expect("refused move item")),
                    before,
                    "refused move must change nothing"
                );
            }
        }
    }

    let referenced = at_status(store, Status::Submitted).await;
    let referenced = changed(
        &referenced,
        store
            .set_status(referenced.id, Status::Held, Some(&reference))
            .await
            .expect("set reference"),
    );
    assert_saved(store, &referenced).await;
    let cleared = changed(
        &referenced,
        store
            .set_status(referenced.id, Status::Queued, None)
            .await
            .expect("clear reference"),
    );
    assert_saved(store, &cleared).await;
    assert!(
        matches!(
            cleared.events.last(),
            Some(Event::Status {
                reference: None,
                ..
            })
        ),
        "None must store ref null after a referenced move"
    );

    let terminal = at_status(store, Status::Shipped).await;
    let note = Note {
        kind: EventKind::new("review").expect("valid note kind"),
        body: "\n\t Verified the release.\n\t ".into(),
    };
    let annotated = changed(
        &terminal,
        store
            .add_note(terminal.id, &note)
            .await
            .expect("terminal note"),
    );
    assert_saved(store, &annotated).await;
    assert_eq!(
        annotated.status,
        Status::Shipped,
        "note must not change status"
    );
    assert_eq!(
        annotated.events.len(),
        terminal.events.len() + 1,
        "note appends exactly one event"
    );
    assert!(
        matches!(annotated.events.last(), Some(Event::Note { kind, body, .. }) if kind == &note.kind && body == &note.body),
        "note must be the last event"
    );
    assert!(
        EventKind::new("status").is_err(),
        "status is reserved for status events"
    );
    let bad_note = Note {
        kind: note.kind.clone(),
        body: "bad\rnote".into(),
    };
    assert!(
        matches!(
            store.add_note(terminal.id, &bad_note).await,
            Err(StoreError::Invalid(_))
        ),
        "invalid note must return Invalid"
    );
    assert_eq!(
        valid(store.get(terminal.id).await.expect("unchanged note item")),
        annotated,
        "invalid note must change nothing"
    );

    let mut full = at_status(store, Status::Submitted).await;
    while full.events.len() < 256 {
        full = changed(
            &full,
            store.add_note(full.id, &note).await.expect("fill history"),
        );
    }
    assert_saved(store, &full).await;
    assert!(
        matches!(store.add_note(full.id, &note).await, Err(StoreError::Invalid(error))
            if error.field == "events" && error.reason == "event history is full"),
        "the 257th event must return Invalid with the history error"
    );
    assert_saved(store, &full).await;

    let missing = ItemId::new(u64::MAX).expect("nonzero id");
    assert!(
        matches!(store.get(missing).await, Err(StoreError::NotFound(id)) if id == missing),
        "missing get"
    );
    assert!(
        matches!(store.set_status(missing, Status::Held, None).await, Err(StoreError::NotFound(id)) if id == missing),
        "missing set_status"
    );
    assert!(
        matches!(store.add_note(missing, &note).await, Err(StoreError::NotFound(id)) if id == missing),
        "missing add_note"
    );
    assert!(
        matches!(store.read_file(missing, 0).await, Err(StoreError::NotFound(id)) if id == missing),
        "missing read_file"
    );
    assert!(
        matches!(store.record_processing(missing, &output).await, Err(StoreError::NotFound(id)) if id == missing),
        "missing record_processing"
    );
    assert!(
        matches!(store.record_processing(missing, &bad).await, Err(StoreError::NotFound(id)) if id == missing),
        "missing processing precedes invalid input"
    );
    assert!(
        matches!(store.add_note(missing, &bad_note).await, Err(StoreError::NotFound(id)) if id == missing),
        "missing note precedes invalid input"
    );
    assert!(
        matches!(store.set_status(missing, Status::Processed, None).await, Err(StoreError::NotFound(id)) if id == missing),
        "missing status precedes invalid input"
    );
    assert!(
        matches!(store.read_file(first.id, 2).await, Err(StoreError::NoFile { id, index: 2 }) if id == first.id),
        "missing attachment index"
    );

    let mut other = new_item();
    other.source = "mailbox".parse().expect("valid source");
    other.requester = "other@example.com".parse().expect("valid requester");
    let other = valid(
        store
            .submit(&other, &[])
            .await
            .expect("different source item"),
    );
    assert_saved(store, &other).await;
    let all: Vec<_> = store
        .list(&Filter::default())
        .await
        .expect("all items")
        .into_iter()
        .map(valid)
        .collect();
    assert!(
        all.windows(2).all(|pair| pair[0].id < pair[1].id),
        "list must ascend by id"
    );
    let filters = [
        Filter {
            statuses: vec![Status::Submitted],
            ..Filter::default()
        },
        Filter {
            source: Some(other.source.clone()),
            ..Filter::default()
        },
        Filter {
            requester: Some(other.requester.clone()),
            ..Filter::default()
        },
        Filter {
            statuses: vec![Status::Submitted],
            source: Some(other.source.clone()),
            requester: Some(other.requester.clone()),
            limit: Some(1),
        },
        Filter {
            limit: Some(2),
            ..Filter::default()
        },
        Filter {
            limit: Some(0),
            ..Filter::default()
        },
        Filter::pending(),
    ];
    for filter in filters {
        let mut expected: Vec<_> = all
            .iter()
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
            .cloned()
            .collect();
        if let Some(limit) = filter.limit {
            expected.truncate(limit as usize);
        }
        let actual: Vec<_> = store
            .list(&filter)
            .await
            .expect("filtered list")
            .into_iter()
            .map(valid)
            .collect();
        assert_eq!(
            actual, expected,
            "list must apply every filter and limit in ascending order"
        );
    }
    assert_eq!(
        Filter::pending().statuses,
        vec![Status::Submitted, Status::Processed],
        "pending means waiting to be routed"
    );
}
