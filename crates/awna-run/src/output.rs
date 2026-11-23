use crate::{Stderr, Stdin, Stdout};
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Read},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
    process::Stdio,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::{Duration, Instant},
};

type OnLine = Box<dyn FnMut(&[u8]) + Send + 'static>;
pub(super) enum Mode {
    Capture(u64),
    Lines(usize, OnLine),
    Tail(usize),
}
#[derive(Default)]
pub(super) struct Output {
    pub bytes: Vec<u8>,
    pub truncated: bool,
    pub skipped: u64,
}

pub(super) struct Prepared {
    pub stdin: Stdio,
    pub stdout: Stdio,
    pub stderr: Stdio,
    pub bytes: Option<Vec<u8>>,
    pub out_mode: Option<Mode>,
    pub err_mode: Option<Mode>,
    pub shared: Option<io::PipeReader>,
}

const FILE_FLAGS: libc::c_int = libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY;

fn checked(file: File, output: bool) -> io::Result<File> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a regular file",
        ));
    }
    // SAFETY: geteuid has no arguments or preconditions.
    if output
        && (metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "output must be private, owned by the caller, and have one link",
        ));
    }
    // SAFETY: file owns a valid descriptor; F_GETFL takes no further argument.
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: The descriptor is valid and these are its existing flags minus O_NONBLOCK.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags & !libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(file)
}

fn log(path: &Path, append: bool) -> io::Result<File> {
    checked(
        OpenOptions::new()
            .write(true)
            .create(true)
            .append(append)
            .mode(0o600)
            .custom_flags(FILE_FLAGS)
            .open(path)?,
        true,
    )
}

impl Prepared {
    pub fn open(input: Stdin, output: Stdout, error: Stderr) -> io::Result<Self> {
        let (stdin, bytes) = match input {
            Stdin::Null => (Stdio::null(), None),
            Stdin::Bytes(bytes) => (Stdio::piped(), Some(bytes)),
            Stdin::File(path) => {
                let file = checked(
                    OpenOptions::new()
                        .read(true)
                        .custom_flags(FILE_FLAGS)
                        .open(path)?,
                    false,
                )?;
                (file.into(), None)
            }
        };
        let truncate = matches!(output, Stdout::File(_));
        let (out_mode, file) = match output {
            Stdout::Null => (None, None),
            Stdout::Capture { cap } => (Some(Mode::Capture(cap)), None),
            Stdout::Lines { max_line, on_line } => (Some(Mode::Lines(max_line, on_line)), None),
            Stdout::AppendLog(path) => (None, Some(log(&path, true)?)),
            Stdout::File(path) => (None, Some(log(&path, false)?)),
        };
        let mut shared = None;
        let mut shared_writer = None;
        let (stderr, err_mode) = match error {
            Stderr::Null => (Stdio::null(), None),
            Stderr::Capture { cap } => (Stdio::piped(), Some(Mode::Capture(cap))),
            Stderr::Tail(n) => (Stdio::piped(), Some(Mode::Tail(n))),
            Stderr::AppendLog(path) => (log(&path, true)?.into(), None),
            Stderr::SameAsStdout => {
                let destination = if let Some(file) = &file {
                    file.try_clone()?.into()
                } else if out_mode.is_some() {
                    let (reader, writer) = io::pipe()?;
                    let clone = writer.try_clone()?;
                    shared = Some(reader);
                    shared_writer = Some(writer);
                    clone.into()
                } else {
                    Stdio::null()
                };
                (destination, None)
            }
        };
        let stdout = if let Some(file) = file {
            if truncate {
                file.set_len(0)?;
            }
            file.into()
        } else if let Some(writer) = shared_writer {
            writer.into()
        } else if out_mode.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        };
        Ok(Self {
            stdin,
            stdout,
            stderr,
            bytes,
            out_mode,
            err_mode,
            shared,
        })
    }
}

pub(super) struct Reader {
    rx: Receiver<io::Result<Output>>,
    result: Option<io::Result<Output>>,
}
impl Reader {
    pub fn start(reader: impl Read + Send + 'static, mode: Mode) -> Self {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(read(reader, mode));
        });
        Self { rx, result: None }
    }
    pub fn ready(&mut self) -> bool {
        if self.result.is_none() {
            self.result = match self.rx.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(io::Error::other(
                    "output reader ended without a result",
                ))),
            };
        }
        self.result.is_some()
    }
    fn collect(mut self, remaining: Duration) -> Option<Output> {
        self.result
            .take()
            .or_else(|| self.rx.recv_timeout(remaining).ok())?
            .ok()
    }
}

#[derive(Default)]
pub(super) struct Readers {
    pub out: Option<Reader>,
    pub err: Option<Reader>,
}
impl Readers {
    pub fn ready(&mut self) -> bool {
        let out = self.out.as_mut().is_none_or(Reader::ready);
        let err = self.err.as_mut().is_none_or(Reader::ready);
        out && err
    }
    pub fn collect(self, grace: Duration) -> Option<(Output, Output)> {
        let start = Instant::now();
        let out = match self.out {
            Some(r) => r.collect(grace.saturating_sub(start.elapsed()))?,
            None => Output::default(),
        };
        let err = match self.err {
            Some(r) => r.collect(grace.saturating_sub(start.elapsed()))?,
            None => Output::default(),
        };
        Some((out, err))
    }
}

fn read(mut reader: impl Read, mode: Mode) -> io::Result<Output> {
    match mode {
        Mode::Capture(cap) => {
            let mut bytes = Vec::new();
            reader.by_ref().take(cap).read_to_end(&mut bytes)?;
            let discarded = io::copy(&mut reader, &mut io::sink())?;
            Ok(Output {
                bytes,
                truncated: discarded != 0,
                ..Output::default()
            })
        }
        Mode::Tail(n) => {
            let mut tail = VecDeque::new();
            let mut chunk = [0; 8192];
            loop {
                let count = match reader.read(&mut chunk) {
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    result => result?,
                };
                if count == 0 {
                    break;
                }
                let keep = count.min(n);
                let discard = tail.len().saturating_sub(n - keep);
                tail.drain(..discard);
                tail.extend(&chunk[count - keep..count]);
            }
            Ok(Output {
                bytes: tail.into(),
                ..Output::default()
            })
        }
        Mode::Lines(max, callback) => read_lines(BufReader::new(reader), max, callback),
    }
}

fn read_lines(mut reader: impl BufRead, max: usize, mut callback: OnLine) -> io::Result<Output> {
    let mut line = Vec::new();
    let mut skipped = 0;
    loop {
        line.clear();
        let count = reader
            .by_ref()
            .take(max.saturating_add(1) as u64)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            break;
        }
        if line.last() == Some(&b'\n') {
            line.pop();
            callback(&line);
        } else if line.len() > max {
            skipped += 1;
            reader.skip_until(b'\n')?;
        } else {
            callback(&line);
        }
    }
    Ok(Output {
        skipped,
        ..Output::default()
    })
}
