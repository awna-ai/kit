#![cfg(unix)]
use awna_keystore::{CredentialStore, Error, Item, MAX_SECRET, Secret, SecurityCli};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    dir: PathBuf,
    store: SecurityCli,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "awna-keystore-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let program = dir.join("stand-in");
        fs::write(
            &program,
            r#"#!/bin/sh
first=1
for arg in "$@"; do
    if [ "$first" = 0 ]; then printf '\t' >> "$HOME/argv.log"; fi
    printf '%s' "$arg" >> "$HOME/argv.log"
    first=0
done
printf '\n' >> "$HOME/argv.log"
printf 'call=%s\n' "$1" >> "$HOME/env.log"
/usr/bin/env >> "$HOME/env.log"
# Preserve the original descriptors before command substitution redirects stdout.
exec 3<&0 4>&1 5>&2
if [ -x /usr/bin/sw_vers ]; then
    null=$(/usr/bin/stat -L -f %d:%i:%r /dev/null)
    input=$(/usr/bin/stat -L -f %d:%i:%r /dev/fd/3)
    output=$(/usr/bin/stat -L -f %d:%i:%r /dev/fd/4)
    error=$(/usr/bin/stat -L -f %d:%i:%r /dev/fd/5)
else
    null=$(/usr/bin/stat -L -c %d:%i:%r /dev/null)
    input=$(/usr/bin/stat -L -c %d:%i:%r /dev/fd/3)
    output=$(/usr/bin/stat -L -c %d:%i:%r /dev/fd/4)
    error=$(/usr/bin/stat -L -c %d:%i:%r /dev/fd/5)
fi
in_kind=pipe; out_kind=pipe; err_kind=pipe
if [ "$input" = "$null" ]; then in_kind=null; fi
if [ "$output" = "$null" ]; then out_kind=null; fi
if [ "$error" = "$null" ]; then err_kind=null; fi
printf '%s in=%s out=%s err=%s\n' "$1" "$in_kind" "$out_kind" "$err_kind" >> "$HOME/stdio.log"
exec 3<&- 4>&- 5>&-
case "$1" in
    -i)
        /bin/cat > "$HOME/stdin.log"
        code=0
        if [ -f "$HOME/store.code" ]; then code=$(/bin/cat "$HOME/store.code"); fi
        exit "$code" ;;
    find-generic-password)
        if [ -f "$HOME/find.kill" ]; then /bin/kill -KILL $$; fi
        if [ -f "$HOME/find.out" ]; then /bin/cat "$HOME/find.out"; fi
        code=0
        if [ -f "$HOME/find.code" ]; then code=$(/bin/cat "$HOME/find.code"); fi
        exit "$code" ;;
    delete-generic-password)
        code=0
        if [ -f "$HOME/delete.code" ]; then code=$(/bin/cat "$HOME/delete.code"); fi
        exit "$code" ;;
esac
"#,
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        let store = SecurityCli::with_program(program, Some(dir.clone().into_os_string()));
        Self { dir, store }
    }
    fn set(&self, name: &str, value: impl AsRef<[u8]>) {
        fs::write(self.dir.join(name), value).unwrap();
    }
    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.dir.join(name)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).unwrap();
    }
}
fn item() -> Item {
    Item::new("awna-keystore.test", "invented", "Invented label (test)").unwrap()
}
fn value() -> Secret {
    Secret::new("invented-token".into())
}
fn failed(op: &'static str, why: &str) -> Error {
    Error::Failed {
        item: "awna-keystore.test/invented".into(),
        op,
        why: why.into(),
    }
}

#[test]
fn store_sends_the_value_on_stdin_only_and_reads_back() {
    let f = Fixture::new();
    f.set("find.out", "invented-token\n");
    assert_eq!(f.store.store(&item(), &value()), Ok(()));
    assert_eq!(
        f.read("argv.log"),
        "-i\nfind-generic-password\t-s\tawna-keystore.test\t-a\tinvented\t-w\n"
    );
    assert!(!f.read("argv.log").contains(value().expose()));
    assert_eq!(
        f.read("stdin.log"),
        "add-generic-password -U -s awna-keystore.test -a invented -l \"Invented label (test)\" -w \"invented-token\"\n"
    );
}

#[test]
fn only_home_is_inherited_and_none_omits_home() {
    let f = Fixture::new();
    assert_eq!(f.store.exists(&item()), Ok(true));
    let env = f.read("env.log");
    assert!(
        env.lines()
            .any(|line| line == format!("HOME={}", f.dir.display()))
    );
    assert!(!env.lines().any(|line| line.starts_with("PATH=")));
    let program = f.dir.join("no-home");
    // Embed a fixture path so this program can record its environment without HOME.
    fs::write(
        &program,
        format!(
            "#!/bin/sh\n/usr/bin/env > '{}'\n",
            f.dir.join("none.log").display()
        ),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    let store = SecurityCli::with_program(program, None);
    assert_eq!(store.exists(&item()), Ok(true));
    assert!(
        !f.read("none.log")
            .lines()
            .any(|line| line.starts_with("HOME="))
    );
    assert!(
        !f.read("none.log")
            .lines()
            .any(|line| line.starts_with("PATH="))
    );
}

#[test]
fn read_back_rejects_a_different_or_missing_value() {
    for missing in [false, true] {
        let f = Fixture::new();
        f.set("find.out", "other-token\n");
        if missing {
            f.set("find.code", "44");
        }
        assert_eq!(
            f.store.store(&item(), &value()),
            Err(Error::ReadBack {
                item: "awna-keystore.test/invented".into()
            })
        );
    }
}

#[test]
fn failed_store_stops_before_read_back() {
    let f = Fixture::new();
    f.set("store.code", "1");
    assert_eq!(
        f.store.store(&item(), &value()),
        Err(failed("store", "exit 1"))
    );
    assert_eq!(f.read("argv.log"), "-i\n");
}

#[test]
fn refused_values_run_nothing_and_errors_do_not_print_them() {
    let f = Fixture::new();
    let value = Secret::new("invented value".into());
    let error = f.store.store(&item(), &value).unwrap_err();
    assert_eq!(error, Error::NotStorable);
    assert!(!f.dir.join("argv.log").exists());
    assert!(!error.to_string().contains(value.expose()));
    assert!(!format!("{error:?}").contains(value.expose()));
}

#[test]
fn find_recognizes_missing_items_and_strips_exactly_one_newline() {
    let f = Fixture::new();
    f.set("find.code", "44");
    assert!(matches!(f.store.find(&item()), Ok(None)));
    f.set("find.code", "0");
    for (out, expected) in [("v\n", "v"), ("v\n\n", "v\n"), ("v", "v")] {
        f.set("find.out", out);
        assert_eq!(f.store.find(&item()).unwrap().unwrap().expose(), expected);
    }
}

#[test]
fn find_rejects_empty_and_invalid_utf8() {
    let f = Fixture::new();
    for out in [&b""[..], &b"\n"[..], &[0xff, 0xfe][..]] {
        f.set("find.out", out);
        assert!(
            matches!(f.store.find(&item()), Err(Error::Unreadable { item }) if item == "awna-keystore.test/invented")
        );
    }
}

#[test]
fn find_sanitizes_exit_and_signal_failures() {
    let f = Fixture::new();
    f.set("find.code", "3");
    assert!(matches!(f.store.find(&item()), Err(e) if e == failed("find", "exit 3")));
    f.set("find.kill", "");
    assert!(matches!(f.store.find(&item()), Err(e) if e == failed("find", "killed")));
}

#[test]
fn exists_omits_password_output_and_maps_statuses() {
    let f = Fixture::new();
    assert_eq!(f.store.exists(&item()), Ok(true));
    assert_eq!(
        f.read("argv.log"),
        "find-generic-password\t-s\tawna-keystore.test\t-a\tinvented\n"
    );
    f.set("find.code", "44");
    assert_eq!(f.store.exists(&item()), Ok(false));
    f.set("find.code", "2");
    assert_eq!(f.store.exists(&item()), Err(failed("exists", "exit 2")));
}

#[test]
fn delete_accepts_missing_and_reports_other_failures() {
    let f = Fixture::new();
    assert_eq!(f.store.delete(&item()), Ok(()));
    assert_eq!(
        f.read("argv.log"),
        "delete-generic-password\t-s\tawna-keystore.test\t-a\tinvented\n"
    );
    f.set("delete.code", "44");
    assert_eq!(f.store.delete(&item()), Ok(()));
    f.set("delete.code", "5");
    assert_eq!(f.store.delete(&item()), Err(failed("delete", "exit 5")));
}

#[test]
fn a_missing_program_is_a_sanitized_failure_for_every_operation() {
    let f = Fixture::new();
    let store = SecurityCli::with_program(f.dir.join("missing"), None);
    assert!(matches!(store.find(&item()), Err(e) if e == failed("find", "could not run")));
    assert_eq!(
        store.exists(&item()),
        Err(failed("exists", "could not run"))
    );
    assert_eq!(
        store.store(&item(), &value()),
        Err(failed("store", "could not run"))
    );
    assert_eq!(
        store.delete(&item()),
        Err(failed("delete", "could not run"))
    );
}

#[test]
fn maximum_value_and_fields_reach_stdin_whole() {
    let f = Fixture::new();
    let field = "a".repeat(64);
    let item = Item::new(&field, &field, &field).unwrap();
    let value = Secret::new("a".repeat(MAX_SECRET));
    f.set("find.out", value.expose());
    assert_eq!(f.store.store(&item, &value), Ok(()));
    assert_eq!(
        f.read("stdin.log"),
        format!(
            "add-generic-password -U -s {field} -a {field} -l \"{field}\" -w \"{}\"\n",
            value.expose()
        )
    );
    let too_long = Secret::new("a".repeat(MAX_SECRET + 1));
    fs::remove_file(f.dir.join("argv.log")).unwrap();
    assert_eq!(f.store.store(&item, &too_long), Err(Error::NotStorable));
    assert!(!f.dir.join("argv.log").exists());
}

#[test]
fn every_call_has_the_required_stdio_and_environment() {
    let f = Fixture::new();
    f.set("find.out", "invented-token\n");
    assert_eq!(f.store.store(&item(), &value()), Ok(()));
    assert_eq!(
        f.store.find(&item()).unwrap().unwrap().expose(),
        value().expose()
    );
    assert_eq!(f.store.exists(&item()), Ok(true));
    assert_eq!(f.store.delete(&item()), Ok(()));
    assert_eq!(
        f.read("stdio.log"),
        concat!(
            "-i in=pipe out=null err=null\n",
            "find-generic-password in=null out=pipe err=null\n",
            "find-generic-password in=null out=pipe err=null\n",
            "find-generic-password in=null out=null err=null\n",
            "delete-generic-password in=null out=null err=null\n",
        )
    );
    let env = f.read("env.log");
    let blocks: Vec<_> = env.split("call=").skip(1).collect();
    assert_eq!(blocks.len(), 5);
    for (block, call) in blocks.iter().zip([
        "-i",
        "find-generic-password",
        "find-generic-password",
        "find-generic-password",
        "delete-generic-password",
    ]) {
        let mut lines = block.lines();
        assert_eq!(lines.next(), Some(call));
        let lines: Vec<_> = lines.collect();
        assert!(
            lines
                .iter()
                .any(|line| *line == format!("HOME={}", f.dir.display()))
        );
        assert!(!lines.iter().any(|line| line.starts_with("PATH=")));
    }
}
