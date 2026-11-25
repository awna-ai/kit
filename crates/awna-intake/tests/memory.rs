use std::convert::Infallible;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};

use awna_intake::*;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
}

fn new_item() -> NewItem {
    NewItem {
        source: Source::new("web-form").unwrap(),
        requester: RequesterKey::new("someone@example.com").unwrap(),
        title: "The total on the summary page is wrong".into(),
        body: "The total should include the last row.".into(),
        snapshot: None,
    }
}

fn processed() -> Processed {
    Processed {
        triage: Triage {
            kind: TriageKind::Bug,
            size: TriageSize::Small,
            clear: true,
            summary: "The total omits a row".into(),
            details: String::new(),
        },
        proposal: Proposal {
            kind: ProposalKind::Tickets,
            tickets: vec![Ticket {
                title: "Include the last row".into(),
                spec: "Sum all the rows.".into(),
                list: TicketList::ToDo,
                blocked_by: vec![],
            }],
            reason: "The current total is incomplete.".into(),
        },
    }
}

struct Success {
    expected_files: usize,
    image_only: bool,
}

impl Processor for Success {
    type Error = Infallible;

    async fn process(&self, _item: &Item, files: &[File<'_>]) -> Result<Processed, Self::Error> {
        assert_eq!(files.len(), self.expected_files);
        if self.image_only {
            assert!(
                files
                    .iter()
                    .all(|file| file.attachment.content_type.starts_with("image/"))
            );
        }
        for file in files {
            assert_eq!(file.bytes.len() as u64, file.attachment.size);
        }
        Ok(processed())
    }
}

#[derive(Debug)]
struct Failed;
impl std::fmt::Display for Failed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("processing failed")
    }
}
impl std::error::Error for Failed {}
impl Processor for Failed {
    type Error = Failed;
    async fn process(&self, _item: &Item, _files: &[File<'_>]) -> Result<Processed, Self::Error> {
        Err(Failed)
    }
}

struct Invalid;
impl Processor for Invalid {
    type Error = Infallible;
    async fn process(&self, _item: &Item, _files: &[File<'_>]) -> Result<Processed, Self::Error> {
        let mut output = processed();
        output.proposal.tickets.clear();
        Ok(output)
    }
}

struct Never;
impl Processor for Never {
    type Error = Infallible;
    async fn process(&self, _item: &Item, _files: &[File<'_>]) -> Result<Processed, Self::Error> {
        panic!("a non-submitted item reached the processor")
    }
}

#[test]
fn memory_store_satisfies_the_store_contract() {
    block_on(conformance::run(&MemoryStore::new(conformance::limits())));
}

#[test]
fn processing_success_records_both_outputs_and_one_event() {
    block_on(async {
        let store = MemoryStore::default();
        let submitted = store.submit(&new_item(), &[]).await.unwrap();
        let item = process(
            &store,
            &Success {
                expected_files: 0,
                image_only: false,
            },
            submitted.id,
            Files::All,
        )
        .await
        .unwrap();
        assert_eq!(item.status, Status::Processed);
        assert_eq!(item.triage, Some(processed().triage));
        assert_eq!(item.proposal, Some(processed().proposal));
        assert_eq!(item.events.len(), 2);
        item.validate().unwrap();
    });
}

#[test]
fn processor_error_leaves_the_submission_unchanged() {
    block_on(async {
        let store = MemoryStore::default();
        let submitted = store.submit(&new_item(), &[]).await.unwrap();
        assert!(matches!(
            process(&store, &Failed, submitted.id, Files::All).await,
            Err(ProcessError::Processor(Failed))
        ));
        assert_eq!(store.get(submitted.id).await.unwrap(), submitted);
    });
}

#[test]
fn invalid_processor_output_leaves_the_submission_unchanged() {
    block_on(async {
        let store = MemoryStore::default();
        let submitted = store.submit(&new_item(), &[]).await.unwrap();
        assert!(matches!(
            process(&store, &Invalid, submitted.id, Files::None).await,
            Err(ProcessError::Invalid(_))
        ));
        assert_eq!(store.get(submitted.id).await.unwrap(), submitted);
    });
}

#[test]
fn non_submitted_items_never_reach_the_processor() {
    block_on(async {
        let store = MemoryStore::default();
        let submitted = store.submit(&new_item(), &[]).await.unwrap();
        let held = store
            .set_status(submitted.id, Status::Held, None)
            .await
            .unwrap();
        assert!(matches!(
            process(&store, &Never, held.id, Files::None).await,
            Err(ProcessError::Store(StoreError::IllegalMove(IllegalMove {
                from: Status::Held,
                to: Status::Processed
            })))
        ));
        assert_eq!(store.get(held.id).await.unwrap(), held);
    });
}

#[test]
fn file_selection_passes_all_images_or_none() {
    block_on(async {
        for (selection, count, image_only) in [
            (Files::All, 2, false),
            (Files::Images, 1, true),
            (Files::None, 0, false),
        ] {
            let store = MemoryStore::default();
            let files = [
                NewFile {
                    name: AttachmentName::new("screen.png").unwrap(),
                    content_type: "image/png".into(),
                    role: Role::Screenshot,
                    bytes: b"image",
                },
                NewFile {
                    name: AttachmentName::new("notes.txt").unwrap(),
                    content_type: "text/plain".into(),
                    role: Role::File,
                    bytes: b"notes",
                },
            ];
            let item = store.submit(&new_item(), &files).await.unwrap();
            process(
                &store,
                &Success {
                    expected_files: count,
                    image_only,
                },
                item.id,
                selection,
            )
            .await
            .unwrap();
        }
    });
}

#[test]
fn concurrent_status_changes_compare_and_write_atomically() {
    let store = MemoryStore::default();
    let item = block_on(store.submit(&new_item(), &[])).unwrap();
    let successes = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                if block_on(store.set_status(item.id, Status::Held, None)).is_ok() {
                    successes.fetch_add(1, Ordering::SeqCst);
                }
            });
        }
    });
    assert_eq!(successes.load(Ordering::SeqCst), 1);
    assert_eq!(block_on(store.get(item.id)).unwrap().events.len(), 2);
}

struct InvalidItemStore(MemoryStore);
impl Store for InvalidItemStore {
    type Error = Infallible;
    fn limits(&self) -> &Limits {
        self.0.limits()
    }
    async fn submit(
        &self,
        item: &NewItem,
        files: &[NewFile<'_>],
    ) -> Result<Item, StoreError<Self::Error>> {
        self.0.submit(item, files).await
    }
    async fn get(&self, id: ItemId) -> Result<Item, StoreError<Self::Error>> {
        let mut item = self.0.get(id).await?;
        item.title.clear();
        Ok(item)
    }
    async fn list(&self, filter: &Filter) -> Result<Vec<Item>, StoreError<Self::Error>> {
        self.0.list(filter).await
    }
    async fn record_processing(
        &self,
        id: ItemId,
        processed: &Processed,
    ) -> Result<Item, StoreError<Self::Error>> {
        self.0.record_processing(id, processed).await
    }
    async fn set_status(
        &self,
        id: ItemId,
        to: Status,
        reference: Option<&Reference>,
    ) -> Result<Item, StoreError<Self::Error>> {
        self.0.set_status(id, to, reference).await
    }
    async fn add_note(&self, id: ItemId, note: &Note) -> Result<Item, StoreError<Self::Error>> {
        self.0.add_note(id, note).await
    }
    async fn read_file(&self, id: ItemId, index: u32) -> Result<Vec<u8>, StoreError<Self::Error>> {
        self.0.read_file(id, index).await
    }
}

#[test]
fn invalid_stored_items_never_reach_the_processor() {
    block_on(async {
        let store = InvalidItemStore(MemoryStore::default());
        let item = store.submit(&new_item(), &[]).await.unwrap();
        assert!(matches!(
            process(&store, &Never, item.id, Files::None).await,
            Err(ProcessError::Invalid(_))
        ));
        assert_eq!(store.0.get(item.id).await.unwrap(), item);
    });
}

#[test]
fn file_debug_reports_lengths_without_bytes() {
    let bytes = [231, 232, 233];
    let new_file = NewFile {
        name: AttachmentName::new("bytes.txt").unwrap(),
        content_type: "text/plain".into(),
        role: Role::File,
        bytes: &bytes,
    };
    let attachment = Attachment {
        index: 0,
        name: new_file.name.clone(),
        content_type: new_file.content_type.clone(),
        role: Role::File,
        size: 3,
    };
    for debug in [
        format!("{new_file:?}"),
        format!(
            "{:?}",
            File {
                attachment: &attachment,
                bytes: &bytes
            }
        ),
    ] {
        for value in ["231", "232", "233"] {
            assert!(!debug.contains(value), "{debug}");
        }
        assert!(debug.contains("bytes.txt"));
        assert!(debug.contains("text/plain"));
        assert!(debug.contains("File"));
        assert!(debug.contains('3'));
    }
}

#[test]
fn missing_items_take_precedence_over_invalid_inputs() {
    block_on(async {
        let store = MemoryStore::default();
        let missing = ItemId::new(u64::MAX).unwrap();
        let mut invalid = processed();
        invalid.proposal.tickets.clear();
        assert!(
            matches!(store.record_processing(missing, &invalid).await, Err(StoreError::NotFound(id)) if id == missing)
        );
        let note = Note {
            kind: EventKind::new("review").unwrap(),
            body: "bad\rtext".into(),
        };
        assert!(
            matches!(store.add_note(missing, &note).await, Err(StoreError::NotFound(id)) if id == missing)
        );
        assert!(
            matches!(store.set_status(missing, Status::Processed, None).await, Err(StoreError::NotFound(id)) if id == missing)
        );
        let held = store.submit(&new_item(), &[]).await.unwrap();
        store.set_status(held.id, Status::Held, None).await.unwrap();
        assert!(matches!(
            store.record_processing(held.id, &invalid).await,
            Err(StoreError::Invalid(_))
        ));
    });
}

#[test]
fn image_selection_excludes_scriptable_images_in_custom_stores() {
    block_on(async {
        let mut limits = Limits::default();
        limits.allowed.push(AllowedType {
            content_type: "image/svg+xml".into(),
            extensions: vec!["svg".into()],
        });
        let store = MemoryStore::new(limits);
        let files: Vec<_> = [
            ("vector.svg", "image/svg+xml"),
            ("screen.png", "image/png"),
            ("photo.jpg", "image/jpeg"),
            ("motion.gif", "image/gif"),
            ("picture.webp", "image/webp"),
            ("photo.heic", "image/heic"),
        ]
        .into_iter()
        .map(|(name, content_type)| NewFile {
            name: AttachmentName::new(name).unwrap(),
            content_type: content_type.into(),
            role: Role::File,
            bytes: b"x",
        })
        .collect();
        let item = store.submit(&new_item(), &files).await.unwrap();
        process(
            &store,
            &Success {
                expected_files: 5,
                image_only: true,
            },
            item.id,
            Files::Images,
        )
        .await
        .unwrap();
    });
}

#[test]
fn duplicate_submission_names_are_refused_after_ascii_lowercasing() {
    let files = ["a.txt", "A.TXT"].map(|name| NewFile {
        name: AttachmentName::new(name).unwrap(),
        content_type: "text/plain".into(),
        role: Role::File,
        bytes: b"x",
    });
    let error = Limits::default().check(&files).unwrap_err();
    assert_eq!(error.field, "files[1].name");
    assert_eq!(error.reason, "duplicate attachment name");
}

#[test]
fn invalid_processing_display_covers_items_and_output() {
    let error: ProcessError<Infallible, Infallible> = ProcessError::Invalid(ValidationError {
        field: "title".into(),
        reason: "invalid text",
    });
    assert_eq!(error.to_string(), "invalid item or processor output");
}
