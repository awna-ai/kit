//! Runs one child in its own process group with an environment rebuilt from the caller's
//! pairs, bounded output, and timeout and stop policies. Group signals are sent before the
//! leader is reaped, so its group identifier cannot be reused before cleanup.
//!
//! ```
//! use awna_run::{run, Kill, Launch, Ran, Stderr, Stdin, Stdout};
//! use std::time::Duration;
//! let result = run(&Launch::new("/bin/sh").args(["-c", "echo hi"]),
//!     Stdin::Null, Stdout::Capture { cap: 1024 }, Stderr::Null,
//!     Kill::after(Duration::from_secs(5)), None, None);
//! match result {
//!     Ran::Finished { status, stdout, .. } => {
//!         assert!(status.success());
//!         assert_eq!(stdout, b"hi\n");
//!     }
//!     other => panic!("unexpected result: {other:?}"),
//! }
//! ```
//!
//! # Limits
//!
//! A descendant that leaves the group with `setsid` or `setpgid` is not signalled.
//! A [`Stdout::Lines`] callback runs on a reader thread and can still run after [`run`]
//! returns, until whoever holds the pipe closes it. Callbacks should return promptly.
//!
//! The host must not ignore SIGCHLD or reap children elsewhere (for example with
//! `waitpid(-1)`). If it does, runs end [`Ran::Unfinished`] and descendants may keep running.
//!
//! Only file descriptors opened close-on-exec stay out of the child. Rust opens its own
//! descriptors that way, but a descriptor other host code opens without `O_CLOEXEC` is inherited.
//! The child stays in the caller's session, so it can open the caller's controlling terminal
//! if there is one.
//!
//! `O_NOFOLLOW` guards only the last path component; a link in a parent directory is followed.
//! Input files must be regular files. Output files must also belong to the effective user,
//! have exactly one hard link, and grant no permissions to the group or others. Existing
//! files that fail these checks are refused without changing their contents or permissions.
//!
//! `Zeroizing` wipes [`Launch`]'s copies of a secret. The standard library's command builder
//! and the operating system's environment copy are not wiped.
//!
//! A program without a `/` is searched for using the operating system's default search path,
//! not the child's `PATH`. Use [`find_program`] and pass an absolute path.

#![forbid(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

#[cfg(not(unix))]
compile_error!("awna-run supports Unix only");

mod group;
mod output;
mod supervisor;

use std::{
    ffi::{OsStr, OsString},
    fmt, io,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::atomic::AtomicBool,
    time::Duration,
};
use zeroize::Zeroizing;

/// What to run. The child's environment is exactly `env` plus `secret_env`, nothing else.
#[derive(Clone)]
pub struct Launch {
    program: PathBuf,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    secret_env: Vec<(&'static str, Zeroizing<String>)>,
    cwd: Option<PathBuf>,
}

impl Launch {
    /// Starts a description of the program to run.
    pub fn new(program: impl Into<PathBuf>) -> Launch {
        Launch {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            secret_env: Vec::new(),
            cwd: None,
        }
    }
    /// Appends one argument, without shell interpretation.
    pub fn arg(mut self, arg: impl Into<OsString>) -> Launch {
        self.args.push(arg.into());
        self
    }
    /// Appends arguments, without shell interpretation.
    pub fn args<I, S>(mut self, args: I) -> Launch
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }
    /// Sets one plain environment variable.
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Launch {
        self.env.push((key.into(), value.into()));
        self
    }
    /// Sets plain environment variables in iteration order.
    pub fn envs<I, K, V>(mut self, vars: I) -> Launch
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        self.env
            .extend(vars.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }
    /// Keeps a secret in zeroizing storage and redacts its value in `Debug`.
    /// Secret variables are set after plain variables, so a repeated secret key wins.
    pub fn secret_env(mut self, key: &'static str, value: Zeroizing<String>) -> Launch {
        self.secret_env.push((key, value));
        self
    }
    /// The child's working directory; unset, it inherits the caller's.
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Launch {
        self.cwd = Some(dir.into());
        self
    }
    /// The program path supplied to this description.
    pub fn program(&self) -> &Path {
        &self.program
    }

    fn validate(&self) -> io::Result<()> {
        let valid_pair = |key: &OsStr, value: &OsStr| {
            let key = key.as_bytes();
            !key.is_empty()
                && !key.contains(&b'=')
                && !key.contains(&0)
                && !value.as_bytes().contains(&0)
        };
        if self.program.as_os_str().as_bytes().contains(&0)
            || self.args.iter().any(|arg| arg.as_bytes().contains(&0))
            || self
                .cwd
                .as_ref()
                .is_some_and(|cwd| cwd.as_os_str().as_bytes().contains(&0))
            || self.env.iter().any(|(k, v)| !valid_pair(k, v))
            || self
                .secret_env
                .iter()
                .any(|(k, v)| !valid_pair(OsStr::new(k), OsStr::new(v.as_str())))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid program, argument, directory or environment",
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for Launch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secrets: Vec<_> = self
            .secret_env
            .iter()
            .map(|(k, _)| (*k, "<redacted>"))
            .collect();
        f.debug_struct("Launch")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("env", &self.env)
            .field("secret_env", &secrets)
            .field("cwd", &self.cwd)
            .finish()
    }
}

/// The child's stdin.
pub enum Stdin {
    /// Closed through `/dev/null`.
    Null,
    /// Written on a detached thread, then closed. Write failures leave the exit status decisive.
    Bytes(Vec<u8>),
    /// A regular file opened read-only without following a final symlink.
    File(PathBuf),
}

impl fmt::Debug for Stdin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("Null"),
            Self::Bytes(bytes) => f.debug_struct("Bytes").field("len", &bytes.len()).finish(),
            Self::File(path) => f.debug_tuple("File").field(path).finish(),
        }
    }
}

/// The child's stdout.
pub enum Stdout {
    /// Discarded through `/dev/null`.
    Null,
    /// Keeps the first `cap` bytes and drains the rest without retaining them.
    Capture {
        /// Maximum retained bytes.
        cap: u64,
    },
    /// Delivers lines on a reader thread, without `\n`; skips oversized lines.
    Lines {
        /// Maximum line length in bytes, excluding `\n`.
        max_line: usize,
        /// Receives each kept line, including a final unterminated line.
        #[allow(clippy::type_complexity)]
        on_line: Box<dyn FnMut(&[u8]) + Send + 'static>,
    },
    /// Appends to a private, caller-owned regular file with one link; new files have mode 0600.
    /// Refuses a final symlink.
    /// A missing output file is created before later files are checked, so a refused run can leave a new empty file behind.
    AppendLog(PathBuf),
    /// Truncates after all files pass validation; a later spawn failure still leaves it empty.
    /// Requires a private, caller-owned regular file with one link and refuses a final symlink.
    /// New files have mode 0600.
    /// A missing output file is created before later files are checked, so a refused run can leave a new empty file behind.
    File(PathBuf),
}

impl fmt::Debug for Stdout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("Null"),
            Self::Capture { cap } => f.debug_struct("Capture").field("cap", cap).finish(),
            Self::Lines { max_line, .. } => f
                .debug_struct("Lines")
                .field("max_line", max_line)
                .field("on_line", &format_args!(".."))
                .finish(),
            Self::AppendLog(p) => f.debug_tuple("AppendLog").field(p).finish(),
            Self::File(p) => f.debug_tuple("File").field(p).finish(),
        }
    }
}

/// The child's stderr.
#[derive(Debug)]
pub enum Stderr {
    /// Discarded. Useful when stderr may echo sensitive input.
    Null,
    /// Keeps the first `cap` bytes and drains the rest.
    Capture {
        /// Maximum retained bytes.
        cap: u64,
    },
    /// Keeps only the last `n` bytes.
    Tail(usize),
    /// Shares stdout's pipe, file or `/dev/null` so writes interleave.
    SameAsStdout,
    /// Appends to a private, caller-owned regular file with one link; new files have mode 0600.
    /// Refuses a final symlink.
    /// A missing output file is created before later files are checked, so a refused run can leave a new empty file behind.
    AppendLog(PathBuf),
}

/// How the child is bounded and ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kill {
    /// Time from the start until the group is ended as [`Ran::TimedOut`].
    pub timeout: Duration,
    /// Sends SIGTERM, waits up to this grace for the leader, then sends SIGKILL
    /// unless the child has already been reaped.
    /// `None` sends SIGKILL immediately.
    pub term_grace: Option<Duration>,
    /// How long piped readers may finish after the leader exits, before group cleanup.
    /// This grace may extend past `timeout`; zero means no wait.
    pub exit_grace: Duration,
    /// Sends SIGKILL after the exit grace, before reaping the leader.
    /// `false` leaves descendants running.
    pub kill_group_on_exit: bool,
}
impl Kill {
    /// Uses an immediate SIGKILL, no exit grace, and group cleanup on normal exit.
    pub fn after(timeout: Duration) -> Kill {
        Kill {
            timeout,
            term_grace: None,
            exit_grace: Duration::ZERO,
            kill_group_on_exit: true,
        }
    }
}

/// Called immediately with the child's pid, then periodically on the thread calling [`run`].
pub struct Tick<'a> {
    /// Callback interval; values below 20 ms count as 20 ms.
    pub every: Duration,
    /// Receives the child's pid while its leader is running.
    pub call: &'a mut dyn FnMut(u32),
}
impl fmt::Debug for Tick<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tick").field("every", &self.every).finish()
    }
}

/// How a run ended.
pub enum Ran {
    /// The child exited and every output reader reported successfully.
    Finished {
        /// The leader's exit status.
        status: ExitStatus,
        /// Captured stdout bytes; empty for other stdout policies.
        stdout: Vec<u8>,
        /// Whether stdout capture discarded bytes beyond its cap.
        stdout_truncated: bool,
        /// Captured stderr or its retained tail; empty for other stderr policies.
        stderr: Vec<u8>,
        /// Number of oversized lines skipped by the stdout reader.
        skipped_lines: u64,
    },
    /// The timeout passed and the group was ended.
    TimedOut,
    /// The stop flag was set. An already-set flag starts nothing.
    Stopped,
    /// The child started, but waiting or collecting output failed. No output is returned.
    /// Also returned when timeout or stop cleanup cannot reap the leader within one second
    /// after SIGKILL. Output collection allows at most one second after the leader is reaped.
    Unfinished,
    /// Nothing started because validation, file opening or spawning failed.
    NotStarted(io::Error),
}

impl fmt::Debug for Ran {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Finished {
                status,
                stdout,
                stdout_truncated,
                stderr,
                skipped_lines,
            } => f
                .debug_struct("Finished")
                .field("status", status)
                .field("stdout_len", &stdout.len())
                .field("stdout_truncated", stdout_truncated)
                .field("stderr_len", &stderr.len())
                .field("skipped_lines", skipped_lines)
                .finish(),
            Self::TimedOut => f.write_str("TimedOut"),
            Self::Stopped => f.write_str("Stopped"),
            Self::Unfinished => f.write_str("Unfinished"),
            Self::NotStarted(error) => f.debug_tuple("NotStarted").field(error).finish(),
        }
    }
}

/// Runs one child until it exits, is stopped, or reaches its timeout.
pub fn run(
    launch: &Launch,
    stdin: Stdin,
    stdout: Stdout,
    stderr: Stderr,
    kill: Kill,
    stop: Option<&AtomicBool>,
    tick: Option<Tick<'_>>,
) -> Ran {
    supervisor::run_with_signal(
        launch,
        stdin,
        stdout,
        stderr,
        kill,
        stop,
        tick,
        group::signal_group,
    )
}

/// Finds an executable regular file, following links. A name containing `/` must be absolute.
/// Bare names search only absolute, nonempty `path` folders, then full paths in `fallbacks`.
pub fn find_program(name: &str, path: Option<&OsStr>, fallbacks: &[PathBuf]) -> Option<PathBuf> {
    fn runnable(p: &Path) -> bool {
        p.is_absolute()
            && p.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    if name.contains('/') {
        return runnable(Path::new(name)).then(|| PathBuf::from(name));
    }
    if let Some(path) = path {
        for folder in std::env::split_paths(path).filter(|p| p.is_absolute()) {
            let candidate = folder.join(name);
            if runnable(&candidate) {
                return Some(candidate);
            }
        }
    }
    fallbacks.iter().find(|p| runnable(p)).cloned()
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::group::signal_group;
    use std::cell::RefCell;
    #[derive(Debug, PartialEq, Eq)]
    enum Reap {
        Running,
        Unreaped,
        Gone,
    }
    fn reap_state(pid: u32) -> Reap {
        loop {
            // SAFETY: An all-zero siginfo_t is valid storage for waitid to fill.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            // SAFETY: The pointer is valid and WNOWAIT never reaps the child.
            let rc = unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
                )
            };
            if rc != 0 {
                if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Reap::Gone;
            }
            // SAFETY: A successful waitid initialized the signal information.
            if unsafe { info.si_pid() } == 0 {
                return Reap::Running;
            } else {
                return Reap::Unreaped;
            }
        }
    }
    #[test]
    fn group_signals_are_refused_for_unsafe_ids() {
        // SAFETY: getpgrp takes no arguments and has no preconditions.
        let own = unsafe { libc::getpgrp() } as u32;
        for pid in [0, 1, u32::MAX, own] {
            assert!(!signal_group(pid, libc::SIGKILL));
        }
    }
    #[test]
    fn the_group_is_signalled_before_the_leader_is_reaped() {
        for (script, kill, expected, timed_out) in [
            (
                "exit 0",
                Kill::after(Duration::from_secs(5)),
                vec![libc::SIGKILL],
                false,
            ),
            (
                "sleep 300",
                Kill::after(Duration::from_millis(300)),
                vec![libc::SIGKILL],
                true,
            ),
            (
                "trap 'exit 0' TERM; sleep 300 & wait",
                Kill {
                    timeout: Duration::from_millis(300),
                    term_grace: Some(Duration::from_secs(1)),
                    ..Kill::after(Duration::from_secs(5))
                },
                vec![libc::SIGTERM, libc::SIGKILL],
                true,
            ),
        ] {
            let records = RefCell::new(Vec::new());
            let result = supervisor::run_with_signal(
                &Launch::new("/bin/sh").args(["-c", script]),
                Stdin::Null,
                Stdout::Null,
                Stderr::Null,
                kill,
                None,
                None,
                |pid, signal| {
                    records.borrow_mut().push((pid, signal, reap_state(pid)));
                    signal_group(pid, signal)
                },
            );
            let records = records.into_inner();
            assert_eq!(records.iter().map(|r| r.1).collect::<Vec<_>>(), expected);
            if timed_out {
                assert!(matches!(result, Ran::TimedOut), "{result:?}");
            } else {
                assert!(
                    matches!(result, Ran::Finished { status, .. } if status.success()),
                    "{result:?}"
                );
            }
            for (pid, _, state) in records {
                assert_ne!(state, Reap::Gone);
                assert_eq!(reap_state(pid), Reap::Gone);
            }
        }
    }
    fn isolated_test(name: &str, variable: &str) {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture", "--test-threads=1"])
            .env(variable, "1")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let start = std::time::Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "isolated test failed: {status}");
                let output = child.wait_with_output().unwrap();
                let stdout = String::from_utf8(output.stdout).unwrap();
                assert!(stdout.contains("1 passed"), "{stdout}");
                assert!(stdout.contains("isolated body ran"), "{stdout}");
                return;
            }
            if start.elapsed() > Duration::from_secs(10) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("isolated test did not return");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_failed_wait_never_leads_to_a_group_signal() {
        isolated_test("tests::sigchld_ignored_child", "AWNA_RUN_TEST_SIGCHLD");
    }

    #[test]
    fn sigchld_ignored_child() {
        if std::env::var("AWNA_RUN_TEST_SIGCHLD").as_deref() != Ok("1") {
            return;
        }
        println!("isolated body ran");
        // SAFETY: This isolated process runs no other tests and exits after this test.
        unsafe { libc::signal(libc::SIGCHLD, libc::SIG_IGN) };
        let records = RefCell::new(Vec::new());
        let start = std::time::Instant::now();
        let result = supervisor::run_with_signal(
            &Launch::new("/bin/sh").args(["-c", "trap 'exit 0' TERM; sleep 300 & wait"]),
            Stdin::Null,
            Stdout::Null,
            Stderr::Null,
            Kill {
                term_grace: Some(Duration::from_secs(2)),
                ..Kill::after(Duration::from_millis(300))
            },
            None,
            None,
            |pid, signal| {
                records.borrow_mut().push(reap_state(pid));
                signal_group(pid, signal)
            },
        );
        assert!(!records.borrow().is_empty());
        assert!(
            records.borrow().iter().all(|state| *state != Reap::Gone),
            "{:?}",
            records.borrow()
        );
        assert!(start.elapsed() < Duration::from_secs(5));
        assert!(matches!(result, Ran::Unfinished), "{result:?}");
    }

    #[test]
    fn a_closed_stdin_pipe_does_not_kill_the_host() {
        isolated_test("tests::sigpipe_default_child", "AWNA_RUN_TEST_SIGPIPE");
    }

    #[test]
    fn sigpipe_default_child() {
        if std::env::var("AWNA_RUN_TEST_SIGPIPE").as_deref() != Ok("1") {
            return;
        }
        println!("isolated body ran");
        // SAFETY: Only this isolated test process changes its signal disposition.
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
        let result = run(
            &Launch::new("/bin/sh").args(["-c", "exit 0"]),
            Stdin::Bytes(vec![b'x'; 4 * 1024 * 1024]),
            Stdout::Null,
            Stderr::Null,
            Kill::after(Duration::from_secs(5)),
            None,
            None,
        );
        std::thread::sleep(Duration::from_millis(500));
        assert!(matches!(result, Ran::Finished { status, .. } if status.success()));
    }
}
