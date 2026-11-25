use awna_intake::{EventKind, ItemId, Reference, Status};

pub const USAGE: &str = "Usage: awna-intake --db-env <NAME> pending --json\n       awna-intake --db-env <NAME> get <id> --json\n       awna-intake --db-env <NAME> export <id> <dir>\n       awna-intake --db-env <NAME> set-status <id> <status> [--ref <text>]\n       awna-intake --db-env <NAME> event <id> --kind <kind> --body-file <file>\n       awna-intake --help | -h";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Database { variable: String, action: Action },
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Pending,
    Get(ItemId),
    Export(ItemId, String),
    SetStatus(ItemId, Status, Option<Reference>),
    Event(ItemId, EventKind, String),
}

pub fn parse(args: Vec<String>) -> Result<Command, ()> {
    let words = args.iter().map(String::as_str).collect::<Vec<_>>();
    if matches!(words.as_slice(), ["--help"] | ["-h"]) {
        return Ok(Command::Help);
    }
    let ["--db-env", variable, rest @ ..] = words.as_slice() else {
        return Err(());
    };
    if !valid_variable(variable) {
        return Err(());
    }
    let action = match rest {
        ["pending", "--json"] => Action::Pending,
        ["get", id, "--json"] => Action::Get(id.parse().map_err(|_| ())?),
        ["export", id, directory] if !directory.starts_with("--") => {
            Action::Export(id.parse().map_err(|_| ())?, (*directory).into())
        }
        ["set-status", id, status] => Action::SetStatus(
            id.parse().map_err(|_| ())?,
            status.parse().map_err(|_| ())?,
            None,
        ),
        ["set-status", id, status, "--ref", reference] => Action::SetStatus(
            id.parse().map_err(|_| ())?,
            status.parse().map_err(|_| ())?,
            Some(Reference::new(*reference).map_err(|_| ())?),
        ),
        ["event", id, first, first_value, second, second_value] => {
            let (kind, file) = match (*first, *second) {
                ("--kind", "--body-file") => (*first_value, *second_value),
                ("--body-file", "--kind") => (*second_value, *first_value),
                _ => return Err(()),
            };
            if file.starts_with("--") {
                return Err(());
            }
            Action::Event(
                id.parse().map_err(|_| ())?,
                EventKind::new(kind).map_err(|_| ())?,
                file.into(),
            )
        }
        _ => return Err(()),
    };
    Ok(Command::Database {
        variable: (*variable).into(),
        action,
    })
}

fn valid_variable(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_uppercase() || (index > 0 && byte.is_ascii_digit())
        })
}

pub struct Failure {
    pub code: u8,
    pub message: String,
}

impl Failure {
    fn file(message: &str) -> Self {
        Self {
            code: 1,
            message: message.into(),
        }
    }
}

fn storage<E: std::fmt::Display>(error: awna_intake::StoreError<E>) -> Failure {
    let code = if matches!(error, awna_intake::StoreError::Backend(_)) {
        1
    } else {
        3
    };
    Failure {
        code,
        message: error.to_string(),
    }
}

pub async fn run(variable: &str, action: Action) -> Result<String, Failure> {
    use awna_intake::{Limits, Store};
    let url = std::env::var(variable)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Failure::file(&format!("{variable} is not set")))?;
    let options =
        awna_intake_pg::connect_options(&url).map_err(|error| Failure::file(&error.to_string()))?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect_with(options)
        .await
        .map_err(|error| Failure::file(&awna_intake_pg::PgError::from(error).to_string()))?;
    let store = awna_intake_pg::PgStore::new(pool, Limits::default());
    let result = match action {
        Action::Pending => {
            let items = store
                .list(&awna_intake::Filter::pending())
                .await
                .map_err(storage)?;
            serde_json::to_string(&items)
                .map_err(|_| Failure::file("items could not be serialized"))
        }
        Action::Get(id) => item_json(&store.get(id).await.map_err(storage)?),
        Action::SetStatus(id, status, reference) => item_json(
            &store
                .set_status(id, status, reference.as_ref())
                .await
                .map_err(storage)?,
        ),
        Action::Event(id, kind, file) => {
            let body = read_body(std::path::Path::new(&file))?;
            item_json(
                &store
                    .add_note(id, &awna_intake::Note { kind, body })
                    .await
                    .map_err(storage)?,
            )
        }
        Action::Export(id, directory) => {
            let item = store.get(id).await.map_err(storage)?;
            export(&store, &item, std::path::Path::new(&directory)).await?;
            Ok(directory)
        }
    };
    store.pool().close().await;
    result
}

fn item_json(item: &awna_intake::Item) -> Result<String, Failure> {
    serde_json::to_string(item).map_err(|_| Failure::file("item could not be serialized"))
}

fn read_body(path: &std::path::Path) -> Result<String, Failure> {
    use std::{io::Read, os::unix::fs::OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| Failure::file("body file could not be opened"))?;
    if !file
        .metadata()
        .map_err(|_| Failure::file("body file could not be checked"))?
        .is_file()
    {
        return Err(Failure::file("body file is not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::file("body file could not be read"))?;
    if bytes.len() > 1_048_576 {
        return Err(Failure::file("body file exceeds 1 MiB"));
    }
    String::from_utf8(bytes).map_err(|_| Failure::file("body file is not UTF-8"))
}

async fn export(
    store: &awna_intake_pg::PgStore,
    item: &awna_intake::Item,
    directory: &std::path::Path,
) -> Result<(), Failure> {
    use awna_intake::Store;
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(directory)
        .map_err(|_| Failure::file("export directory could not be created"))?;
    let json = item_json(item)? + "\n";
    write_private(&directory.join("item.json"), json.as_bytes())
        .map_err(|_| Failure::file("export item.json could not be written"))?;
    for file in &item.attachments {
        let fail = || Failure::file(&format!("export file {} failed", file.index));
        let name = file.name.as_str();
        if name.contains('/') || name.contains('\0') || matches!(name, "." | "..") {
            return Err(fail());
        }
        let bytes = store
            .read_file(item.id, file.index)
            .await
            .map_err(|_| fail())?;
        write_private(&directory.join(format!("{}-{name}", file.index)), &bytes)
            .map_err(|_| fail())?;
    }
    Ok(())
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn accepts_each_command() {
        for text in [
            "--help",
            "-h",
            "--db-env DB pending --json",
            "--db-env DB get IN-1 --json",
            "--db-env DB export IN-1 output",
            "--db-env DB set-status IN-1 held",
            "--db-env DB set-status IN-1 queued --ref IN-2",
            "--db-env DB event IN-1 --kind comment --body-file note.txt",
            "--db-env DB event IN-1 --body-file note.txt --kind comment",
        ] {
            assert!(parse(argv(text)).is_ok(), "{text}");
        }
    }

    #[test]
    fn parsed_values_keep_their_types_and_text() {
        let id = ItemId::new(27).unwrap();
        assert_eq!(
            parse(argv("--db-env _DB9 get IN-27 --json")),
            Ok(Command::Database {
                variable: "_DB9".into(),
                action: Action::Get(id)
            })
        );
        assert_eq!(
            parse(vec![
                "--db-env".into(),
                "DB".into(),
                "set-status".into(),
                "IN-27".into(),
                "queued".into(),
                "--ref".into(),
                "work 12".into()
            ]),
            Ok(Command::Database {
                variable: "DB".into(),
                action: Action::SetStatus(
                    id,
                    Status::Queued,
                    Some(Reference::new("work 12").unwrap())
                )
            })
        );
        assert_eq!(
            parse(argv(
                "--db-env DB event IN-27 --body-file note.txt --kind comment"
            )),
            Ok(Command::Database {
                variable: "DB".into(),
                action: Action::Event(id, EventKind::new("comment").unwrap(), "note.txt".into())
            })
        );
        assert!(
            parse(vec![
                "--db-env".into(),
                "A".repeat(64),
                "pending".into(),
                "--json".into()
            ])
            .is_ok()
        );
    }

    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new() -> Self {
            static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "intake_cli_{}_{}",
                std::process::id(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn body_reader_checks_opened_files_and_bounds() {
        use std::os::unix::fs::symlink;
        let scratch = Scratch::new();
        let path = scratch.0.join("body");
        std::fs::write(&path, "text\n").unwrap();
        assert_eq!(read_body(&path).ok().as_deref(), Some("text\n"));
        let link = scratch.0.join("link");
        symlink(&path, &link).unwrap();
        assert_eq!(read_body(&link).err().unwrap().code, 1);
        assert!(read_body(&scratch.0).is_err());
        std::fs::write(&path, [255]).unwrap();
        assert!(read_body(&path).is_err());
        std::fs::write(&path, vec![b'a'; 1_048_576]).unwrap();
        assert_eq!(read_body(&path).ok().unwrap().len(), 1_048_576);
        std::fs::write(&path, vec![b'a'; 1_048_577]).unwrap();
        assert!(read_body(&path).is_err());
    }

    #[test]
    fn private_writer_refuses_replacement_and_links() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let scratch = Scratch::new();
        let path = scratch.0.join("data");
        write_private(&path, b"first").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(write_private(&path, b"second").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        let link = scratch.0.join("link");
        symlink(&path, &link).unwrap();
        assert!(write_private(&link, b"second").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
    }

    #[test]
    fn rejects_bad_argv() {
        for text in [
            "",
            "pending --json",
            "--help extra",
            "--db-env",
            "--db-env DB",
            "--db-env db pending --json",
            "--db-env postgres://someone:secret@localhost/db pending --json",
            "--db-env 1DB pending --json",
            "--db-env DB pending",
            "--db-env DB pending --json --json",
            "--db-env DB pending --other",
            "--db-env DB get IN-1",
            "--db-env DB get IN-01 --json",
            "--db-env DB get 1 --json",
            "--db-env DB get IN-1 --json extra",
            "--db-env DB export IN-1",
            "--db-env DB set-status IN-1 unknown",
            "--db-env DB set-status IN-1 held --ref",
            "--db-env DB set-status IN-1 held --ref x --ref y",
            "--db-env DB event IN-1 --kind comment",
            "--db-env DB event IN-1 --kind status --body-file f",
            "--db-env DB event IN-1 --kind comment --kind comment --body-file f",
            "--db-env DB event IN-1 --body-file f --body-file f --kind comment",
            "--db-env DB event IN-1 --kind comment --body-file f extra",
            "--db-env DB --db-env DB pending --json",
        ] {
            assert!(parse(argv(text)).is_err(), "{text}");
        }
        assert!(
            parse(vec![
                "--db-env".into(),
                "A".repeat(65),
                "pending".into(),
                "--json".into()
            ])
            .is_err()
        );
        assert!(
            parse(vec![
                "--db-env".into(),
                "DB".into(),
                "set-status".into(),
                "IN-1".into(),
                "held".into(),
                "--ref".into(),
                " bad ".into()
            ])
            .is_err()
        );
    }
}
