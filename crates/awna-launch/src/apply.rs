use crate::{Manager, ManagerError, Platform, State, Unit, UnitError, render, valid_label};
use std::fmt;
use std::fs::{self, DirBuilder, Metadata, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::thread;
use std::time::Duration;

/// The state checks allowed after an unload request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wait {
    /// Maximum number of state checks. Zero counts as one.
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
    /// Skip an already loaded service whose unit file matches.
    pub skip_unchanged: bool,
    /// Preserve the service manager's disabled setting.
    ///
    /// On Linux only a masked unit counts as disabled: a unit the user disabled with
    /// `systemctl --user disable` is loaded again by the next `apply`.
    pub keep_disabled: bool,
    /// Refuse a change that would unload a running service.
    pub never_stop_running: bool,
    /// State checks after requesting an unload.
    pub bootout_wait: Wait,
    /// Attempts to load the new unit file.
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
    /// The new unit file was removed.
    Removed,
    /// The previous unit file was restored and loaded again only if it was loaded before.
    Restored,
    /// The previous state could not be recovered.
    Failed,
}

/// The result of removing a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removed {
    /// The service was unloaded or its unit file was removed.
    Removed,
    /// The service had neither a loaded job nor a unit file.
    NotInstalled,
}

/// A failure while applying or removing a service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// The service definition or label is invalid.
    Unit(UnitError),
    /// The user's service session is unavailable.
    NoSession,
    /// The path has unsafe ownership, permissions, spelling, or file type.
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
    /// The manager reported an error loading the service.
    LoadErrored {
        /// The underlying manager error.
        error: ManagerError,
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
            Self::NoSession => f.write_str("The user's service session is unavailable"),
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
            Self::LoadErrored { error, rollback } => {
                write!(f, "{error} (rollback: {rollback:?})")
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
            Self::Launchctl(error) | Self::LoadErrored { error, .. } => Some(error),
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

fn safe_directory(metadata: &Metadata, uid: u32) -> bool {
    metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o022 == 0
}

fn regular_file(path: &Path, uid: u32) -> Result<bool, ApplyError> {
    match metadata(path)? {
        Some(metadata) if metadata.is_file() && metadata.uid() == uid => Ok(true),
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

fn missing_logs(unit: &Unit, uid: u32) -> Result<Vec<&Path>, ApplyError> {
    let mut missing = Vec::new();
    for path in log_paths(unit) {
        if let Some(parent) = path.parent() {
            let entry: PathBuf = parent.components().collect();
            if metadata(&entry)?.is_some_and(|metadata| !safe_directory(&metadata, uid)) {
                return Err(ApplyError::Unsafe(parent.into()));
            }
        }
        if !regular_file(path, uid)? && !missing.contains(&path) {
            missing.push(path);
        }
    }
    Ok(missing)
}

fn prepare_logs(missing: &[&Path], uid: u32) -> Result<(), ApplyError> {
    for path in missing {
        if let Some(parent) = path.parent() {
            make_dir(parent, 0o700)?;
            let entry: PathBuf = parent.components().collect();
            if !metadata(&entry)?.is_some_and(|metadata| safe_directory(&metadata, uid)) {
                return Err(ApplyError::Unsafe(parent.into()));
            }
        }
        let result = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path);
        if let Err(error) = result {
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(io_error("create log", path, error));
            }
            // Accept only a file of the manager's user that is not a link to another file.
            if !metadata(path)?.is_some_and(|metadata| {
                metadata.is_file() && metadata.uid() == uid && metadata.nlink() == 1
            }) {
                return Err(ApplyError::Unsafe((*path).into()));
            }
        }
    }
    Ok(())
}

fn write_unit(path: &Path, bytes: &[u8]) -> Result<(), ApplyError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("unit path has a UTF-8 file name");
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|error| io_error("create temporary unit file", &temporary, error))?;
    let result = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error("write temporary unit file", &temporary, error))
        .and_then(|()| {
            fs::rename(&temporary, path).map_err(|error| io_error("replace unit file", path, error))
        });
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

/// Reports whether a Linux unit path is a mask: a symbolic link to `/dev/null`.
fn masked(path: &Path, platform: Platform) -> Result<bool, ApplyError> {
    if platform != Platform::Linux
        || !metadata(path)?.is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Ok(false);
    }
    let target = fs::read_link(path).map_err(|error| io_error("read link", path, error))?;
    Ok(target == Path::new("/dev/null"))
}

fn delete_unit(path: &Path) -> Result<bool, ApplyError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error("remove unit file", path, error)),
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
    for check in 0..wait.checks.max(1) {
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
    path: &Path,
    current: Option<&[u8]>,
    was_loaded: bool,
) -> Rollback {
    match current {
        Some(bytes)
            if write_unit(path, bytes).is_ok()
                && (!was_loaded || manager.load(path) == Ok(true)) =>
        {
            Rollback::Restored
        }
        None if delete_unit(path).is_ok() => Rollback::Removed,
        _ => Rollback::Failed,
    }
}

/// Returns the unit file path for a label. Callers building paths directly must validate the label first.
pub fn unit_path(units_dir: &Path, label: &str, platform: Platform) -> PathBuf {
    let extension = match platform {
        Platform::MacOs => "plist",
        Platform::Linux => "service",
    };
    units_dir.join(format!("{label}.{extension}"))
}

/// Returns the plist path for a label. Callers building paths directly must validate the label first.
pub fn plist_path(units_dir: &Path, label: &str) -> PathBuf {
    unit_path(units_dir, label, Platform::MacOs)
}

/// Installs or updates a service and checks its health after loading it.
///
/// `units_dir` is `~/Library/LaunchAgents` for launchd and `~/.config/systemd/user` for systemd user
/// units. On Linux it must be a directory on the systemd user unit search path, because systemd finds
/// the unit by name. A unit path there that links to `/dev/null` is a mask: with `keep_disabled` it
/// counts as disabled and is left as it is, and `remove` never deletes it.
/// The units directory must be absolute and contain no parent components. Existing
/// unit files, log files, the units directory, and immediate log parents must belong
/// to the manager's user. The two kinds of directories must also exclude group
/// and other write access. Only the immediate log parent is checked. Callers
/// choose which ancestors to trust, including system links such as `/var`.
/// The special log destination `/dev/null` is exempt.
pub fn apply<M: Manager + ?Sized>(
    manager: &M,
    units_dir: &Path,
    unit: &Unit,
    policy: &Policy,
    health: &mut dyn FnMut() -> bool,
) -> Result<Applied, ApplyError> {
    let platform = manager.platform();
    let rendered = render(unit, platform).map_err(ApplyError::Unit)?;
    require_session(manager)?;
    if !units_dir.is_absolute()
        || units_dir
            .components()
            .any(|part| part == Component::ParentDir)
    {
        return Err(ApplyError::Unsafe(units_dir.into()));
    }
    let uid = manager.uid();
    // Strip trailing separators and "." so metadata inspects the final component itself.
    let directory_entry: PathBuf = units_dir.components().collect();
    let dir = metadata(&directory_entry)?;
    if dir
        .as_ref()
        .is_some_and(|metadata| !safe_directory(metadata, uid))
    {
        return Err(ApplyError::Unsafe(units_dir.into()));
    }
    let label = &unit.label;
    let path = unit_path(units_dir, label, platform);
    if policy.keep_disabled && masked(&path, platform)? {
        return Ok(Applied::LeftDisabled);
    }
    let current = regular_file(&path, uid)?
        .then(|| fs::read(&path).map_err(|error| io_error("read unit file", &path, error)))
        .transpose()?;
    let logs = missing_logs(unit, uid)?;

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
                make_dir(units_dir, 0o700)?;
            }
            prepare_logs(&logs, uid)?;
            write_unit(&path, bytes)?;
        }
        return Ok(Applied::LeftDisabled);
    }
    protect_running(&state, policy)?;

    if dir.is_none() {
        make_dir(units_dir, 0o700)?;
    }
    prepare_logs(&logs, uid)?;
    if loaded {
        unload(manager, label, policy.bootout_wait)?;
    }
    if let Err(error) = write_unit(&path, bytes) {
        if loaded && current.is_some() {
            let _ = manager.load(&path);
        }
        return Err(error);
    }

    let revert = || rollback(manager, &path, current.as_deref(), loaded);
    let revert_failed_load = || match manager.state(label) {
        Ok(State::NotLoaded) => revert(),
        Ok(_) if unload(manager, label, policy.bootout_wait).is_ok() => revert(),
        _ => Rollback::Failed,
    };
    match load(manager, &path, policy.bootstrap_tries) {
        Ok(true) => {}
        Ok(false) => {
            return Err(ApplyError::LoadFailed {
                tries: policy.bootstrap_tries.tries,
                rollback: revert_failed_load(),
            });
        }
        Err(error) => {
            return Err(ApplyError::LoadErrored {
                error,
                rollback: revert_failed_load(),
            });
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

/// Unloads a service if needed and removes its unit file.
pub fn remove<M: Manager + ?Sized>(
    manager: &M,
    units_dir: &Path,
    label: &str,
    policy: &Policy,
) -> Result<Removed, ApplyError> {
    if !valid_label(label) {
        return Err(ApplyError::Unit(UnitError::Label));
    }
    require_session(manager)?;
    let path = unit_path(units_dir, label, manager.platform());
    if masked(&path, manager.platform())? {
        return Ok(Removed::NotInstalled);
    }
    if metadata(&path)?.is_some_and(|metadata| metadata.is_dir()) {
        return Err(ApplyError::Unsafe(path));
    }
    let state = manager.state(label).map_err(ApplyError::Launchctl)?;
    protect_running(&state, policy)?;
    let loaded = state != State::NotLoaded;
    if loaded {
        unload(manager, label, policy.bootout_wait)?;
    }
    let deleted = delete_unit(&path)?;
    Ok(if loaded || deleted {
        Removed::Removed
    } else {
        Removed::NotInstalled
    })
}

/// Removes the listed services in order and returns on the first error.
pub fn retire<M: Manager + ?Sized>(
    manager: &M,
    units_dir: &Path,
    old_labels: &[&str],
    policy: &Policy,
) -> Result<Vec<Removed>, ApplyError> {
    old_labels
        .iter()
        .map(|label| remove(manager, units_dir, label, policy))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{ApplyError, missing_logs, prepare_logs, regular_file};
    use crate::{Run, Unit};
    use std::fs::{self, DirBuilder, File};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt, symlink};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let path = std::env::temp_dir().join(format!(
                "example-apply-helpers-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn regular_file_requires_matching_owner() {
        let scratch = Scratch::new();
        let path = scratch.0.join("com.example.alpha.plist");
        let own_uid = File::create(&path).unwrap().metadata().unwrap().uid();

        assert_eq!(regular_file(&path, own_uid), Ok(true));
        assert_eq!(
            regular_file(&path, own_uid + 1),
            Err(ApplyError::Unsafe(path))
        );
    }

    #[test]
    fn regular_file_distinguishes_missing_paths_from_directories() {
        let scratch = Scratch::new();
        let own_uid = File::create(scratch.0.join("owner"))
            .unwrap()
            .metadata()
            .unwrap()
            .uid();

        assert_eq!(regular_file(&scratch.0.join("missing"), own_uid), Ok(false));
        assert_eq!(
            regular_file(&scratch.0, own_uid),
            Err(ApplyError::Unsafe(scratch.0.clone()))
        );
    }

    #[test]
    fn missing_logs_accepts_an_existing_log_for_its_owner() {
        let scratch = Scratch::new();
        let path = scratch.0.join("output.log");
        let own_uid = File::create(&path).unwrap().metadata().unwrap().uid();
        let mut unit = Unit::new(
            "com.example.alpha",
            vec!["/opt/example/bin/alpha".into()],
            Run::Once,
        );
        unit.logs.stdout = Some(path);

        assert!(missing_logs(&unit, own_uid).unwrap().is_empty());
        assert!(matches!(
            missing_logs(&unit, own_uid + 1),
            Err(ApplyError::Unsafe(_))
        ));
    }

    #[test]
    fn prepare_logs_accepts_an_owned_file_that_appeared() {
        let scratch = Scratch::new();
        let path = scratch.0.join("output.log");
        fs::write(&path, b"existing data").unwrap();
        let uid = fs::metadata(&path).unwrap().uid();

        assert_eq!(prepare_logs(&[&path], uid), Ok(()));
        assert_eq!(fs::read(&path).unwrap(), b"existing data");
    }

    #[test]
    fn prepare_logs_rejects_a_directory_that_appeared() {
        let scratch = Scratch::new();
        let path = scratch.0.join("output.log");
        fs::create_dir(&path).unwrap();
        let uid = fs::metadata(&path).unwrap().uid();

        assert_eq!(prepare_logs(&[&path], uid), Err(ApplyError::Unsafe(path)));
    }

    #[test]
    fn prepare_logs_rejects_a_symlink_that_appeared() {
        let scratch = Scratch::new();
        let path = scratch.0.join("output.log");
        let target = scratch.0.join("target.log");
        let uid = File::create(&target).unwrap().metadata().unwrap().uid();
        symlink(&target, &path).unwrap();

        assert_eq!(prepare_logs(&[&path], uid), Err(ApplyError::Unsafe(path)));
    }

    #[test]
    fn prepare_logs_rejects_a_hard_link_that_appeared() {
        let scratch = Scratch::new();
        let target = scratch.0.join("profile");
        fs::write(&target, b"keep").unwrap();
        let uid = fs::metadata(&target).unwrap().uid();
        let path = scratch.0.join("output.log");
        fs::hard_link(&target, &path).unwrap();

        assert_eq!(prepare_logs(&[&path], uid), Err(ApplyError::Unsafe(path)));
        assert_eq!(fs::read(&target).unwrap(), b"keep");
    }

    #[test]
    fn prepare_logs_rejects_a_parent_that_appeared_unsafe() {
        let scratch = Scratch::new();
        let parent = scratch.0.join("logs");
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o777)).unwrap();
        let uid = fs::metadata(&parent).unwrap().uid();
        let path = parent.join("output.log");

        assert_eq!(prepare_logs(&[&path], uid), Err(ApplyError::Unsafe(parent)));
        assert!(!path.exists());
    }
}
