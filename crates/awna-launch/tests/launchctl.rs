use awna_launch::{Launchctl, Manager, ManagerError, ResolveError, State};
use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "awna-launch-command-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn manager(&self, script: &str) -> Launchctl {
        let program = self.0.join("control");
        fs::write(
            &program,
            format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"${{0%/*}}/calls\"\n{script}\n"),
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        Launchctl::new(program, 501)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const LABEL: &str = "com.example.alpha";
const PLIST: &str = "/var/tmp/example/LaunchAgents/com.example.alpha.plist";

#[test]
fn each_verb_passes_the_expected_arguments() {
    let fixture = Fixture::new();
    let manager = fixture.manager(
        r#"case "$1" in
        print) printf 'gui/501/com.example.alpha = {\n\tstate = running\n\tpid = 37027\n}\n' ;;
        print-disabled) printf '"com.example.alpha" => disabled\n' ;;
        esac"#,
    );
    assert_eq!(manager.domain(), "gui/501");
    assert_eq!(manager.target(LABEL), "gui/501/com.example.alpha");
    assert!(manager.session_ok().unwrap());
    assert_eq!(
        manager.state(LABEL).unwrap(),
        State::Running { pid: Some(37027) }
    );
    assert!(manager.disabled(LABEL).unwrap());
    assert!(manager.load(Path::new(PLIST)).unwrap());
    assert!(manager.unload(LABEL).unwrap());
    assert!(manager.restart(LABEL).unwrap());
    assert_eq!(
        fs::read_to_string(fixture.0.join("calls")).unwrap(),
        concat!(
            "print gui/501\n",
            "print gui/501/com.example.alpha\n",
            "print-disabled gui/501\n",
            "bootstrap gui/501 /var/tmp/example/LaunchAgents/com.example.alpha.plist\n",
            "bootout gui/501/com.example.alpha\n",
            "kickstart -k gui/501/com.example.alpha\n",
        )
    );
}

#[test]
fn invalid_arguments_never_run_the_program() {
    use std::os::unix::ffi::OsStrExt;

    let fixture = Fixture::new();
    let manager = fixture.manager("exit 0");
    assert_eq!(manager.state("../x"), Err(ManagerError::BadLabel));
    assert_eq!(manager.disabled("../x"), Err(ManagerError::BadLabel));
    assert_eq!(manager.unload("../x"), Err(ManagerError::BadLabel));
    assert_eq!(manager.restart("../x"), Err(ManagerError::BadLabel));
    assert_eq!(
        manager.load(Path::new("relative.plist")),
        Err(ManagerError::BadPath)
    );
    let malformed = Path::new(OsStr::from_bytes(b"/example/\xff.plist"));
    assert_eq!(manager.load(malformed), Err(ManagerError::BadPath));
    assert!(!fixture.0.join("calls").exists());
}

#[test]
fn unsuccessful_commands_use_verb_specific_results() {
    let fixture = Fixture::new();
    let manager = fixture.manager("printf '  first line  \\nsecond line\\n' >&2; exit 1");
    assert!(!manager.session_ok().unwrap());
    assert_eq!(manager.state(LABEL).unwrap(), State::NotLoaded);
    assert!(!manager.load(Path::new(PLIST)).unwrap());
    assert!(!manager.unload(LABEL).unwrap());
    assert!(!manager.restart(LABEL).unwrap());
    assert_eq!(
        manager.disabled(LABEL),
        Err(ManagerError::Failed {
            verb: "print-disabled",
            code: Some(1),
            stderr: "first line".into(),
        })
    );
}

#[test]
fn error_messages_keep_at_most_two_hundred_characters() {
    let fixture = Fixture::new();
    let manager = fixture.manager(&format!("printf '%s\\n' '{}' >&2; exit 1", "é".repeat(250)));
    assert_eq!(
        manager.disabled(LABEL),
        Err(ManagerError::Failed {
            verb: "print-disabled",
            code: Some(1),
            stderr: "é".repeat(200),
        })
    );
}

#[test]
fn both_pipes_are_drained_without_waiting_for_the_child_to_exit() {
    let fixture = Fixture::new();
    let manager = fixture.manager(
        r#"/bin/dd if=/dev/zero bs=1024 count=300 2>/dev/null
        /bin/dd if=/dev/zero bs=1024 count=1100 >&2 2>/dev/null"#,
    );
    assert!(manager.session_ok().unwrap());
}

#[test]
fn timeout_returns_without_waiting_for_a_descendant_to_close_the_pipes() {
    let fixture = Fixture::new();
    let manager = fixture
        .manager("/bin/sleep 5; exit 0")
        .with_timeouts(Duration::from_millis(200), Duration::from_secs(1));
    let start = Instant::now();
    assert_eq!(
        manager.session_ok(),
        Err(ManagerError::TimedOut { verb: "print" })
    );
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn a_successful_exit_can_return_while_a_descendant_holds_the_pipes() {
    let fixture = Fixture::new();
    let manager = fixture
        .manager("/bin/sleep 5 & exit 0")
        .with_timeouts(Duration::from_millis(200), Duration::from_secs(1));
    let start = Instant::now();
    assert!(manager.session_ok().unwrap());
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn bootout_has_its_own_timeout() {
    let fixture = Fixture::new();
    let manager = fixture
        .manager("/bin/sleep 0.3; exit 0")
        .with_timeouts(Duration::from_millis(100), Duration::from_secs(2));
    assert_eq!(
        manager.restart(LABEL),
        Err(ManagerError::TimedOut { verb: "kickstart" })
    );
    assert!(manager.unload(LABEL).unwrap());

    let manager = manager.with_timeouts(Duration::from_secs(2), Duration::from_millis(100));
    assert_eq!(
        manager.unload(LABEL),
        Err(ManagerError::TimedOut { verb: "bootout" })
    );
    assert!(manager.restart(LABEL).unwrap());
}

#[test]
fn a_missing_program_returns_its_io_error_kind() {
    let fixture = Fixture::new();
    let manager = Launchctl::new(fixture.0.join("absent"), 501);
    assert_eq!(
        manager.session_ok(),
        Err(ManagerError::Spawn(std::io::ErrorKind::NotFound))
    );
}

#[test]
fn system_resolution_requires_the_accounts_home() {
    let home = Path::new("/var/tmp/example/home");
    let own_home = Path::new("/var/tmp/example/account");
    let program = OsStr::new("/opt/example/bin/alpha");
    assert_eq!(
        Launchctl::resolve(Some(program), home, own_home, 501)
            .unwrap()
            .program(),
        Path::new(program)
    );
    assert_eq!(
        Launchctl::resolve(Some(OsStr::new("")), home, own_home, 501),
        Err(ResolveError::ForeignHome)
    );
    assert_eq!(
        Launchctl::resolve(None, home, own_home, 501),
        Err(ResolveError::ForeignHome)
    );
    assert_eq!(
        Launchctl::resolve(None, own_home, own_home, 501)
            .unwrap()
            .program(),
        Path::new("/bin/launchctl")
    );
    assert_eq!(
        Launchctl::resolve(Some(OsStr::new("")), own_home, own_home, 501)
            .unwrap()
            .program(),
        Path::new("/bin/launchctl")
    );
}
