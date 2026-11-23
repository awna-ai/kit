use awna_launch::{
    Applied, ApplyError, Call, Logs, ManagerError, Platform, Policy, Removed, Rollback, Run,
    StandIn, State, Tries, Unit, UnitError, Wait, apply, plist_path, remove, render, retire,
};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

const LABEL: &str = "com.example.alpha";
const OLD: &str = "com.example.old";
const BETA: &str = "com.example.beta";
const RUNNING: State = State::Running { pid: Some(42) };

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "example-apply-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn uid(&self) -> u32 {
        static UID: OnceLock<u32> = OnceLock::new();
        *UID.get_or_init(|| {
            let probe = self.0.join("owner");
            fs::File::create(probe).unwrap().metadata().unwrap().uid()
        })
    }

    fn manager(&self) -> StandIn {
        StandIn::new().with_uid(self.uid())
    }

    fn agents(&self) -> PathBuf {
        self.0.join("agents")
    }

    fn plist(&self) -> PathBuf {
        plist_path(&self.agents(), LABEL)
    }

    fn seed(&self, bytes: &[u8]) {
        fs::create_dir(self.agents()).unwrap();
        fs::write(self.plist(), bytes).unwrap();
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn unit() -> Unit {
    Unit::new(LABEL, vec!["/opt/example/bin/alpha".into()], Run::Once)
}

fn policy() -> Policy {
    Policy {
        bootout_wait: Wait {
            checks: 2,
            pause: Duration::ZERO,
        },
        bootstrap_tries: Tries {
            tries: 3,
            pause: Duration::ZERO,
        },
        ..Policy::default()
    }
}

fn install(manager: &StandIn, scratch: &Scratch) -> Result<Applied, ApplyError> {
    apply(manager, &scratch.agents(), &unit(), &policy(), &mut || true)
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn defaults_protect_running_and_disabled_services() {
    assert_eq!(
        Policy::default(),
        Policy {
            skip_unchanged: true,
            keep_disabled: true,
            never_stop_running: true,
            bootout_wait: Wait {
                checks: 10,
                pause: Duration::from_millis(500)
            },
            bootstrap_tries: Tries {
                tries: 3,
                pause: Duration::from_secs(1)
            },
        }
    );
}

#[test]
fn fresh_install_creates_plist_and_logs() {
    let scratch = Scratch::new();
    let manager = scratch.manager();
    let mut unit = unit();
    let log_dir = scratch.0.join("logs");
    let log = log_dir.join("alpha.log");
    unit.logs = Logs::both(&log);
    assert_eq!(
        apply(&manager, &scratch.agents(), &unit, &policy(), &mut || true),
        Ok(Applied::Loaded)
    );
    assert_eq!(
        manager.calls(),
        [
            Call::SessionOk,
            Call::State(LABEL.into()),
            Call::Disabled(LABEL.into()),
            Call::Load(scratch.plist())
        ]
    );
    assert_eq!(
        fs::read(scratch.plist()).unwrap(),
        render(&unit, Platform::MacOs).unwrap().as_bytes()
    );
    assert_eq!(mode(&scratch.plist()), 0o600);
    assert_eq!(mode(&scratch.agents()), 0o700);
    assert_eq!(mode(&log_dir), 0o700);
    assert_eq!(mode(&log), 0o600);
}

#[test]
fn missing_session_and_invalid_unit_write_nothing() {
    let scratch = Scratch::new();
    let manager = scratch.manager().without_session();
    assert_eq!(install(&manager, &scratch), Err(ApplyError::NoSession));
    assert_eq!(manager.calls(), [Call::SessionOk]);
    let manager = scratch.manager();
    let mut unit = unit();
    unit.label.clear();
    assert_eq!(
        apply(&manager, &scratch.agents(), &unit, &policy(), &mut || true),
        Err(ApplyError::Unit(UnitError::Label))
    );
    assert!(manager.calls().is_empty());
    assert!(!scratch.agents().exists());
}

#[test]
fn unchanged_services_are_skipped_only_when_loaded() {
    let scratch = Scratch::new();
    let unit = unit();
    scratch.seed(render(&unit, Platform::MacOs).unwrap().as_bytes());
    for state in [State::Idle, RUNNING] {
        let manager = scratch.manager().with_loaded(LABEL, state);
        assert_eq!(
            apply(
                &manager,
                &scratch.agents(),
                &unit,
                &policy(),
                &mut || panic!("health called")
            ),
            Ok(Applied::Unchanged)
        );
        assert_eq!(
            manager.calls(),
            [Call::SessionOk, Call::State(LABEL.into())]
        );
    }
    assert_eq!(install(&scratch.manager(), &scratch), Ok(Applied::Loaded));
    let manager = scratch.manager().with_loaded(LABEL, State::Idle);
    let policy = Policy {
        skip_unchanged: false,
        ..policy()
    };
    assert_eq!(
        apply(&manager, &scratch.agents(), &unit, &policy, &mut || true),
        Ok(Applied::Reloaded)
    );
    assert_eq!(
        manager.calls(),
        [
            Call::SessionOk,
            Call::State(LABEL.into()),
            Call::Disabled(LABEL.into()),
            Call::Unload(LABEL.into()),
            Call::State(LABEL.into()),
            Call::Load(scratch.plist()),
        ]
    );
}

#[test]
fn a_running_service_is_never_stopped_by_default() {
    let scratch = Scratch::new();
    scratch.seed(b"old");
    let manager = scratch.manager().with_loaded(LABEL, RUNNING);
    assert_eq!(
        install(&manager, &scratch),
        Err(ApplyError::Running { pid: Some(42) })
    );
    assert_eq!(fs::read(scratch.plist()).unwrap(), b"old");
    assert_eq!(
        manager.calls(),
        [
            Call::SessionOk,
            Call::State(LABEL.into()),
            Call::Disabled(LABEL.into())
        ]
    );
    let policy = Policy {
        never_stop_running: false,
        ..policy()
    };
    assert_eq!(
        apply(&manager, &scratch.agents(), &unit(), &policy, &mut || true),
        Ok(Applied::Reloaded)
    );
}

#[test]
fn disabled_service_gets_a_plist_without_service_calls() {
    let scratch = Scratch::new();
    let manager = scratch.manager().with_disabled(LABEL);
    assert_eq!(install(&manager, &scratch), Ok(Applied::LeftDisabled));
    assert_eq!(
        fs::read(scratch.plist()).unwrap(),
        render(&unit(), Platform::MacOs).unwrap().as_bytes()
    );
    assert_eq!(
        manager.calls(),
        [
            Call::SessionOk,
            Call::State(LABEL.into()),
            Call::Disabled(LABEL.into())
        ]
    );
}

#[test]
fn unreadable_disabled_state_prevents_writes() {
    let scratch = Scratch::new();
    let manager = scratch.manager().with_disabled_unreadable();
    assert!(matches!(
        install(&manager, &scratch),
        Err(ApplyError::Launchctl(_))
    ));
    assert!(!scratch.agents().exists());
}

#[test]
fn stuck_unload_keeps_the_old_plist_after_bounded_polls() {
    let scratch = Scratch::new();
    scratch.seed(b"old");
    let manager = scratch
        .manager()
        .with_loaded(LABEL, State::Idle)
        .stuck_unloads();
    assert_eq!(install(&manager, &scratch), Err(ApplyError::StillLoaded));
    assert_eq!(fs::read(scratch.plist()).unwrap(), b"old");
    assert_eq!(
        manager.calls(),
        [
            Call::SessionOk,
            Call::State(LABEL.into()),
            Call::Disabled(LABEL.into()),
            Call::Unload(LABEL.into()),
            Call::State(LABEL.into()),
            Call::State(LABEL.into()),
        ]
    );
}

#[test]
fn loads_retry_and_roll_back_when_exhausted() {
    let scratch = Scratch::new();
    let manager = scratch.manager().failing_loads(2);
    assert_eq!(install(&manager, &scratch), Ok(Applied::Loaded));
    assert_eq!(
        manager
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::Load(_)))
            .count(),
        3
    );

    let scratch = Scratch::new();
    let manager = scratch.manager().failing_loads(3);
    assert_eq!(
        install(&manager, &scratch),
        Err(ApplyError::LoadFailed {
            tries: 3,
            rollback: Rollback::Removed
        })
    );
    assert!(!scratch.plist().exists());

    let scratch = Scratch::new();
    scratch.seed(b"old");
    let manager = scratch
        .manager()
        .with_loaded(LABEL, State::Idle)
        .failing_loads(3);
    assert_eq!(
        install(&manager, &scratch),
        Err(ApplyError::LoadFailed {
            tries: 3,
            rollback: Rollback::Restored
        })
    );
    assert_eq!(fs::read(scratch.plist()).unwrap(), b"old");
    assert_eq!(manager.state_of(LABEL), State::Idle);
}

#[test]
fn failed_load_restores_an_unloaded_services_plist() {
    let scratch = Scratch::new();
    scratch.seed(b"old");
    let manager = scratch.manager().failing_loads(3);

    assert_eq!(
        install(&manager, &scratch),
        Err(ApplyError::LoadFailed {
            tries: 3,
            rollback: Rollback::Restored,
        })
    );
    assert_eq!(fs::read(scratch.plist()).unwrap(), b"old");
    assert_eq!(
        manager
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::Load(_)))
            .count(),
        3
    );
    assert_eq!(manager.state_of(LABEL), State::NotLoaded);
}

#[test]
fn failed_health_check_restores_an_unloaded_services_plist() {
    let scratch = Scratch::new();
    scratch.seed(b"old");
    let manager = scratch.manager();

    assert_eq!(
        apply(&manager, &scratch.agents(), &unit(), &policy(), &mut || {
            false
        }),
        Err(ApplyError::Unhealthy {
            rollback: Rollback::Restored,
        })
    );
    assert_eq!(fs::read(scratch.plist()).unwrap(), b"old");
    assert_eq!(
        manager
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::Load(_)))
            .count(),
        1
    );
    assert_eq!(manager.state_of(LABEL), State::NotLoaded);
}

#[test]
fn unhealthy_service_is_unloaded_before_rollback() {
    let scratch = Scratch::new();
    let manager = scratch.manager();
    let mut checks = 0;
    let result = apply(&manager, &scratch.agents(), &unit(), &policy(), &mut || {
        checks += 1;
        false
    });
    assert_eq!(
        result,
        Err(ApplyError::Unhealthy {
            rollback: Rollback::Removed
        })
    );
    assert_eq!(checks, 1);
    assert!(!scratch.plist().exists());
    assert!(
        manager
            .calls()
            .ends_with(&[Call::Unload(LABEL.into()), Call::State(LABEL.into())])
    );
}

#[test]
fn unsafe_paths_are_rejected_before_writes() {
    let scratch = Scratch::new();
    let manager = scratch.manager();
    let target = scratch.0.join("target");
    fs::create_dir(&target).unwrap();
    symlink(&target, scratch.agents()).unwrap();
    assert_eq!(
        install(&manager, &scratch),
        Err(ApplyError::Unsafe(scratch.agents()))
    );
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);

    fs::remove_file(scratch.agents()).unwrap();
    fs::create_dir(scratch.agents()).unwrap();
    fs::set_permissions(scratch.agents(), fs::Permissions::from_mode(0o775)).unwrap();
    assert_eq!(
        install(&manager, &scratch),
        Err(ApplyError::Unsafe(scratch.agents()))
    );
    assert!(!scratch.plist().exists());

    fs::set_permissions(scratch.agents(), fs::Permissions::from_mode(0o755)).unwrap();
    let target = target.join("file");
    fs::write(&target, b"old").unwrap();
    symlink(&target, scratch.plist()).unwrap();
    assert_eq!(
        install(&manager, &scratch),
        Err(ApplyError::Unsafe(scratch.plist()))
    );
    fs::remove_file(scratch.plist()).unwrap();
    let log = scratch.0.join("report");
    symlink(&target, &log).unwrap();
    let mut unit = unit();
    unit.logs = Logs::both(&log);
    assert_eq!(
        apply(&manager, &scratch.agents(), &unit, &policy(), &mut || true),
        Err(ApplyError::Unsafe(log))
    );
    assert!(!scratch.plist().exists());
    assert_eq!(fs::read(target).unwrap(), b"old");
    assert_eq!(manager.calls(), vec![Call::SessionOk; 4]);
}

#[test]
fn remove_respects_running_and_stuck_services() {
    let scratch = Scratch::new();
    scratch.seed(b"old");
    let manager = scratch.manager().with_loaded(LABEL, RUNNING);
    assert_eq!(
        remove(&manager, &scratch.agents(), LABEL, &policy()),
        Err(ApplyError::Running { pid: Some(42) })
    );
    assert_eq!(fs::read(scratch.plist()).unwrap(), b"old");

    let manager = scratch
        .manager()
        .with_loaded(LABEL, State::Idle)
        .stuck_unloads();
    assert_eq!(
        remove(&manager, &scratch.agents(), LABEL, &policy()),
        Err(ApplyError::StillLoaded)
    );
    assert_eq!(fs::read(scratch.plist()).unwrap(), b"old");

    let manager = scratch.manager().with_loaded(LABEL, State::Idle);
    assert_eq!(
        remove(&manager, &scratch.agents(), LABEL, &policy()),
        Ok(Removed::Removed)
    );
    assert_eq!(
        remove(&manager, &scratch.agents(), LABEL, &policy()),
        Ok(Removed::NotInstalled)
    );
    assert_eq!(manager.state_of(LABEL), State::NotLoaded);
    assert!(!scratch.plist().exists());
}

#[test]
fn retire_works_in_order_and_stops_at_first_error() {
    let scratch = Scratch::new();
    let manager = scratch
        .manager()
        .with_loaded(OLD, State::Idle)
        .with_loaded(BETA, State::Idle);
    assert_eq!(
        retire(&manager, &scratch.agents(), &[OLD, BETA], &policy()),
        Ok(vec![Removed::Removed; 2])
    );
    assert_eq!(
        manager.calls(),
        [
            Call::SessionOk,
            Call::State(OLD.into()),
            Call::Unload(OLD.into()),
            Call::State(OLD.into()),
            Call::SessionOk,
            Call::State(BETA.into()),
            Call::Unload(BETA.into()),
            Call::State(BETA.into()),
        ]
    );
    let manager = scratch.manager().with_loaded(OLD, RUNNING);
    assert_eq!(
        retire(&manager, &scratch.agents(), &[OLD, BETA], &policy()),
        Err(ApplyError::Running { pid: Some(42) })
    );
    assert_eq!(manager.calls(), [Call::SessionOk, Call::State(OLD.into())]);
}

#[test]
fn a_failed_plist_write_reloads_the_previous_file() {
    let scratch = Scratch::new();
    scratch.seed(b"old");
    let temporary = scratch
        .agents()
        .join(format!(".{LABEL}.plist.{}.tmp", std::process::id()));
    fs::write(&temporary, b"occupied").unwrap();
    let manager = scratch.manager().with_loaded(LABEL, State::Idle);

    assert!(matches!(
        install(&manager, &scratch),
        Err(ApplyError::Io {
            kind: std::io::ErrorKind::AlreadyExists,
            ..
        })
    ));
    assert_eq!(fs::read(scratch.plist()).unwrap(), b"old");
    assert_eq!(fs::read(temporary).unwrap(), b"occupied");
    assert_eq!(manager.state_of(LABEL), State::Idle);
    assert!(manager.calls().ends_with(&[Call::Load(scratch.plist())]));
}

#[test]
fn a_stuck_unhealthy_service_keeps_its_plist() {
    let scratch = Scratch::new();
    let manager = scratch.manager().stuck_unloads();
    assert_eq!(
        apply(&manager, &scratch.agents(), &unit(), &policy(), &mut || {
            false
        },),
        Err(ApplyError::Unhealthy {
            rollback: Rollback::Failed,
        })
    );
    assert!(scratch.plist().is_file());
    assert_eq!(manager.state_of(LABEL), State::Idle);
}

#[test]
fn remove_rejects_a_directory_and_unlinks_a_symlink() {
    let scratch = Scratch::new();
    fs::create_dir_all(scratch.plist()).unwrap();
    let manager = scratch.manager();
    assert_eq!(
        remove(&manager, &scratch.agents(), LABEL, &policy()),
        Err(ApplyError::Unsafe(scratch.plist()))
    );
    assert_eq!(manager.calls(), [Call::SessionOk]);
    fs::remove_dir(scratch.plist()).unwrap();
    let target = scratch.0.join("target");
    fs::create_dir(&target).unwrap();
    symlink(&target, scratch.plist()).unwrap();

    assert_eq!(
        remove(&manager, &scratch.agents(), LABEL, &policy()),
        Ok(Removed::Removed)
    );
    assert!(target.is_dir());
    assert!(fs::symlink_metadata(scratch.plist()).is_err());
}

#[test]
fn a_disabled_service_with_matching_bytes_keeps_its_plist() {
    let scratch = Scratch::new();
    scratch.seed(render(&unit(), Platform::MacOs).unwrap().as_bytes());
    let inode = fs::metadata(scratch.plist()).unwrap().ino();
    let manager = scratch.manager().with_disabled(LABEL);
    assert_eq!(install(&manager, &scratch), Ok(Applied::LeftDisabled));
    assert_eq!(fs::metadata(scratch.plist()).unwrap().ino(), inode);
}

#[test]
fn zero_attempts_roll_back_without_calling_load() {
    let scratch = Scratch::new();
    let manager = scratch.manager();
    let mut policy = policy();
    policy.bootstrap_tries.tries = 0;
    assert_eq!(
        apply(&manager, &scratch.agents(), &unit(), &policy, &mut || true),
        Err(ApplyError::LoadFailed {
            tries: 0,
            rollback: Rollback::Removed,
        })
    );
    assert_eq!(
        manager.calls(),
        [
            Call::SessionOk,
            Call::State(LABEL.into()),
            Call::Disabled(LABEL.into()),
            Call::State(LABEL.into()),
        ]
    );
    assert!(!scratch.plist().exists());
}

#[test]
fn a_directory_symlink_stays_unsafe_with_a_trailing_separator() {
    let scratch = Scratch::new();
    let target = scratch.0.join("target");
    fs::create_dir(&target).unwrap();
    symlink(&target, scratch.agents()).unwrap();
    let agents = scratch.agents().join(".");
    let manager = scratch.manager();

    assert_eq!(
        apply(&manager, &agents, &unit(), &policy(), &mut || true),
        Err(ApplyError::Unsafe(agents))
    );
    assert_eq!(manager.calls(), [Call::SessionOk]);
    assert_eq!(fs::read_dir(target).unwrap().count(), 0);
}

#[test]
fn unsafe_agents_directory_spellings_are_rejected_before_state() {
    let scratch = Scratch::new();
    for agents in [
        PathBuf::from("relative-agents"),
        scratch.0.join("../agents"),
    ] {
        let manager = scratch.manager();
        assert_eq!(
            apply(&manager, &agents, &unit(), &policy(), &mut || true),
            Err(ApplyError::Unsafe(agents))
        );
        assert_eq!(manager.calls(), [Call::SessionOk]);
    }
}

#[test]
fn unsafe_immediate_log_parents_are_rejected_before_writes() {
    let scratch = Scratch::new();
    let target = scratch.0.join("target");
    fs::create_dir(&target).unwrap();
    let link = scratch.0.join("link");
    symlink(&target, &link).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o770)).unwrap();
    let file = scratch.0.join("file");
    fs::write(&file, b"").unwrap();

    for parent in [link, target, file] {
        let manager = scratch.manager();
        let mut unit = unit();
        unit.logs = Logs::both(parent.join("alpha.log"));
        assert_eq!(
            apply(&manager, &scratch.agents(), &unit, &policy(), &mut || true),
            Err(ApplyError::Unsafe(parent))
        );
        assert_eq!(manager.calls(), [Call::SessionOk]);
        assert!(!scratch.agents().exists());
    }
}

#[test]
fn disabled_install_prepares_logs() {
    let scratch = Scratch::new();
    let manager = scratch.manager().with_disabled(LABEL);
    let mut unit = unit();
    let parent = scratch.0.join("logs");
    let log = parent.join("alpha.log");
    unit.logs = Logs::both(&log);

    assert_eq!(
        apply(&manager, &scratch.agents(), &unit, &policy(), &mut || true),
        Ok(Applied::LeftDisabled)
    );
    assert_eq!(mode(&scratch.agents()), 0o700);
    assert_eq!(mode(&parent), 0o700);
    assert_eq!(mode(&log), 0o600);
}

#[test]
fn zero_unload_checks_still_observes_the_service_state() {
    let scratch = Scratch::new();
    scratch.seed(b"old");
    let manager = scratch.manager().with_loaded(LABEL, State::Idle);
    let mut policy = policy();
    policy.bootout_wait.checks = 0;
    policy.skip_unchanged = false;
    assert_eq!(
        apply(&manager, &scratch.agents(), &unit(), &policy, &mut || true),
        Ok(Applied::Reloaded)
    );
}

#[test]
fn failed_health_check_reloads_the_previous_service() {
    let scratch = Scratch::new();
    scratch.seed(b"old");
    let manager = scratch.manager().with_loaded(LABEL, State::Idle);

    assert_eq!(
        apply(&manager, &scratch.agents(), &unit(), &policy(), &mut || {
            false
        }),
        Err(ApplyError::Unhealthy {
            rollback: Rollback::Restored
        })
    );
    assert_eq!(fs::read(scratch.plist()).unwrap(), b"old");
    assert_eq!(manager.state_of(LABEL), State::Idle);
}

#[test]
fn apply_rejects_an_agents_dir_owned_by_another_user() {
    let scratch = Scratch::new();
    scratch.seed(b"old");
    let manager = scratch.manager().with_uid(scratch.uid() ^ 1);
    assert_eq!(
        install(&manager, &scratch),
        Err(ApplyError::Unsafe(scratch.agents()))
    );
    assert_eq!(fs::read(scratch.plist()).unwrap(), b"old");
    assert_eq!(manager.calls(), [Call::SessionOk]);
}

#[test]
fn failed_load_unloads_a_partially_loaded_service() {
    let scratch = Scratch::new();
    let manager = scratch.manager().failing_loads(3).loads_despite_failure();
    assert_eq!(
        install(&manager, &scratch),
        Err(ApplyError::LoadFailed {
            tries: 3,
            rollback: Rollback::Removed
        })
    );
    assert!(manager.calls().contains(&Call::Unload(LABEL.into())));
    assert_eq!(manager.state_of(LABEL), State::NotLoaded);
    assert!(!scratch.plist().exists());
}

#[test]
fn failed_load_preserves_the_plist_if_the_partial_load_cannot_be_unloaded() {
    let scratch = Scratch::new();
    let manager = scratch
        .manager()
        .failing_loads(3)
        .loads_despite_failure()
        .stuck_unloads();
    assert_eq!(
        install(&manager, &scratch),
        Err(ApplyError::LoadFailed {
            tries: 3,
            rollback: Rollback::Failed
        })
    );
    assert!(manager.calls().contains(&Call::Unload(LABEL.into())));
    assert_eq!(manager.state_of(LABEL), State::Idle);
    assert_eq!(
        fs::read(scratch.plist()).unwrap(),
        render(&unit(), Platform::MacOs).unwrap().as_bytes()
    );
}

#[test]
fn load_error_unloads_a_partially_loaded_service() {
    let scratch = Scratch::new();
    let manager = scratch.manager().erroring_loads(1).loads_despite_failure();
    let mut policy = policy();
    policy.bootstrap_tries.tries = 1;

    assert_eq!(
        apply(&manager, &scratch.agents(), &unit(), &policy, &mut || true),
        Err(ApplyError::LoadErrored {
            error: ManagerError::Failed {
                verb: "bootstrap",
                code: Some(5),
                stderr: String::new(),
            },
            rollback: Rollback::Removed,
        })
    );
    assert!(manager.calls().contains(&Call::Unload(LABEL.into())));
    assert_eq!(manager.state_of(LABEL), State::NotLoaded);
    assert!(!scratch.plist().exists());
}

#[test]
fn load_error_keeps_the_unit_if_the_partial_load_cannot_be_unloaded() {
    let scratch = Scratch::new();
    let manager = scratch
        .manager()
        .erroring_loads(1)
        .loads_despite_failure()
        .stuck_unloads();
    let mut policy = policy();
    policy.bootstrap_tries.tries = 1;

    assert_eq!(
        apply(&manager, &scratch.agents(), &unit(), &policy, &mut || true),
        Err(ApplyError::LoadErrored {
            error: ManagerError::Failed {
                verb: "bootstrap",
                code: Some(5),
                stderr: String::new(),
            },
            rollback: Rollback::Failed,
        })
    );
    assert!(manager.calls().contains(&Call::Unload(LABEL.into())));
    assert_eq!(manager.state_of(LABEL), State::Idle);
    assert_eq!(
        fs::read(scratch.plist()).unwrap(),
        render(&unit(), Platform::MacOs).unwrap().as_bytes()
    );
}

#[test]
fn load_error_reports_rollback_and_reloads_the_previous_service() {
    let scratch = Scratch::new();
    scratch.seed(b"old");
    let manager = scratch
        .manager()
        .with_loaded(LABEL, State::Idle)
        .erroring_loads(1);
    let error = install(&manager, &scratch).unwrap_err();
    let bootstrap = ManagerError::Failed {
        verb: "bootstrap",
        code: Some(5),
        stderr: String::new(),
    };
    assert_eq!(
        error,
        ApplyError::LoadErrored {
            error: bootstrap.clone(),
            rollback: Rollback::Restored,
        }
    );
    assert_eq!(
        std::error::Error::source(&error).unwrap().downcast_ref(),
        Some(&bootstrap)
    );
    assert!(error.to_string().contains("rollback: Restored"));
    assert_eq!(fs::read(scratch.plist()).unwrap(), b"old");
    assert_eq!(manager.state_of(LABEL), State::Idle);
}

#[test]
fn linux_install_writes_a_service_unit() {
    let scratch = Scratch::new();
    let manager = scratch.manager().with_platform(Platform::Linux);
    let units = scratch.0.join("units");
    let service = units.join("com.example.alpha.service");

    assert_eq!(
        apply(&manager, &units, &unit(), &policy(), &mut || true),
        Ok(Applied::Loaded)
    );
    assert_eq!(
        fs::read_to_string(&service).expect("service unit was written"),
        render(&unit(), Platform::Linux).unwrap()
    );
    assert_eq!(mode(&service), 0o600);
    assert!(!units.join("com.example.alpha.plist").exists());
    assert!(manager.calls().contains(&Call::Load(service)));
}

#[test]
fn linux_unchanged_unit_is_skipped() {
    let scratch = Scratch::new();
    let manager = scratch
        .manager()
        .with_platform(Platform::Linux)
        .with_loaded(LABEL, State::Idle);
    let units = scratch.0.join("units");
    fs::create_dir(&units).unwrap();
    fs::write(
        units.join("com.example.alpha.service"),
        render(&unit(), Platform::Linux).unwrap(),
    )
    .unwrap();

    assert_eq!(
        apply(&manager, &units, &unit(), &policy(), &mut || true),
        Ok(Applied::Unchanged)
    );
}

#[test]
fn linux_remove_deletes_the_service_unit() {
    let scratch = Scratch::new();
    let manager = scratch.manager().with_platform(Platform::Linux);
    let units = scratch.0.join("units");
    let service = units.join("com.example.alpha.service");
    fs::create_dir(&units).unwrap();
    fs::write(&service, render(&unit(), Platform::Linux).unwrap()).unwrap();

    assert_eq!(
        remove(&manager, &units, LABEL, &policy()),
        Ok(Removed::Removed)
    );
    assert!(!service.exists());
}

#[test]
fn linux_timer_unit_writes_nothing() {
    let scratch = Scratch::new();
    let manager = scratch.manager().with_platform(Platform::Linux);
    let units = scratch.0.join("units");
    let mut unit = unit();
    unit.run = Run::Interval(60);

    assert_eq!(
        apply(&manager, &units, &unit, &policy(), &mut || true),
        Err(ApplyError::Unit(UnitError::Unsupported))
    );
    assert!(!units.exists());
    assert!(manager.calls().is_empty());
}
