use crate::{Manager, ManagerError, State, parse_disabled, parse_print, valid_label};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

const SYSTEM_LAUNCHCTL: &str = "/bin/launchctl";
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const OUTPUT_LIMIT: usize = 1024 * 1024;

/// A manager backed by launchctl or a caller-supplied executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launchctl {
    program: PathBuf,
    uid: u32,
    timeout: Duration,
    unload_timeout: Duration,
}

/// An error choosing the manager executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveError {
    /// A home other than the account's own requires a stand-in.
    ForeignHome,
    /// A supplied stand-in is relative.
    StandInNotAbsolute,
    /// A supplied stand-in resolves to system launchctl.
    StandInIsSystem,
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ForeignHome => "Pass a stand-in because this home is outside the account's own home and the real launchctl is skipped",
            Self::StandInNotAbsolute => "The stand-in path must be absolute",
            Self::StandInIsSystem => "The stand-in must differ from system launchctl",
        })
    }
}

impl std::error::Error for ResolveError {}

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
        if let Some(program) = stand_in.filter(|program| !program.is_empty()) {
            let program = Path::new(program);
            if !program.is_absolute() {
                return Err(ResolveError::StandInNotAbsolute);
            }
            if program == Path::new(SYSTEM_LAUNCHCTL)
                || matches!(
                    (fs::canonicalize(program), fs::canonicalize(SYSTEM_LAUNCHCTL)),
                    (Ok(candidate), Ok(system)) if candidate == system
                )
            {
                return Err(ResolveError::StandInIsSystem);
            }
            Ok(Self::new(program, uid))
        } else if home == account_home {
            Ok(Self::system(uid))
        } else {
            Err(ResolveError::ForeignHome)
        }
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
        let mut child = Command::new(&self.program)
            .arg(verb)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(spawn_error)?;
        let start = Instant::now();
        let result = collect(&mut child, verb, start, timeout);
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        result
    }
}

impl Manager for Launchctl {
    fn uid(&self) -> u32 {
        self.uid
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
            let stderr = String::from_utf8_lossy(&output.stderr)
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .chars()
                .take(200)
                .collect();
            return Err(ManagerError::Failed {
                verb,
                code: output.status.code(),
                stderr,
            });
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

struct Output {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn spawn_error(error: io::Error) -> ManagerError {
    ManagerError::Spawn(error.kind())
}

fn collect(
    child: &mut Child,
    verb: &'static str,
    start: Instant,
    timeout: Duration,
) -> Result<Output, ManagerError> {
    let stdout = drain(child.stdout.take()).map_err(spawn_error)?;
    let stderr = drain(child.stderr.take()).map_err(spawn_error)?;

    let status = loop {
        if let Some(status) = child.try_wait().map_err(spawn_error)? {
            break status;
        }
        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return Err(ManagerError::TimedOut { verb });
        }
        thread::sleep(POLL_INTERVAL.min(remaining));
    };
    // A descendant can retain either pipe after launchctl exits.
    let stdout = stdout
        .recv_timeout(timeout.saturating_sub(start.elapsed()))
        .unwrap_or_default();
    let stderr = stderr
        .recv_timeout(timeout.saturating_sub(start.elapsed()))
        .unwrap_or_default();
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> io::Result<Receiver<Vec<u8>>> {
    let (send, receive) = mpsc::channel();
    if let Some(mut pipe) = pipe {
        thread::Builder::new().spawn(move || {
            let mut output = Vec::new();
            let mut prefix = (&mut pipe).take(OUTPUT_LIMIT as u64);
            if prefix.read_to_end(&mut output).is_ok() {
                let _ = io::copy(&mut pipe, &mut io::sink());
            }
            let _ = send.send(output);
        })?;
    }
    Ok(receive)
}
