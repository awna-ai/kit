use awna_run::{Kill, Launch, Ran, Stderr, Stdin, Stdout, Tick, run};
use std::{
    fs, io,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("awna-run-{}-{name}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn script(&self, text: &str) -> Launch {
        shell(text).env("DIR", self.0.as_os_str())
    }
    fn pid(&self) -> i32 {
        fs::read_to_string(self.0.join("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Cleanup(i32);
impl Drop for Cleanup {
    fn drop(&mut self) {
        // SAFETY: This pid was read from our disposable child's output.
        unsafe {
            libc::kill(self.0, libc::SIGKILL);
        }
    }
}
fn alive(pid: i32) -> bool {
    // SAFETY: Signal zero only checks whether the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}
fn dead(pid: i32) {
    let end = Instant::now() + Duration::from_secs(10);
    while alive(pid) && Instant::now() < end {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(!alive(pid), "process {pid} is still present");
}
fn shell(text: &str) -> Launch {
    Launch::new("/bin/sh").args(["-c", text])
}
fn policy() -> Kill {
    Kill::after(Duration::from_secs(30))
}
fn call(launch: &Launch, input: Stdin, output: Stdout, error: Stderr, kill: Kill) -> Ran {
    run(launch, input, output, error, kill, None, None)
}
fn finished(result: Ran) -> (std::process::ExitStatus, Vec<u8>, bool, Vec<u8>, u64) {
    assert!(matches!(result, Ran::Finished { .. }), "{result:?}");
    match result {
        Ran::Finished {
            status,
            stdout,
            stdout_truncated,
            stderr,
            skipped_lines,
        } => (status, stdout, stdout_truncated, stderr, skipped_lines),
        _ => unreachable!(),
    }
}
fn lines(max_line: usize) -> (Stdout, Arc<Mutex<Vec<Vec<u8>>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let copy = Arc::clone(&seen);
    (
        Stdout::Lines {
            max_line,
            on_line: Box::new(move |line| copy.lock().unwrap().push(line.to_vec())),
        },
        seen,
    )
}

#[test]
fn a_spawn_failure_is_told_apart_from_a_timeout() {
    assert!(matches!(
        call(
            &Launch::new("/missing-spark-program"),
            Stdin::Null,
            Stdout::Null,
            Stderr::Null,
            policy()
        ),
        Ran::NotStarted(_)
    ));
    assert!(matches!(
        call(
            &shell("sleep 5"),
            Stdin::Null,
            Stdout::Null,
            Stderr::Null,
            Kill::after(Duration::from_millis(300))
        ),
        Ran::TimedOut
    ));
}
#[test]
fn a_timeout_ends_everything_the_child_started() {
    let f = Fixture::new("amber");
    let result = call(
        &f.script("sleep 300 & echo $! > \"$DIR/pid\"; wait"),
        Stdin::Null,
        Stdout::Null,
        Stderr::Null,
        Kill::after(Duration::from_secs(1)),
    );
    assert!(matches!(result, Ran::TimedOut), "{result:?}");
    dead(f.pid());
}
#[test]
fn a_stop_ends_the_group_within_the_grace() {
    let f = Fixture::new("briar");
    let flag = Arc::new(AtomicBool::new(false));
    let copy = Arc::clone(&flag);
    let setter = thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        copy.store(true, Ordering::Relaxed);
    });
    let start = Instant::now();
    let result = run(
        &f.script("sleep 300 & echo $! > \"$DIR/pid\"; wait"),
        Stdin::Null,
        Stdout::Null,
        Stderr::Null,
        Kill {
            term_grace: Some(Duration::from_secs(1)),
            ..policy()
        },
        Some(&flag),
        None,
    );
    setter.join().unwrap();
    assert!(matches!(result, Ran::Stopped), "{result:?}");
    assert!(start.elapsed() < Duration::from_secs(30));
    dead(f.pid());
}
#[test]
fn a_stop_already_set_starts_nothing() {
    let f = Fixture::new("cedar");
    let flag = AtomicBool::new(true);
    let result = run(
        &f.script("touch \"$DIR/ran\""),
        Stdin::File(f.0.join("missing")),
        Stdout::Null,
        Stderr::Null,
        policy(),
        Some(&flag),
        None,
    );
    assert!(matches!(result, Ran::Stopped));
    assert!(!f.0.join("ran").exists());
}
#[test]
fn a_term_signal_comes_before_the_kill_signal() {
    let f = Fixture::new("dapple");
    let result = call(
        &f.script("trap 'touch \"$DIR/term\"; exit 0' TERM; sleep 300 & wait"),
        Stdin::Null,
        Stdout::Null,
        Stderr::Null,
        Kill {
            timeout: Duration::from_millis(300),
            term_grace: Some(Duration::from_secs(5)),
            ..policy()
        },
    );
    assert!(matches!(result, Ran::TimedOut));
    assert!(f.0.join("term").exists());
}
#[test]
fn a_child_ignoring_term_is_killed_after_the_grace() {
    let start = Instant::now();
    let result = call(
        &shell("trap '' TERM; sleep 300 & wait"),
        Stdin::Null,
        Stdout::Null,
        Stderr::Null,
        Kill {
            timeout: Duration::from_millis(300),
            term_grace: Some(Duration::from_millis(500)),
            ..policy()
        },
    );
    assert!(matches!(result, Ran::TimedOut));
    assert!(start.elapsed() >= Duration::from_millis(800));
    assert!(start.elapsed() < Duration::from_secs(30));
}
#[test]
fn captured_stdout_stops_at_its_cap_without_blocking_the_child() {
    let (status, out, truncated, _, _) = finished(call(
        &shell("head -c 17000000 /dev/zero | tr '\\0' a; exit 0"),
        Stdin::Null,
        Stdout::Capture { cap: 1024 },
        Stderr::Null,
        policy(),
    ));
    assert!(status.success());
    assert_eq!(out, vec![b'a'; 1024]);
    assert!(truncated);
    let (_, out, truncated, _, _) = finished(call(
        &shell("printf short"),
        Stdin::Null,
        Stdout::Capture { cap: 1024 },
        Stderr::Null,
        policy(),
    ));
    assert_eq!(out, b"short");
    assert!(!truncated);
}
#[test]
fn stdin_bytes_reach_the_child_and_are_closed() {
    let (status, out, _, _, _) = finished(call(
        &Launch::new("/bin/cat"),
        Stdin::Bytes(b"hello\n".to_vec()),
        Stdout::Capture { cap: 1024 },
        Stderr::Null,
        policy(),
    ));
    assert!(status.success());
    assert_eq!(out, b"hello\n");
}
#[test]
fn a_child_that_never_reads_input_is_cut_at_the_timeout() {
    let start = Instant::now();
    let result = call(
        &shell("sleep 300"),
        Stdin::Bytes(vec![b'a'; 512 * 1024]),
        Stdout::Null,
        Stderr::Null,
        Kill::after(Duration::from_millis(300)),
    );
    assert!(matches!(result, Ran::TimedOut));
    assert!(start.elapsed() < Duration::from_secs(30));
}
#[test]
fn a_descendant_holding_stdin_does_not_hold_the_caller() {
    let f = Fixture::new("elm");
    let start = Instant::now();
    let (status, _, _, _, _) = finished(call(
        &f.script("sleep 300 & echo $! > \"$DIR/pid\"; exit 0"),
        Stdin::Bytes(vec![b'a'; 512 * 1024]),
        Stdout::Null,
        Stderr::Null,
        Kill::after(Duration::from_secs(120)),
    ));
    assert!(status.success());
    assert!(start.elapsed() < Duration::from_secs(30));
    dead(f.pid());
}
#[test]
fn a_child_that_exits_without_reading_still_answers() {
    let (status, _, _, err, _) = finished(call(
        &shell("echo refused >&2; exit 3"),
        Stdin::Bytes(vec![b'a'; 512 * 1024]),
        Stdout::Null,
        Stderr::Capture { cap: 1024 },
        policy(),
    ));
    assert_eq!(status.code(), Some(3));
    assert_eq!(err, b"refused\n");
}
#[test]
fn stderr_is_discarded_unless_asked_for() {
    let (_, out, _, err, _) = finished(call(
        &shell("echo out; echo err >&2"),
        Stdin::Null,
        Stdout::Capture { cap: 1024 },
        Stderr::Null,
        policy(),
    ));
    assert_eq!(out, b"out\n");
    assert!(err.is_empty());
}
#[test]
fn the_tail_keeps_only_the_end() {
    let (_, _, _, err, _) = finished(call(
        &shell("i=0; while [ $i -lt 10000 ]; do printf '%s\\n' \"$i\" >&2; i=$((i+1)); done"),
        Stdin::Null,
        Stdout::Null,
        Stderr::Tail(64),
        policy(),
    ));
    assert_eq!(
        err,
        b"987\n9988\n9989\n9990\n9991\n9992\n9993\n9994\n9995\n9996\n9997\n9998\n9999\n"
    );
}
#[test]
fn lines_arrive_in_order_and_overlong_lines_are_skipped() {
    let (output, seen) = lines(50);
    let (_, _, _, _, skipped) = finished(call(
        &shell(
            "printf 'one\\n'; i=0; while [ $i -lt 100 ]; do printf x; i=$((i+1)); done; printf '\\ntwo\\nthree'",
        ),
        Stdin::Null,
        output,
        Stderr::Null,
        policy(),
    ));
    assert_eq!(
        *seen.lock().unwrap(),
        [b"one".to_vec(), b"two".to_vec(), b"three".to_vec()]
    );
    assert_eq!(skipped, 1);
    let (output, seen) = lines(5);
    let (_, _, _, _, skipped) = finished(call(
        &shell("printf '12345\\n123456\\n12345'"),
        Stdin::Null,
        output,
        Stderr::Null,
        policy(),
    ));
    assert_eq!(
        *seen.lock().unwrap(),
        [b"12345".to_vec(), b"12345".to_vec()]
    );
    assert_eq!(skipped, 1);
}
#[test]
fn stderr_can_share_stdout() {
    let (_, out, _, err, _) = finished(call(
        &shell("echo a; echo b >&2; echo c"),
        Stdin::Null,
        Stdout::Capture { cap: 1024 },
        Stderr::SameAsStdout,
        policy(),
    ));
    assert_eq!(out, b"a\nb\nc\n");
    assert!(err.is_empty());
}
#[test]
fn append_logs_preserve_content_and_new_files_are_private() {
    let f = Fixture::new("fennel");
    let p = f.0.join("log");
    fs::write(&p, b"before\n").unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
    let (status, _, _, _, _) = finished(call(
        &shell("echo out; echo err >&2"),
        Stdin::Null,
        Stdout::AppendLog(p.clone()),
        Stderr::SameAsStdout,
        policy(),
    ));
    assert!(status.success());
    assert_eq!(fs::read(&p).unwrap(), b"before\nout\nerr\n");
    let p = f.0.join("new");
    finished(call(
        &shell("echo fresh"),
        Stdin::Null,
        Stdout::AppendLog(p.clone()),
        Stderr::SameAsStdout,
        policy(),
    ));
    assert_eq!(
        fs::metadata(&p).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::write(&p, b"old contents").unwrap();
    finished(call(
        &shell("printf new"),
        Stdin::Null,
        Stdout::File(p.clone()),
        Stderr::Null,
        policy(),
    ));
    assert_eq!(fs::read(p).unwrap(), b"new");
}
#[test]
fn file_destinations_refuse_links_and_real_input_files_feed_the_child() {
    let f = Fixture::new("garnet");
    let target = f.0.join("target");
    let link = f.0.join("link");
    fs::write(&target, b"original").unwrap();
    symlink(&target, &link).unwrap();
    for (input, output, error) in [
        (Stdin::Null, Stdout::AppendLog(link.clone()), Stderr::Null),
        (Stdin::Null, Stdout::File(link.clone()), Stderr::Null),
        (Stdin::Null, Stdout::Null, Stderr::AppendLog(link.clone())),
        (Stdin::File(link), Stdout::Null, Stderr::Null),
    ] {
        assert!(matches!(
            call(
                &shell("echo altered; echo error >&2"),
                input,
                output,
                error,
                policy()
            ),
            Ran::NotStarted(_)
        ));
        assert_eq!(fs::read(&target).unwrap(), b"original");
    }
    let (_, out, _, _, _) = finished(call(
        &Launch::new("/bin/cat"),
        Stdin::File(target),
        Stdout::Capture { cap: 1024 },
        Stderr::Null,
        policy(),
    ));
    assert_eq!(out, b"original");
}
#[test]
fn the_environment_is_exactly_the_one_given() {
    assert!(std::env::var_os("HOME").is_some());
    assert!(std::env::var_os("PATH").is_some());
    let launch = Launch::new("/usr/bin/env")
        .envs([("ALPHA_ONE", "1"), ("BETA_TWO", "plain")])
        .secret_env("BETA_TWO", Zeroizing::new("s3cret".into()));
    let (_, out, _, _, _) = finished(call(
        &launch,
        Stdin::Null,
        Stdout::Capture { cap: 4096 },
        Stderr::Null,
        policy(),
    ));
    let text = String::from_utf8(out).unwrap();
    assert!(text.lines().any(|s| s == "ALPHA_ONE=1"));
    assert!(text.lines().any(|s| s == "BETA_TWO=s3cret"));
    assert!(
        !text
            .lines()
            .any(|s| s.starts_with("HOME=") || s.starts_with("PATH=") || s == "BETA_TWO=plain")
    );
}
#[test]
fn an_invalid_launch_starts_nothing() {
    let f = Fixture::new("hazel");
    for launch in [
        f.script("touch \"$DIR/ran\"").env("A=B", "x"),
        f.script("touch \"$DIR/ran\"").env("", "x"),
        f.script("touch \"$DIR/ran\"").env("A", "x\0y"),
        f.script("touch \"$DIR/ran\"").env("A\0B", "x"),
        f.script("touch \"$DIR/ran\"")
            .secret_env("A=B", Zeroizing::new("x".into())),
        f.script("touch \"$DIR/ran\"")
            .secret_env("A", Zeroizing::new("x\0y".into())),
        Launch::new("/bin/sh\0oops"),
    ] {
        let result = call(&launch, Stdin::Null, Stdout::Null, Stderr::Null, policy());
        assert!(
            matches!(result, Ran::NotStarted(ref e) if e.kind() == io::ErrorKind::InvalidInput),
            "{result:?}"
        );
        assert!(!f.0.join("ran").exists());
    }
}
#[test]
fn debug_never_shows_a_secret() {
    let launch = shell("exit 0").secret_env("BETA_TWO", Zeroizing::new("s3cret".into()));
    let text = format!("{launch:?}");
    assert!(text.contains("BETA_TWO"));
    assert!(text.contains("<redacted>"));
    assert!(!text.contains("s3cret"));
}
#[test]
fn a_normal_exit_kills_a_background_pipe_holder() {
    let f = Fixture::new("iris");
    let start = Instant::now();
    let (status, out, _, _, _) = finished(call(
        &f.script("sleep 300 & echo $! > \"$DIR/pid\"; echo done"),
        Stdin::Null,
        Stdout::Capture { cap: 1024 },
        Stderr::Null,
        policy(),
    ));
    assert!(status.success());
    assert_eq!(out, b"done\n");
    assert!(start.elapsed() < Duration::from_secs(30));
    dead(f.pid());
}
#[test]
fn disabling_exit_kill_leaves_descendants_running() {
    let f = Fixture::new("juniper");
    let result = call(
        &f.script("sleep 300 & echo $! > \"$DIR/pid\""),
        Stdin::Null,
        Stdout::Null,
        Stderr::Null,
        Kill {
            kill_group_on_exit: false,
            ..policy()
        },
    );
    let guard = Cleanup(f.pid());
    let (status, _, _, _, _) = finished(result);
    assert!(status.success());
    assert!(alive(guard.0));
}
#[test]
fn exit_grace_allows_late_output_then_gives_up() {
    let (output, seen) = lines(50);
    let (status, _, _, _, _) = finished(call(
        &shell("(sleep 0.5; echo late) & echo early"),
        Stdin::Null,
        output,
        Stderr::Null,
        Kill {
            exit_grace: Duration::from_secs(3),
            ..policy()
        },
    ));
    assert!(status.success());
    assert_eq!(*seen.lock().unwrap(), [b"early".to_vec(), b"late".to_vec()]);
    let (output, seen) = lines(50);
    let start = Instant::now();
    finished(call(
        &shell("(sleep 0.5; echo late) & echo early"),
        Stdin::Null,
        output,
        Stderr::Null,
        Kill {
            exit_grace: Duration::ZERO,
            ..policy()
        },
    ));
    assert_eq!(*seen.lock().unwrap(), [b"early".to_vec()]);
    assert!(start.elapsed() < Duration::from_secs(30));
}
#[test]
fn the_tick_sees_the_child_pid_while_it_runs() {
    let f = Fixture::new("kelp");
    let mut seen = Vec::new();
    let mut callback = |pid| seen.push(pid);
    let (status, _, _, _, _) = finished(run(
        &f.script("echo $$ > \"$DIR/pid\"; exec sleep 1"),
        Stdin::Null,
        Stdout::Null,
        Stderr::Null,
        policy(),
        None,
        Some(Tick {
            every: Duration::from_millis(100),
            call: &mut callback,
        }),
    ));
    assert!(status.success());
    assert!(seen.len() >= 2);
    assert!(seen.iter().all(|pid| *pid == f.pid() as u32));
}

#[test]
fn a_leader_that_changes_group_does_not_hold_the_timeout() {
    let f = Fixture::new("larch");
    let path = f.0.join("pid");
    let (tx, rx) = std::sync::mpsc::channel();
    let start = Instant::now();
    thread::spawn(move || {
        let result = call(
            &Launch::new("/usr/bin/perl").args([
                "-e",
                "$| = 1; print \"$$\\n\"; setpgrp(0, getpgrp(getppid())) or die $!; sleep 300",
            ]),
            Stdin::Null,
            Stdout::AppendLog(path),
            Stderr::Null,
            Kill::after(Duration::from_millis(500)),
        );
        let _ = tx.send(result);
    });
    let end = Instant::now() + Duration::from_secs(5);
    while !fs::read_to_string(f.0.join("pid")).is_ok_and(|s| !s.trim().is_empty())
        && Instant::now() < end
    {
        thread::sleep(Duration::from_millis(20));
    }
    let guard = Cleanup(f.pid());
    let result = rx.recv_timeout(Duration::from_secs(5));
    assert!(result.is_ok(), "timeout cleanup did not return: {result:?}");
    assert!(matches!(result.unwrap(), Ran::TimedOut | Ran::Unfinished));
    assert!(start.elapsed() < Duration::from_secs(10));
    dead(guard.0);
}

fn fifo_is_refused(destination: usize) {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let f = Fixture::new(&format!("maple-{destination}"));
    let path = f.0.join("pipe");
    let name = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: name is a valid NUL-terminated path in our disposable directory.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let (input, output, error) = match destination {
            0 => (Stdin::File(path), Stdout::Null, Stderr::Null),
            1 => (Stdin::Null, Stdout::AppendLog(path), Stderr::Null),
            2 => (Stdin::Null, Stdout::File(path), Stderr::Null),
            3 => (Stdin::Null, Stdout::Null, Stderr::AppendLog(path)),
            _ => unreachable!(),
        };
        let _ = tx.send(call(&shell("exit 0"), input, output, error, policy()));
    });
    let result = rx.recv_timeout(Duration::from_secs(5));
    assert!(matches!(result, Ok(Ran::NotStarted(_))), "{result:?}");
}
#[test]
fn stdin_fifo_is_refused_without_blocking() {
    fifo_is_refused(0);
}
#[test]
fn stdout_log_fifo_is_refused_without_blocking() {
    fifo_is_refused(1);
}
#[test]
fn stdout_file_fifo_is_refused_without_blocking() {
    fifo_is_refused(2);
}
#[test]
fn stderr_log_fifo_is_refused_without_blocking() {
    fifo_is_refused(3);
}

#[test]
fn stdout_devices_are_refused() {
    let result = call(
        &shell("exit 0"),
        Stdin::Null,
        Stdout::File("/dev/null".into()),
        Stderr::Null,
        policy(),
    );
    assert!(
        matches!(result, Ran::NotStarted(ref e) if e.kind() == io::ErrorKind::InvalidInput),
        "{result:?}"
    );
}
#[test]
fn stdin_devices_are_refused() {
    let result = call(
        &shell("exit 0"),
        Stdin::File("/dev/null".into()),
        Stdout::Null,
        Stderr::Null,
        policy(),
    );
    assert!(
        matches!(result, Ran::NotStarted(ref e) if e.kind() == io::ErrorKind::InvalidInput),
        "{result:?}"
    );
}
#[test]
fn public_logs_are_refused_without_changes() {
    let f = Fixture::new("nettle");
    let path = f.0.join("log");
    fs::write(&path, b"precious").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let result = call(
        &shell("echo altered"),
        Stdin::Null,
        Stdout::AppendLog(path.clone()),
        Stderr::Null,
        policy(),
    );
    assert!(
        matches!(result, Ran::NotStarted(ref e) if e.kind() == io::ErrorKind::PermissionDenied),
        "{result:?}"
    );
    assert_eq!(fs::read(path).unwrap(), b"precious");
}
#[test]
fn hard_linked_logs_are_refused() {
    let f = Fixture::new("olive");
    let path = f.0.join("log");
    fs::write(&path, b"precious").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::hard_link(&path, f.0.join("alias")).unwrap();
    let result = call(
        &shell("echo altered"),
        Stdin::Null,
        Stdout::AppendLog(path.clone()),
        Stderr::Null,
        policy(),
    );
    assert!(
        matches!(result, Ran::NotStarted(ref e) if e.kind() == io::ErrorKind::PermissionDenied),
        "{result:?}"
    );
    assert_eq!(fs::read(path).unwrap(), b"precious");
}
fn precious_output(name: &str, launch: Launch, missing_error: bool) {
    let f = Fixture::new(name);
    let path = f.0.join("out");
    fs::write(&path, b"precious").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let error = if missing_error {
        Stderr::AppendLog(f.0.join("missing/log"))
    } else {
        Stderr::Null
    };
    let result = call(
        &launch,
        Stdin::Null,
        Stdout::File(path.clone()),
        error,
        policy(),
    );
    assert!(matches!(result, Ran::NotStarted(_)), "{result:?}");
    assert_eq!(fs::read(path).unwrap(), b"precious");
}
#[test]
fn a_later_file_error_preserves_output() {
    precious_output("pine", shell("exit 0"), true);
}
#[test]
fn a_nul_argument_preserves_output() {
    precious_output("quince", shell("exit 0").arg("a\0b"), false);
}
#[test]
fn a_nul_directory_preserves_output() {
    precious_output("reed", shell("exit 0").cwd("a\0b"), false);
}

#[test]
fn stdin_debug_hides_bytes() {
    let text = format!("{:?}", Stdin::Bytes(b"tok".to_vec()));
    assert!(!text.contains("116") && !text.contains("tok"), "{text}");
    assert_eq!(text, "Bytes { len: 3 }");
}
#[test]
fn finished_debug_hides_bytes() {
    use std::os::unix::process::ExitStatusExt;
    let result = Ran::Finished {
        status: std::process::ExitStatus::from_raw(0),
        stdout: b"tok".to_vec(),
        stdout_truncated: true,
        stderr: b"tok".to_vec(),
        skipped_lines: 2,
    };
    let text = format!("{result:?}");
    assert!(!text.contains("116") && !text.contains("tok"), "{text}");
    for field in [
        "status",
        "stdout_len: 3",
        "stdout_truncated: true",
        "stderr_len: 3",
        "skipped_lines: 2",
    ] {
        assert!(text.contains(field), "{text}");
    }
}

#[test]
fn the_working_directory_is_used() {
    let f = Fixture::new("spruce");
    let (_, out, _, _, _) = finished(call(
        &Launch::new("/bin/pwd").arg("-P").cwd(&f.0),
        Stdin::Null,
        Stdout::Capture { cap: 4096 },
        Stderr::Null,
        policy(),
    ));
    use std::os::unix::ffi::OsStrExt;
    let mut expected = f.0.canonicalize().unwrap().as_os_str().as_bytes().to_vec();
    expected.push(b'\n');
    assert_eq!(out, expected);
}

#[test]
fn an_open_descendant_pipe_returns_unfinished() {
    let f = Fixture::new("thyme");
    let start = Instant::now();
    let result = call(
        &f.script("sleep 300 & echo $! > \"$DIR/pid\"; echo early"),
        Stdin::Null,
        Stdout::Capture { cap: 1024 },
        Stderr::Null,
        Kill {
            kill_group_on_exit: false,
            exit_grace: Duration::ZERO,
            ..policy()
        },
    );
    let guard = Cleanup(f.pid());
    assert!(matches!(result, Ran::Unfinished), "{result:?}");
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(alive(guard.0));
}
