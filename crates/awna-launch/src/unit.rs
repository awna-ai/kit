use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// The program and settings for a background service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    /// The service label used by the manager.
    pub label: String,
    /// The absolute program path followed by its arguments.
    pub argv: Vec<String>,
    /// The schedule or lifetime of the service.
    pub run: Run,
    /// The destinations for standard output and standard error.
    pub logs: Logs,
    /// Environment variables supplied to the service.
    pub env: BTreeMap<String, String>,
    /// The service's working directory.
    pub workdir: Option<PathBuf>,
    /// The scheduling priority adjustment, from -20 through 20.
    pub nice: Option<i8>,
    /// Whether to request low-priority I/O.
    pub low_io: bool,
    /// The process type supplied to the manager.
    pub kind: Option<Kind>,
    /// The file creation mask supplied to the service.
    pub umask: Option<u16>,
    /// The time in seconds allowed for the service to exit.
    pub exit_timeout: Option<u32>,
}

impl Unit {
    /// Creates a unit with only the label, arguments and run mode set.
    pub fn new(label: impl Into<String>, argv: Vec<String>, run: Run) -> Self {
        Self {
            label: label.into(),
            argv,
            run,
            logs: Logs::default(),
            env: BTreeMap::new(),
            workdir: None,
            nice: None,
            low_io: false,
            kind: None,
            umask: None,
            exit_timeout: None,
        }
    }

    /// Checks the unit and returns the first invalid field.
    pub fn validate(&self) -> Result<(), UnitError> {
        if !valid_label(&self.label) {
            return Err(UnitError::Label);
        }
        let program = self.argv.first().ok_or(UnitError::Argv)?;
        if !Path::new(program).is_absolute() {
            return Err(UnitError::Program);
        }
        if self.argv.iter().any(|arg| !valid_text(arg))
            || self
                .env
                .iter()
                .any(|(key, value)| !valid_text(key) || !valid_text(value))
        {
            return Err(UnitError::Text);
        }
        if [
            self.logs.stdout.as_deref(),
            self.logs.stderr.as_deref(),
            self.workdir.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|path| !path.is_absolute() || !path.to_str().is_some_and(valid_text))
        {
            return Err(UnitError::Path);
        }
        if self.env.keys().any(|key| !valid_env_key(key)) {
            return Err(UnitError::EnvKey);
        }
        match &self.run {
            Run::Interval(0) => return Err(UnitError::Interval),
            Run::Calendar(times) if times.is_empty() || times.iter().any(|time| !time.valid()) => {
                return Err(UnitError::Calendar);
            }
            _ => {}
        }
        if self.nice.is_some_and(|nice| !(-20..=20).contains(&nice)) {
            return Err(UnitError::Nice);
        }
        if self.umask.is_some_and(|mask| mask > 0o777) {
            return Err(UnitError::Umask);
        }
        if self.exit_timeout == Some(0) {
            return Err(UnitError::ExitTimeout);
        }
        Ok(())
    }
}

/// Determines when and how often the manager starts the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Run {
    /// Start at load and keep the service running.
    KeepAlive,
    /// Start at load and repeat at the given interval in seconds.
    Interval(u32),
    /// Start at any of the specified calendar times.
    Calendar(Vec<CalendarTime>),
    /// Start at load.
    Once,
}

/// A calendar trigger with a required minute and optional time constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarTime {
    /// Minute of the hour, from 0 through 59.
    pub minute: u8,
    /// Hour of the day, from 0 through 23.
    pub hour: Option<u8>,
    /// Day of the month, from 1 through 31.
    pub day: Option<u8>,
    /// Day of the week, from 0 through 7.
    pub weekday: Option<u8>,
    /// Month of the year, from 1 through 12.
    pub month: Option<u8>,
}

impl CalendarTime {
    fn valid(&self) -> bool {
        self.minute <= 59
            && self.hour.is_none_or(|hour| hour <= 23)
            && self.day.is_none_or(|day| (1..=31).contains(&day))
            && self.weekday.is_none_or(|weekday| weekday <= 7)
            && self.month.is_none_or(|month| (1..=12).contains(&month))
    }
}

/// Optional file destinations for service output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Logs {
    /// Where the manager writes standard output.
    pub stdout: Option<PathBuf>,
    /// Where the manager writes standard error.
    pub stderr: Option<PathBuf>,
}

impl Logs {
    /// Directs standard output and standard error to the same file.
    pub fn both(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        Self {
            stdout: Some(path.clone()),
            stderr: Some(path),
        }
    }
}

/// The process type supplied to the service manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A background process.
    Background,
    /// A standard process.
    Standard,
    /// An interactive process.
    Interactive,
    /// An adaptive process.
    Adaptive,
}

/// Checks whether a label is a safe service identifier and filename component.
pub fn valid_label(label: &str) -> bool {
    label.len() <= 200
        && label
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric())
        && !label.contains("..")
        && label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn valid_text(value: &str) -> bool {
    value.chars().all(|character| {
        !matches!(
            character,
            '\0'..='\u{1f}' | '\u{7f}' | '\u{fffe}' | '\u{ffff}'
        )
    })
}

fn valid_env_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The first validation rule a unit failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitError {
    /// The service label is invalid.
    Label,
    /// The argument list is empty.
    Argv,
    /// The program path is not absolute.
    Program,
    /// An argument or environment string contains a forbidden character.
    Text,
    /// A log or working directory path is invalid.
    Path,
    /// An environment key is invalid.
    EnvKey,
    /// The start interval is zero.
    Interval,
    /// The calendar has no times or has an out-of-range field.
    Calendar,
    /// The scheduling priority adjustment is out of range.
    Nice,
    /// The file creation mask is out of range.
    Umask,
    /// The exit timeout is zero.
    ExitTimeout,
}

impl fmt::Display for UnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Label => "The service label is invalid",
            Self::Argv => "The argument list needs a program",
            Self::Program => "The program path must be absolute",
            Self::Text => "An argument or environment string contains a forbidden character",
            Self::Path => "Log and working directory paths must be absolute UTF-8 without forbidden characters",
            Self::EnvKey => "An environment key is invalid",
            Self::Interval => "The start interval must be greater than zero",
            Self::Calendar => "The calendar must contain at least one valid time and no invalid times",
            Self::Nice => "The scheduling priority adjustment must be from -20 through 20",
            Self::Umask => "The file creation mask must be at most 0o777",
            Self::ExitTimeout => "The exit timeout must be greater than zero",
        })
    }
}

impl std::error::Error for UnitError {}
