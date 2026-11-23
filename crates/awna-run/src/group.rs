use std::{
    io,
    process::Child,
    thread,
    time::{Duration, Instant},
};

pub(super) const POLL: Duration = Duration::from_millis(20);

pub(super) fn signal_group(pid: u32, signal: libc::c_int) -> bool {
    let Ok(group) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: getpgrp has no arguments or preconditions.
    if group <= 1 || group == unsafe { libc::getpgrp() } {
        return false;
    }
    // SAFETY: The guarded identifier addresses only the child's process group.
    unsafe { libc::killpg(group, signal) == 0 }
}

pub(super) fn has_exited(pid: u32) -> Option<bool> {
    loop {
        // SAFETY: A zeroed siginfo_t is valid storage for waitid's result.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: The pointer is valid and these flags observe without reaping.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
            )
        };
        if result == 0 {
            // SAFETY: A successful waitid initialized info, including si_pid.
            return Some(unsafe { info.si_pid() } != 0);
        }
        if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return None;
        }
    }
}

pub(super) fn end_group(
    child: &mut Child,
    grace: Option<Duration>,
    signal: impl Fn(u32, libc::c_int) -> bool,
) {
    let pid = child.id();
    if let Some(grace) = grace {
        signal(pid, libc::SIGTERM);
        let start = Instant::now();
        while has_exited(pid) != Some(true) && start.elapsed() < grace {
            thread::sleep(POLL.min(grace.saturating_sub(start.elapsed())));
        }
    }
    signal(pid, libc::SIGKILL);
    let _ = child.wait();
}
