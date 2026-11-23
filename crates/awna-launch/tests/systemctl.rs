use awna_launch::{Manager, ManagerError, Platform, ResolveError, State, Systemctl};
use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "awna-launch-systemctl-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn manager(&self, script: &str) -> Systemctl {
        let program = self.0.join("control");
        fs::write(
            &program,
            format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"${{0%/*}}/calls\"\n{script}\n"),
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        Systemctl::new(program, 501)
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.0.join("calls")).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const LABEL: &str = "com.example.alpha";
const UNIT: &str = "/var/tmp/example/units/com.example.alpha.service";

#[test]
fn each_verb_runs_the_expected_arguments() {
    let fixture = Fixture::new();
    let manager = fixture.manager(
        r#"case "$2" in
        show) printf 'LoadState=loaded\nActiveState=active\nMainPID=4242\nUnitFileState=enabled\n' ;;
        esac"#,
    );
    assert_eq!(manager.uid(), 501);
    assert_eq!(manager.platform(), Platform::Linux);
    assert_eq!(Systemctl::unit_name(LABEL), "com.example.alpha.service");
    assert!(manager.session_ok().unwrap());
    assert_eq!(manager.state(LABEL), Ok(State::Running { pid: Some(4242) }));
    assert_eq!(manager.disabled(LABEL), Ok(false));
    assert_eq!(manager.load(Path::new(UNIT)), Ok(true));
    assert_eq!(manager.unload(LABEL), Ok(true));
    assert_eq!(manager.restart(LABEL), Ok(true));
    assert_eq!(
        fixture.calls(),
        concat!(
            "--user show-environment\n",
            "--user show com.example.alpha.service --property=LoadState,ActiveState,MainPID,UnitFileState\n",
            "--user show com.example.alpha.service --property=LoadState,ActiveState,MainPID,UnitFileState\n",
            "--user daemon-reload\n",
            "--user enable --now com.example.alpha.service\n",
            "--user disable --now com.example.alpha.service\n",
            "--user restart com.example.alpha.service\n",
        )
    );
}

#[test]
fn a_failed_reload_skips_enable() {
    for (verb, calls) in [
        ("daemon-reload", "--user daemon-reload\n"),
        (
            "enable",
            "--user daemon-reload\n--user enable --now com.example.alpha.service\n",
        ),
    ] {
        let fixture = Fixture::new();
        let manager = fixture.manager(&format!(r#"case "$2" in {verb}) exit 1 ;; esac"#));
        assert_eq!(manager.load(Path::new(UNIT)), Ok(false));
        assert_eq!(fixture.calls(), calls);
    }
}

#[test]
fn invalid_arguments_run_nothing() {
    let fixture = Fixture::new();
    let manager = fixture.manager("exit 0");
    let label = "../x";
    assert_eq!(manager.state(label), Err(ManagerError::BadLabel));
    assert_eq!(manager.disabled(label), Err(ManagerError::BadLabel));
    assert_eq!(manager.unload(label), Err(ManagerError::BadLabel));
    assert_eq!(manager.restart(label), Err(ManagerError::BadLabel));
    for path in [
        "com.example.alpha.service",
        "/var/tmp/example/com.example.alpha.plist",
        "/var/tmp/example/.service",
        "/var/tmp/example/a..b.service",
    ] {
        assert_eq!(manager.load(Path::new(path)), Err(ManagerError::BadPath));
    }
    assert!(!fixture.0.join("calls").exists());
}

#[test]
fn a_failed_show_is_an_error_not_a_state() {
    for (script, error) in [
        (
            r#"printf '  Failed to connect to bus  \nMore details\n' >&2; exit 1"#,
            ManagerError::Failed {
                verb: "show",
                code: Some(1),
                stderr: "Failed to connect to bus".into(),
            },
        ),
        ("exit 0", ManagerError::Unparsed { verb: "show" }),
    ] {
        let fixture = Fixture::new();
        let manager = fixture.manager(script);
        assert_eq!(manager.state(LABEL), Err(error.clone()));
        assert_eq!(manager.disabled(LABEL), Err(error));
    }
}

#[test]
fn disable_and_restart_use_the_unload_timeout() {
    let fixture = Fixture::new();
    let manager = fixture
        .manager(r#"case "$2" in disable|restart|show) sleep 1 ;; esac"#)
        .with_timeouts(Duration::from_millis(200), Duration::from_secs(5));
    assert_eq!(manager.unload(LABEL), Ok(true));
    assert_eq!(manager.restart(LABEL), Ok(true));
    assert_eq!(
        manager.state(LABEL),
        Err(ManagerError::TimedOut { verb: "show" })
    );
}

#[test]
fn resolve_follows_the_same_rules_as_launchctl() {
    let fixture = Fixture::new();
    let manager = fixture.manager("exit 0");
    let home = Path::new("/home/example");
    let foreign = Path::new("/var/tmp/example/home");
    let resolve =
        |stand_in, home| Systemctl::resolve(stand_in, home, Path::new("/home/example"), 501);
    assert_eq!(
        resolve(Some(OsStr::new("control")), foreign),
        Err(ResolveError::StandInNotAbsolute)
    );
    assert_eq!(
        resolve(Some(OsStr::new("/usr/bin/systemctl")), foreign),
        Err(ResolveError::StandInIsSystem)
    );
    assert_eq!(
        resolve(Some(manager.program().as_os_str()), foreign),
        Ok(manager.clone())
    );
    assert_eq!(resolve(None, foreign), Err(ResolveError::ForeignHome));
    assert_eq!(resolve(None, home), Ok(Systemctl::system(501)));
    assert_eq!(
        resolve(Some(OsStr::new("")), foreign),
        Err(ResolveError::ForeignHome)
    );
    assert_eq!(
        resolve(Some(OsStr::new("")), home),
        Ok(Systemctl::system(501))
    );
}
