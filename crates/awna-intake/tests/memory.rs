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
