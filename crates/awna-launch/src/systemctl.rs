use crate::command::{self, Output};
use crate::manager::resolve_program;
use crate::{Manager, ManagerError, Platform, ResolveError, State, valid_label};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SYSTEM_SYSTEMCTL: &str = "/usr/bin/systemctl";
const SHOW_PROPERTIES: &str = "--property=LoadState,ActiveState,MainPID,UnitFileState";

/// A systemd user manager backed by systemctl or a caller-supplied executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Systemctl {
    program: PathBuf,
    uid: u32,
    timeout: Duration,
    unload_timeout: Duration,
}

impl Systemctl {
    /// Uses the given executable with 15 seconds per command and 60 seconds for disabling and restarting.
    pub fn new(program: impl Into<PathBuf>, uid: u32) -> Self {
        Self {
            program: program.into(),
            uid,
            timeout: Duration::from_secs(15),
            unload_timeout: Duration::from_secs(60),
        }
    }

    /// Uses the system systemctl executable.
    pub fn system(uid: u32) -> Self {
        Self::new(SYSTEM_SYSTEMCTL, uid)
    }

    /// Uses a nonempty stand-in when supplied and permits systemctl only for the account's own home.
    ///
    /// Obtain `account_home` from the account's password-database entry rather than `$HOME`.
    /// [Self::new] and [Self::system] bypass this guard. The guard is advisory: a copy of the
    /// system program or a wrapper script passes it.
    pub fn resolve(
        stand_in: Option<&OsStr>,
        home: &Path,
        account_home: &Path,
        uid: u32,
    ) -> Result<Self, ResolveError> {
        let program = resolve_program(stand_in, home, account_home, Path::new(SYSTEM_SYSTEMCTL))?;
        Ok(Self::new(program, uid))
    }

    /// Sets the general command timeout and the separate timeout for disabling and restarting.
    pub fn with_timeouts(mut self, timeout: Duration, unload_timeout: Duration) -> Self {
        self.timeout = timeout;
        self.unload_timeout = unload_timeout;
        self
    }

    /// Returns the configured executable path.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Appends the systemd service suffix to the label.
    pub fn unit_name(label: &str) -> String {
        format!("{label}.service")
    }

    fn run(&self, verb: &'static str, args: &[&str]) -> Result<Output, ManagerError> {
        let timeout = match verb {
            "disable" | "restart" => self.unload_timeout,
            _ => self.timeout,
        };
        let mut argv = Vec::with_capacity(args.len() + 2);
        argv.extend(["--user", verb]);
        argv.extend_from_slice(args);
        command::run(&self.program, &argv, verb, timeout)
    }

    fn show<T>(
        &self,
        label: &str,
        parse: impl FnOnce(&str) -> Option<T>,
    ) -> Result<T, ManagerError> {
        let name = checked_name(label)?;
        let verb = "show";
        let output = self.run(verb, &[&name, SHOW_PROPERTIES])?;
        if !output.status.success() {
            return Err(command::failed(verb, &output));
        }
        parse(&String::from_utf8_lossy(&output.stdout)).ok_or(ManagerError::Unparsed { verb })
    }
}

fn checked_name(label: &str) -> Result<String, ManagerError> {
    if !valid_label(label) {
        return Err(ManagerError::BadLabel);
    }
    Ok(Systemctl::unit_name(label))
}

impl Manager for Systemctl {
    fn uid(&self) -> u32 {
        self.uid
    }

    fn platform(&self) -> Platform {
        Platform::Linux
    }

    fn session_ok(&self) -> Result<bool, ManagerError> {
        Ok(self.run("show-environment", &[])?.status.success())
    }

    fn state(&self, label: &str) -> Result<State, ManagerError> {
        self.show(label, parse_show)
    }

    fn disabled(&self, label: &str) -> Result<bool, ManagerError> {
        self.show(label, parse_masked)
    }

    fn load(&self, path: &Path) -> Result<bool, ManagerError> {
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .filter(|name| name.strip_suffix(".service").is_some_and(valid_label))
            .filter(|_| path.is_absolute() && path.to_str().is_some())
            .ok_or(ManagerError::BadPath)?;

        Ok(self.run("daemon-reload", &[])?.status.success()
            && self.run("enable", &["--now", name])?.status.success())
    }

    fn unload(&self, label: &str) -> Result<bool, ManagerError> {
        let name = checked_name(label)?;
        Ok(self.run("disable", &["--now", &name])?.status.success())
    }

    fn restart(&self, label: &str) -> Result<bool, ManagerError> {
        let name = checked_name(label)?;
        Ok(self.run("restart", &[&name])?.status.success())
    }
}

#[derive(Default)]
struct Properties<'a> {
    load: Option<&'a str>,
    active: Option<&'a str>,
    pid: Option<&'a str>,
    unit_file: Option<&'a str>,
}

impl<'a> Properties<'a> {
    fn parse(output: &'a str) -> Self {
        let mut properties = Self::default();
        for (key, value) in output.lines().filter_map(|line| line.split_once('=')) {
            match key {
                "LoadState" => properties.load = Some(value),
                "ActiveState" => properties.active = Some(value),
                "MainPID" => properties.pid = Some(value),
                "UnitFileState" => properties.unit_file = Some(value),
                _ => {}
            }
        }
        properties
    }
}

/// Interprets a service listing. Both `ActiveState` and `UnitFileState` must be present.
pub fn parse_show(output: &str) -> Option<State> {
    let properties = Properties::parse(output);
    let active = properties.active?;
    let unit_file = properties.unit_file?;
    if matches!(
        active,
        "active" | "activating" | "deactivating" | "reloading" | "refreshing"
    ) {
        let pid = properties.pid.and_then(|pid| pid.parse::<u32>().ok());
        Some(State::Running {
            pid: pid.filter(|&pid| pid > 0),
        })
    } else if matches!(
        unit_file,
        "enabled" | "enabled-runtime" | "linked" | "linked-runtime"
    ) {
        Some(State::Idle)
    } else {
        Some(State::NotLoaded)
    }
}

/// Reports whether a unit is masked. The listing must contain `UnitFileState`.
///
/// On Linux a mask is the disabled state that [crate::Policy::keep_disabled] keeps.
/// Unloading disables (unlinks) a unit without masking it, so a later [crate::apply] loads it again.
pub fn parse_masked(output: &str) -> Option<bool> {
    let properties = Properties::parse(output);
    Some(
        matches!(properties.unit_file?, "masked" | "masked-runtime")
            || properties.load == Some("masked"),
    )
}

/// Where logind keeps one empty file per user whose services outlive a logout.
pub const LINGER_DIR: &str = "/var/lib/systemd/linger";

/// Checks whether a Linux user's services outlive a logout.
///
/// A Linux user unit stops at logout unless its user lingers. This crate never enables
/// lingering. If it is off, callers can tell the user about `loginctl enable-linger` and carry on.
pub fn lingering(linger_dir: &Path, user: &str) -> io::Result<bool> {
    // logind escapes the bytes it cannot use in a file name, so only plain names match its flag file.
    if user.is_empty()
        || user.len() > 256
        || matches!(user, "." | "..")
        || user.bytes().any(|byte| {
            !(0x20..=0x7e).contains(&byte) || matches!(byte, b'/' | b'\\' | b'"' | b'\'')
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The user name is invalid",
        ));
    }
    // logind checks the flag file with access(), which follows links.
    match fs::metadata(linger_dir.join(user)) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_reads_active_state_and_pid() {
        assert_eq!(
            parse_show("ActiveState=active\nMainPID=4242\nUnitFileState=disabled"),
            Some(State::Running { pid: Some(4242) })
        );
        assert_eq!(
            parse_show("ActiveState=activating\nMainPID=0\nUnitFileState=enabled"),
            Some(State::Running { pid: None })
        );
        for state in ["reloading", "refreshing", "deactivating"] {
            assert_eq!(
                parse_show(&format!(
                    "ActiveState={state}\nMainPID=4294967296\nUnitFileState=enabled"
                )),
                Some(State::Running { pid: None })
            );
        }
    }

    #[test]
    fn show_reads_inactive_and_failed_units() {
        for file_state in ["enabled", "enabled-runtime", "linked", "linked-runtime"] {
            assert_eq!(
                parse_show(&format!("ActiveState=inactive\nUnitFileState={file_state}")),
                Some(State::Idle)
            );
        }
        assert_eq!(
            parse_show("ActiveState=failed\nUnitFileState=enabled-runtime"),
            Some(State::Idle)
        );
        assert_eq!(
            parse_show("ActiveState=inactive\nUnitFileState=disabled"),
            Some(State::NotLoaded)
        );
        assert_eq!(
            parse_show("LoadState=not-found\nActiveState=inactive\nUnitFileState="),
            Some(State::NotLoaded)
        );
        assert_eq!(
            parse_show("ActiveState=inactive\nUnitFileState=masked"),
            Some(State::NotLoaded)
        );
    }

    #[test]
    fn show_requires_both_keys_and_uses_the_last_whole_value() {
        assert_eq!(
            parse_show(
                "misformatted\nActiveState=inactive\nUnitFileState=enabled\nUnitFileState=enabled=extra"
            ),
            Some(State::NotLoaded)
        );
        assert_eq!(
            parse_show("ActiveState=\nUnitFileState="),
            Some(State::NotLoaded)
        );
        assert_eq!(parse_show("UnitFileState=enabled"), None);
        assert_eq!(parse_show("ActiveState=active"), None);
    }

    #[test]
    fn masked_checks_both_properties_and_requires_unit_file_state() {
        assert_eq!(parse_masked("UnitFileState=masked"), Some(true));
        assert_eq!(parse_masked("UnitFileState=masked-runtime"), Some(true));
        assert_eq!(parse_masked("LoadState=masked\nUnitFileState="), Some(true));
        assert_eq!(
            parse_masked("LoadState=masked\nLoadState=loaded\nUnitFileState=enabled"),
            Some(false)
        );
        assert_eq!(parse_masked("UnitFileState=enabled"), Some(false));
        assert_eq!(parse_masked("LoadState=masked"), None);
    }

    #[test]
    fn lingering_follows_links_like_logind() {
        use std::os::unix::fs::symlink;

        let dir = std::env::temp_dir().join(format!("awna-launch-linger-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        let result = (|| {
            fs::write(dir.join("example"), "")?;
            symlink(dir.join("missing"), dir.join("linked"))?;
            Ok::<_, io::Error>((
                lingering(&dir, "example")?,
                lingering(&dir, "missing")?,
                lingering(&dir, "linked")?,
            ))
        })();
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(result.unwrap(), (true, false, false));
    }

    #[test]
    fn lingering_rejects_invalid_names() {
        for user in [
            "",
            ".",
            "..",
            "a/b",
            "a\0b",
            &"a".repeat(257),
            "a\\b",
            "a\"b",
            "a'b",
            "\u{e9}",
            "a\tb",
        ] {
            assert_eq!(
                lingering(Path::new("/var/tmp/example/linger"), user).map_err(|error| error.kind()),
                Err(io::ErrorKind::InvalidInput)
            );
        }
    }
}
