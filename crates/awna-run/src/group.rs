use std::{
    io,
    process::Child,
    thread,
    time::{Duration, Instant},
};

const KILL_WAIT: Duration = Duration::from_secs(1);

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
) -> bool {
    let pid = child.id();
    if let Some(grace) = grace {
        signal(pid, libc::SIGTERM);
        let start = Instant::now();
        loop {
            match has_exited(pid) {
                Some(true) => break,
                None => match child.try_wait() {
                    Ok(Some(_)) => return true,
                    Ok(None) => {}
                    Err(_) => return false,
                },
                Some(false) => {}
            }
            if start.elapsed() >= grace {
                break;
            }
            thread::sleep(POLL.min(grace.saturating_sub(start.elapsed())));
        }
    }
    if !signal(pid, libc::SIGKILL) || has_exited(pid) != Some(true) {
        let _ = child.kill();
    }
    let start = Instant::now();
    loop {
        match has_exited(pid) {
            Some(true) => return child.wait().is_ok(),
            None => match child.try_wait() {
                Ok(Some(_)) => return true,
                Ok(None) => {}
                Err(_) => return false,
            },
            Some(false) => {}
        }
        if start.elapsed() >= KILL_WAIT {
            return false;
        }
        thread::sleep(POLL.min(KILL_WAIT.saturating_sub(start.elapsed())));
    }
}
