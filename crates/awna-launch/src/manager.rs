use std::fmt;
use std::io;
use std::path::Path;

/// The state of a service in the current session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// The service is absent from the session.
    NotLoaded,
    /// The service is loaded and has no active process.
    Idle,
    /// The service has an active process.
    Running {
        /// The reported process ID, if available.
        pid: Option<u32>,
    },
}

/// An error checking or controlling the service manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagerError {
    /// The service label is invalid.
    BadLabel,
    /// The plist path is relative or cannot be represented as UTF-8.
    BadPath,
    /// Starting or supervising the process failed.
    Spawn(io::ErrorKind),
    /// The command exceeded its time limit.
    TimedOut {
        /// The command verb.
        verb: &'static str,
    },
    /// The command exited unsuccessfully.
    Failed {
        /// The command verb.
        verb: &'static str,
        /// The exit code, if the process exited normally.
        code: Option<i32>,
        /// A bounded excerpt from the first line of standard error.
        stderr: String,
    },
}

impl fmt::Display for ManagerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadLabel => f.write_str("The service label is invalid"),
            Self::BadPath => f.write_str("The plist path must be absolute and valid UTF-8"),
            Self::Spawn(kind) => write!(f, "The service manager process could not be run ({kind})"),
            Self::TimedOut { verb } => write!(f, "The {verb} command timed out"),
            Self::Failed { verb, code, .. } => match code {
                Some(code) => write!(f, "The {verb} command exited with status {code}"),
                None => write!(f, "The {verb} command terminated unsuccessfully"),
            },
        }
    }
}

impl std::error::Error for ManagerError {}

/// Checks and controls services in a user session.
pub trait Manager {
    /// Checks whether the session is available.
    fn session_ok(&self) -> Result<bool, ManagerError>;

    /// Queries the current state of a service.
    fn state(&self, label: &str) -> Result<State, ManagerError>;

    /// Queries whether a service is disabled.
    fn disabled(&self, label: &str) -> Result<bool, ManagerError>;

    /// Loads a service from its plist and reports whether the command succeeded.
    fn load(&self, plist: &Path) -> Result<bool, ManagerError>;

    /// Unloads a service and reports whether the command succeeded.
    fn unload(&self, label: &str) -> Result<bool, ManagerError>;

    /// Restarts a service and reports whether the command succeeded.
    fn restart(&self, label: &str) -> Result<bool, ManagerError>;
}

/// Interprets the top-level state and pid fields in a successful service listing.
pub fn parse_print(output: &str) -> State {
    let mut running = false;
    let mut pid = None;
    for line in output.lines().filter_map(|line| line.strip_prefix('\t')) {
        if !line.chars().next().is_some_and(|ch| !ch.is_whitespace()) {
            continue;
        }
        if line.trim_end() == "state = running" {
            running = true;
        }
        if let Some(value) = line.strip_prefix("pid = ")
            && let Ok(value) = value.trim().parse()
        {
            pid = Some(value);
        }
    }
    if running || pid.is_some() {
        State::Running { pid }
    } else {
        State::Idle
    }
}

/// Reports whether the exact label is marked disabled in a disabled-service listing.
pub fn parse_disabled(output: &str, label: &str) -> bool {
    let quoted_label = format!("\"{label}\"");
    output.lines().any(|line| {
        let mut words = line.split_whitespace();
        words.next() == Some(quoted_label.as_str())
            && words.next() == Some("=>")
            && matches!(words.next(), Some("disabled" | "true"))
            && words.next().is_none()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_reads_top_level_fields_in_a_service() {
        let output = concat!(
            "gui/501/com.example.alpha = {\n",
            "\tactive count = 1\n",
            "\tpath = /var/tmp/example/LaunchAgents/com.example.alpha.plist\n",
            "\ttype = LaunchAgent\n",
            "\tstate = running\n\n",
            "\tprogram = /opt/example/bin/alpha\n",
            "\tendpoints = {\n",
            "\t\t\"com.example.alpha.port\" = {\n",
            "\t\t\tport = 0\n",
            "\t\t\tstate = active\n",
            "\t\t\tpid = 999\n",
            "\t\t}\n",
            "\t}\n",
            "\truns = 1\n",
            "\tpid = 37027\n",
            "\tlast exit code = 0\n}\n",
        );
        assert_eq!(parse_print(output), State::Running { pid: Some(37027) });
    }

    #[test]
    fn print_ignores_nested_fields_and_nonstandard_indentation() {
        assert_eq!(
            parse_print("\tstate = not running\n\t\tpid = 1"),
            State::Idle
        );
        assert_eq!(parse_print("\t\tstate = running"), State::Idle);
        assert_eq!(
            parse_print("\t state = running\n  pid = 2\npid = 3"),
            State::Idle
        );
    }

    #[test]
    fn either_running_state_or_pid_indicates_a_live_service() {
        assert_eq!(
            parse_print("\tstate = running \n"),
            State::Running { pid: None }
        );
        assert_eq!(
            parse_print("\tstate = not running\n\tpid =  42 "),
            State::Running { pid: Some(42) }
        );
        assert_eq!(parse_print("\tpid = 4294967296\n\tpid = -1"), State::Idle);
    }

    #[test]
    fn braces_inside_environment_values_do_not_hide_service_fields() {
        let output = concat!(
            "gui/501/com.example.alpha = {\n",
            "\tenvironment = {\n",
            "\t\tSAMPLE => fn() {\n",
            "\t}\n",
            "\tstate = running\n",
            "\tpid = 42\n}\n",
        );
        assert_eq!(parse_print(output), State::Running { pid: Some(42) });
    }

    #[test]
    fn disabled_output_accepts_only_the_exact_quoted_label_and_true_value() {
        let output = concat!(
            "disabled services = {\n",
            "\t\t\"com.example.alpha\" => disabled\n",
            "\t\t\"com.example.beta\" => enabled\n",
            "\t\t\"com.example.gamma\" => true\n",
            "\t\t\"com.example.delta\" => false\n}",
        );
        assert!(parse_disabled(output, "com.example.alpha"));
        assert!(parse_disabled(output, "com.example.gamma"));
        assert!(!parse_disabled(output, "com.example.beta"));
        assert!(!parse_disabled(output, "com.example.delta"));
        for output in [
            "\"com.example.alpha.other\" => disabled",
            "com.example.alpha => disabled",
            "\"com.example.alpha\" = disabled",
            "\"com.example.alpha\" => disabled extra",
        ] {
            assert!(!parse_disabled(output, "com.example.alpha"));
        }
    }
}
