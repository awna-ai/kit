use crate::{Manager, ManagerError, Platform, State, Unit, UnitError, render, valid_label};
use std::fmt;
use std::fs::{self, DirBuilder, Metadata, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

/// The state checks allowed after an unload request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wait {
    /// Maximum number of state checks.
    pub checks: u32,
    /// Delay between consecutive checks.
    pub pause: Duration,
}

/// The attempts allowed to load a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tries {
    /// Maximum number of load attempts.
    pub tries: u32,
    /// Delay between consecutive attempts.
    pub pause: Duration,
}

/// Controls how service changes are applied and retried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// Skip an already loaded service whose plist matches.
    pub skip_unchanged: bool,
    /// Preserve the service manager's disabled setting.
    pub keep_disabled: bool,
    /// Refuse a change that would unload a running service.
    pub never_stop_running: bool,
    /// State checks after requesting an unload.
    pub bootout_wait: Wait,
    /// Attempts to load the new plist.
    pub bootstrap_tries: Tries,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            skip_unchanged: true,
            keep_disabled: true,
            never_stop_running: true,
            bootout_wait: Wait {
                checks: 10,
                pause: Duration::from_millis(500),
            },
            bootstrap_tries: Tries {
                tries: 3,
                pause: Duration::from_secs(1),
            },
        }
    }
}

/// The result of applying a service definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// The matching service was already loaded.
    Unchanged,
    /// The definition was saved while honoring the disabled setting.
    LeftDisabled,
    /// The service was loaded.
    Loaded,
    /// The previously loaded service was replaced.
    Reloaded,
}

/// The result of reverting a failed installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rollback {
    /// The new plist was removed.
    Removed,
    /// The previous plist was restored and loaded again only if it was loaded before.
    Restored,
    /// The previous state could not be recovered.
    Failed,
}

/// The result of removing a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removed {
    /// The service was unloaded or its plist was removed.
    Removed,
    /// The service had neither a loaded job nor a plist.
    NotInstalled,
}

/// A failure while applying or removing a service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// The service definition or label is invalid.
    Unit(UnitError),
    /// The manager could not find a graphical session.
    NoSession,
    /// The path has unsafe permissions or an unexpected file type.
    Unsafe(PathBuf),
    /// A file operation failed.
    Io {
        /// The attempted operation.
        what: &'static str,
        /// The path involved in the operation.
        path: PathBuf,
        /// The underlying I/O error kind.
        kind: io::ErrorKind,
    },
    /// The service manager returned an error.
    Launchctl(ManagerError),
    /// The policy protects a running service from being unloaded.
    Running {
        /// The process ID when available.
        pid: Option<u32>,
    },
    /// The service remained loaded after the allotted checks.
    StillLoaded,
    /// The service failed to load within the allotted attempts.
    LoadFailed {
        /// The number of attempts made.
        tries: u32,
        /// The result of reverting the failed installation.
        rollback: Rollback,
    },
    /// The service failed its health check after loading.
    Unhealthy {
        /// The result of reverting the unhealthy installation.
        rollback: Rollback,
    },
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit(error) => error.fmt(f),
            Self::NoSession => f.write_str("The graphical session is unavailable"),
            Self::Unsafe(path) => write!(f, "The path {} is unsafe", path.display()),
            Self::Io { what, path, kind } => {
                write!(f, "Could not {what} at {} ({kind})", path.display())
            }
            Self::Launchctl(error) => error.fmt(f),
            Self::Running { pid: Some(pid) } => write!(f, "The service is running with PID {pid}"),
            Self::Running { pid: None } => f.write_str("The service is running"),
            Self::StillLoaded => f.write_str("The service is still loaded after the unload wait"),
            Self::LoadFailed { tries, rollback } => {
                write!(
                    f,
                    "The service could not load after {tries} attempts (rollback: {rollback:?})"
                )
            }
            Self::Unhealthy { rollback } => {
                write!(
                    f,
                    "The service failed its health check (rollback: {rollback:?})"
                )
            }
        }
    }
}

impl std::error::Error for ApplyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unit(error) => Some(error),
            Self::Launchctl(error) => Some(error),
            _ => None,
        }
    }
}

fn io_error(what: &'static str, path: &Path, error: io::Error) -> ApplyError {
    ApplyError::Io {
        what,
        path: path.into(),
        kind: error.kind(),
    }
}

fn metadata(path: &Path) -> Result<Option<Metadata>, ApplyError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("inspect path", path, error)),
    }
}

fn regular_file(path: &Path) -> Result<bool, ApplyError> {
    match metadata(path)? {
        Some(metadata) if metadata.is_file() => Ok(true),
        Some(_) => Err(ApplyError::Unsafe(path.into())),
        None => Ok(false),
    }
}

fn make_dir(path: &Path, mode: u32) -> Result<(), ApplyError> {
    DirBuilder::new()
        .recursive(true)
        .mode(mode)
        .create(path)
        .map_err(|error| io_error("create directory", path, error))
}

fn log_paths(unit: &Unit) -> impl Iterator<Item = &Path> {
    [unit.logs.stdout.as_deref(), unit.logs.stderr.as_deref()]
        .into_iter()
        .flatten()
        .filter(|path| *path != Path::new("/dev/null"))
}

fn prepare_logs(unit: &Unit) -> Result<(), ApplyError> {
    for path in log_paths(unit) {
        if regular_file(path)? {
            continue;
        }
        if let Some(parent) = path.parent() {
            make_dir(parent, 0o700)?;
        }
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && regular_file(path)? => {}
            Err(error) => return Err(io_error("create log", path, error)),
        }
    }
    Ok(())
}

fn write_plist(dir: &Path, label: &str, path: &Path, bytes: &[u8]) -> Result<(), ApplyError> {
    let temporary = dir.join(format!(".{label}.plist.{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&temporary)
        .map_err(|error| io_error("create temporary plist", &temporary, error))?;
    let result = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error("write temporary plist", &temporary, error))
        .and_then(|()| {
            fs::rename(&temporary, path).map_err(|error| io_error("replace plist", path, error))
        });
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn delete_plist(path: &Path) -> Result<bool, ApplyError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error("remove plist", path, error)),
    }
}

fn require_session(manager: &(impl Manager + ?Sized)) -> Result<(), ApplyError> {
    if manager.session_ok().map_err(ApplyError::Launchctl)? {
        Ok(())
    } else {
        Err(ApplyError::NoSession)
    }
}

fn protect_running(state: &State, policy: &Policy) -> Result<(), ApplyError> {
    if policy.never_stop_running
        && let State::Running { pid } = state
    {
        return Err(ApplyError::Running { pid: *pid });
    }
    Ok(())
}

fn unload(manager: &(impl Manager + ?Sized), label: &str, wait: Wait) -> Result<(), ApplyError> {
    manager.unload(label).map_err(ApplyError::Launchctl)?;
    for check in 0..wait.checks {
        if check > 0 {
            thread::sleep(wait.pause);
        }
        if manager.state(label).map_err(ApplyError::Launchctl)? == State::NotLoaded {
            return Ok(());
        }
    }
    Err(ApplyError::StillLoaded)
}

fn load(
    manager: &(impl Manager + ?Sized),
    path: &Path,
    tries: Tries,
) -> Result<bool, ManagerError> {
    for attempt in 0..tries.tries {
        if attempt > 0 {
            thread::sleep(tries.pause);
        }
        if manager.load(path)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn rollback(
    manager: &(impl Manager + ?Sized),
    dir: &Path,
    label: &str,
    path: &Path,
    current: Option<&[u8]>,
    was_loaded: bool,
) -> Rollback {
    match current {
        Some(bytes)
            if write_plist(dir, label, path, bytes).is_ok()
                && (!was_loaded || manager.load(path) == Ok(true)) =>
        {
            Rollback::Restored
        }
        None if delete_plist(path).is_ok() => Rollback::Removed,
        _ => Rollback::Failed,
    }
}

/// Returns the plist path for a label. Callers building paths directly must validate the label first.
pub fn plist_path(agents_dir: &Path, label: &str) -> PathBuf {
    agents_dir.join(format!("{label}.plist"))
}

/// Installs or updates a service and checks its health after loading it.
pub fn apply<M: Manager + ?Sized>(
    manager: &M,
    agents_dir: &Path,
    unit: &Unit,
    policy: &Policy,
    health: &mut dyn FnMut() -> bool,
) -> Result<Applied, ApplyError> {
    let rendered = render(unit, Platform::MacOs).map_err(ApplyError::Unit)?;
    require_session(manager)?;
    // Strip trailing separators and "." so metadata inspects the final component itself.
    let directory_entry: PathBuf = agents_dir.components().collect();
    let dir = metadata(&directory_entry)?;
    if let Some(metadata) = &dir
        && (!metadata.is_dir() || metadata.permissions().mode() & 0o022 != 0)
    {
        return Err(ApplyError::Unsafe(agents_dir.into()));
    }
    let label = &unit.label;
    let path = plist_path(agents_dir, label);
    let current = regular_file(&path)?
        .then(|| fs::read(&path).map_err(|error| io_error("read plist", &path, error)))
        .transpose()?;
    for path in log_paths(unit) {
        regular_file(path)?;
    }

    let state = manager.state(label).map_err(ApplyError::Launchctl)?;
    let loaded = state != State::NotLoaded;
    let bytes = rendered.as_bytes();
    let unchanged = current.as_deref() == Some(bytes);
    if policy.skip_unchanged && unchanged && loaded {
        return Ok(Applied::Unchanged);
    }
    if policy.keep_disabled && manager.disabled(label).map_err(ApplyError::Launchctl)? {
        if !unchanged {
            if dir.is_none() {
                make_dir(agents_dir, 0o755)?;
            }
            write_plist(agents_dir, label, &path, bytes)?;
        }
        return Ok(Applied::LeftDisabled);
    }
    protect_running(&state, policy)?;

    if dir.is_none() {
        make_dir(agents_dir, 0o755)?;
    }
    prepare_logs(unit)?;
    if loaded {
        unload(manager, label, policy.bootout_wait)?;
    }
    if let Err(error) = write_plist(agents_dir, label, &path, bytes) {
        if loaded && current.is_some() {
            let _ = manager.load(&path);
        }
        return Err(error);
    }

    let revert = || {
        rollback(
            manager,
            agents_dir,
            label,
            &path,
            current.as_deref(),
            loaded,
        )
    };
    match load(manager, &path, policy.bootstrap_tries) {
        Ok(true) => {}
        Ok(false) => {
            return Err(ApplyError::LoadFailed {
                tries: policy.bootstrap_tries.tries,
                rollback: revert(),
            });
        }
        Err(error) => {
            revert();
            return Err(ApplyError::Launchctl(error));
        }
    }
    if !health() {
        let rollback = if unload(manager, label, policy.bootout_wait).is_ok() {
            revert()
        } else {
            Rollback::Failed
        };
        return Err(ApplyError::Unhealthy { rollback });
    }
    Ok(if loaded {
        Applied::Reloaded
    } else {
        Applied::Loaded
    })
}

/// Unloads a service if needed and removes its plist.
pub fn remove<M: Manager + ?Sized>(
    manager: &M,
    agents_dir: &Path,
    label: &str,
    policy: &Policy,
) -> Result<Removed, ApplyError> {
    if !valid_label(label) {
        return Err(ApplyError::Unit(UnitError::Label));
    }
    require_session(manager)?;
    let path = plist_path(agents_dir, label);
    if metadata(&path)?.is_some_and(|metadata| metadata.is_dir()) {
        return Err(ApplyError::Unsafe(path));
    }
    let state = manager.state(label).map_err(ApplyError::Launchctl)?;
    protect_running(&state, policy)?;
    let loaded = state != State::NotLoaded;
    if loaded {
        unload(manager, label, policy.bootout_wait)?;
    }
    let deleted = delete_plist(&path)?;
    Ok(if loaded || deleted {
        Removed::Removed
    } else {
        Removed::NotInstalled
    })
}

/// Removes the listed services in order and returns on the first error.
pub fn retire<M: Manager + ?Sized>(
    manager: &M,
    agents_dir: &Path,
    old_labels: &[&str],
    policy: &Policy,
) -> Result<Vec<Removed>, ApplyError> {
    old_labels
        .iter()
        .map(|label| remove(manager, agents_dir, label, policy))
        .collect()
}
