use crate::ManagerError;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(20);
const OUTPUT_LIMIT: usize = 1024 * 1024;

pub(crate) fn run(
    program: &Path,
    args: &[&str],
    verb: &'static str,
    timeout: Duration,
) -> Result<Output, ManagerError> {
    let mut child = Command::new(program)
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

pub(crate) fn failed(verb: &'static str, output: &Output) -> ManagerError {
    let stderr = String::from_utf8_lossy(&output.stderr)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(200)
        .collect();
    ManagerError::Failed {
        verb,
        code: output.status.code(),
        stderr,
    }
}

pub(crate) struct Output {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Vec<u8>,
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
    // A descendant can retain either pipe after the command exits.
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
