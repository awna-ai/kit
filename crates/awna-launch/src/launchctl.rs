use crate::command::{self, Output};
use crate::manager::resolve_program;
use crate::{
    Manager, ManagerError, Platform, ResolveError, State, parse_disabled, parse_print, valid_label,
};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SYSTEM_LAUNCHCTL: &str = "/bin/launchctl";

/// A manager backed by launchctl or a caller-supplied executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launchctl {
    program: PathBuf,
    uid: u32,
    timeout: Duration,
    unload_timeout: Duration,
}

impl Launchctl {
    /// Uses the given executable with 15 seconds per command and 60 seconds for unloading.
    pub fn new(program: impl Into<PathBuf>, uid: u32) -> Self {
        Self {
            program: program.into(),
            uid,
            timeout: Duration::from_secs(15),
            unload_timeout: Duration::from_secs(60),
        }
    }

    /// Uses the system launchctl executable.
    pub fn system(uid: u32) -> Self {
        Self::new(SYSTEM_LAUNCHCTL, uid)
    }

    /// Uses a nonempty stand-in when supplied and permits system launchctl only for the account's own home.
    ///
    /// Obtain `account_home` from the account's password-database entry rather than `$HOME`.
    /// [Self::new] and [Self::system] bypass this guard.
    pub fn resolve(
        stand_in: Option<&OsStr>,
        home: &Path,
        account_home: &Path,
        uid: u32,
    ) -> Result<Self, ResolveError> {
        let program = resolve_program(stand_in, home, account_home, Path::new(SYSTEM_LAUNCHCTL))?;
        Ok(Self::new(program, uid))
    }

    /// Sets the general command timeout and the separate timeout for unloading.
    pub fn with_timeouts(mut self, timeout: Duration, unload_timeout: Duration) -> Self {
        self.timeout = timeout;
        self.unload_timeout = unload_timeout;
        self
    }

    /// Returns the configured executable path.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Returns the graphical session domain for this user ID.
    pub fn domain(&self) -> String {
        format!("gui/{}", self.uid)
    }

    /// Appends a service label to the graphical session domain.
    pub fn target(&self, label: &str) -> String {
        format!("{}/{label}", self.domain())
    }

    fn checked_target(&self, label: &str) -> Result<String, ManagerError> {
        if !valid_label(label) {
            return Err(ManagerError::BadLabel);
        }
        Ok(self.target(label))
    }

    fn run(&self, verb: &'static str, args: &[&str]) -> Result<Output, ManagerError> {
        let timeout = if verb == "bootout" {
            self.unload_timeout
        } else {
            self.timeout
        };
        let mut argv = Vec::with_capacity(args.len() + 1);
        argv.push(verb);
        argv.extend_from_slice(args);
        command::run(&self.program, &argv, verb, timeout)
    }
}

impl Manager for Launchctl {
    fn uid(&self) -> u32 {
        self.uid
    }

    fn platform(&self) -> Platform {
        Platform::MacOs
    }

    fn session_ok(&self) -> Result<bool, ManagerError> {
        Ok(self.run("print", &[&self.domain()])?.status.success())
    }

    fn state(&self, label: &str) -> Result<State, ManagerError> {
        let target = self.checked_target(label)?;
        let verb = "print";
        let output = self.run(verb, &[&target])?;
        if output.status.success() {
            parse_print(&String::from_utf8_lossy(&output.stdout))
                .ok_or(ManagerError::Unparsed { verb })
        } else {
            Ok(State::NotLoaded)
        }
    }

    fn disabled(&self, label: &str) -> Result<bool, ManagerError> {
        if !valid_label(label) {
            return Err(ManagerError::BadLabel);
        }
        let verb = "print-disabled";
        let output = self.run(verb, &[&self.domain()])?;
        if !output.status.success() {
            return Err(command::failed(verb, &output));
        }
        Ok(parse_disabled(
            &String::from_utf8_lossy(&output.stdout),
            label,
        ))
    }

    fn load(&self, plist: &Path) -> Result<bool, ManagerError> {
        let path = plist
            .to_str()
            .filter(|_| plist.is_absolute())
            .ok_or(ManagerError::BadPath)?;
        Ok(self
            .run("bootstrap", &[&self.domain(), path])?
            .status
            .success())
    }

    fn unload(&self, label: &str) -> Result<bool, ManagerError> {
        let target = self.checked_target(label)?;
        Ok(self.run("bootout", &[&target])?.status.success())
    }

    fn restart(&self, label: &str) -> Result<bool, ManagerError> {
        let target = self.checked_target(label)?;
        Ok(self.run("kickstart", &["-k", &target])?.status.success())
    }
}
