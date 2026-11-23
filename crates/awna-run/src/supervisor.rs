use crate::{
    Kill, Launch, Ran, Stderr, Stdin, Stdout, Tick,
    group::{self, POLL},
    output::{Prepared, Reader, Readers},
};
use std::{
    io::Write,
    os::unix::process::CommandExt,
    process::{Child, Command, ExitStatus},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

const COLLECT_GRACE: Duration = Duration::from_secs(1);

fn stopped(stop: Option<&AtomicBool>) -> bool {
    stop.is_some_and(|s| s.load(Ordering::Relaxed))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_with_signal(
    launch: &Launch,
    stdin: Stdin,
    stdout: Stdout,
    stderr: Stderr,
    kill: Kill,
    stop: Option<&AtomicBool>,
    mut tick: Option<Tick<'_>>,
    signal: impl Fn(u32, libc::c_int) -> bool,
) -> Ran {
    if stopped(stop) {
        return Ran::Stopped;
    }
    if let Err(error) = launch.validate() {
        return Ran::NotStarted(error);
    }
    let prepared = match Prepared::open(stdin, stdout, stderr) {
        Ok(prepared) => prepared,
        Err(error) => return Ran::NotStarted(error),
    };
    let Prepared {
        stdin,
        stdout,
        stderr,
        bytes,
        out_mode,
        err_mode,
        shared,
    } = prepared;
    let mut command = Command::new(&launch.program);
    command
        .args(&launch.args)
        .env_clear()
        .envs(launch.env.iter().map(|(k, v)| (k, v)))
        .envs(launch.secret_env.iter().map(|(k, v)| (k, v.as_str())))
        .process_group(0)
        .stdin(stdin)
        .stdout(stdout)
        .stderr(stderr);
    if let Some(cwd) = &launch.cwd {
        command.current_dir(cwd);
    }
    let start = Instant::now();
    let spawned = command.spawn();
    // Command retains explicit pipe writers, which must close before waiting for EOF.
    drop(command);
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => return Ran::NotStarted(error),
    };
    let pid = child.id();
    if let Some(tick) = &mut tick {
        (tick.call)(pid);
    }
    let mut last_tick = Instant::now();
    let mut readers = Readers::default();
    if let Some(mode) = out_mode {
        readers.out = Some(if let Some(shared) = shared {
            Reader::start(shared, mode)
        } else {
            Reader::start(child.stdout.take().expect("piped stdout"), mode)
        });
    }
    if let Some(mode) = err_mode {
        readers.err = Some(Reader::start(
            child.stderr.take().expect("piped stderr"),
            mode,
        ));
    }
    if let Some(bytes) = bytes {
        let mut pipe = child.stdin.take().expect("piped stdin");
        thread::spawn(move || {
            let _ = pipe.write_all(&bytes);
        });
    }
    loop {
        if stopped(stop) {
            group::end_group(&mut child, kill.term_grace, &signal);
            return Ran::Stopped;
        }
        match group::has_exited(pid) {
            Some(true) => return exited(&mut child, readers, kill, stop, signal),
            None => match child.try_wait() {
                Ok(Some(status)) => return collect(readers, status),
                Ok(None) => {}
                Err(_) => return Ran::Unfinished,
            },
            Some(false) => {}
        }
        if start.elapsed() >= kill.timeout {
            group::end_group(&mut child, kill.term_grace, &signal);
            return Ran::TimedOut;
        }
        if let Some(tick) = &mut tick
            && last_tick.elapsed() >= tick.every.max(POLL)
        {
            (tick.call)(pid);
            last_tick = Instant::now();
        }
        thread::sleep(POLL.min(kill.timeout.saturating_sub(start.elapsed())));
    }
}

fn exited(
    child: &mut Child,
    mut readers: Readers,
    kill: Kill,
    stop: Option<&AtomicBool>,
    signal: impl Fn(u32, libc::c_int) -> bool,
) -> Ran {
    let start = Instant::now();
    while start.elapsed() < kill.exit_grace && !readers.ready() && !stopped(stop) {
        thread::sleep(POLL.min(kill.exit_grace.saturating_sub(start.elapsed())));
    }
    if kill.kill_group_on_exit {
        signal(child.id(), libc::SIGKILL);
    }
    match child.wait() {
        Ok(status) => collect(readers, status),
        Err(_) => Ran::Unfinished,
    }
}

fn collect(readers: Readers, status: ExitStatus) -> Ran {
    match readers.collect(COLLECT_GRACE) {
        Some((out, err)) => Ran::Finished {
            status,
            stdout: out.bytes,
            stdout_truncated: out.truncated,
            stderr: err.bytes,
            skipped_lines: out.skipped,
        },
        None => Ran::Unfinished,
    }
}
