use crate::{CredentialStore, Error, Item, MAX_SECRET, Secret, item_name, read_back, storable};
use std::{
    ffi::OsString,
    io::Write,
    path::PathBuf,
    process::{Command, ExitStatus, Stdio},
};
use zeroize::{Zeroize, Zeroizing};

/// The macOS keychain through a program, available on every Unix for testing.
#[derive(Debug, Clone)]
pub struct SecurityCli {
    program: PathBuf,
    home: Option<OsString>,
}
impl SecurityCli {
    /// Uses `/usr/bin/security` and the caller's `HOME`.
    pub fn new() -> SecurityCli {
        Self::with_program("/usr/bin/security", std::env::var_os("HOME"))
    }
    /// Uses a stand-in program with only the supplied `HOME` in its environment.
    pub fn with_program(program: impl Into<PathBuf>, home: Option<OsString>) -> SecurityCli {
        SecurityCli {
            program: program.into(),
            home,
        }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(home) = &self.home {
            command.env("HOME", home);
        }
        command
    }
    fn failed(item: &Item, op: &'static str, why: impl Into<String>) -> Error {
        Error::Failed {
            item: item_name(item),
            op,
            why: why.into(),
        }
    }
    fn status(item: &Item, op: &'static str, status: ExitStatus) -> Error {
        let why = match status.code() {
            Some(code) => format!("exit {code}"),
            None => "killed".into(),
        };
        Self::failed(item, op, why)
    }
    fn item_command(&self, operation: &str, item: &Item) -> Command {
        let mut command = self.command();
        command.args([operation, "-s", item.service(), "-a", item.account()]);
        command
    }
}
impl Default for SecurityCli {
    fn default() -> Self {
        Self::new()
    }
}

fn store_line(item: &Item, value: &Secret) -> Zeroizing<String> {
    let parts = [
        "add-generic-password -U -s ",
        item.service(),
        " -a ",
        item.account(),
        " -l \"",
        item.label(),
        "\" -w \"",
        value.expose(),
        "\"\n",
    ];
    let capacity = parts.iter().map(|part| part.len()).sum();
    let mut line = Zeroizing::new(String::with_capacity(capacity));
    for part in parts {
        line.push_str(part);
    }
    line
}

impl CredentialStore for SecurityCli {
    fn find(&self, item: &Item) -> Result<Option<Secret>, Error> {
        let output = self
            .item_command("find-generic-password", item)
            .arg("-w")
            .stdout(Stdio::piped())
            .output()
            .map_err(|_| Self::failed(item, "find", "could not run"))?;
        let mut bytes = Zeroizing::new(output.stdout);
        if output.status.code() == Some(44) {
            return Ok(None);
        }
        if !output.status.success() {
            return Err(Self::status(item, "find", output.status));
        }
        if bytes.last() == Some(&b'\n') {
            bytes.pop();
        }
        if bytes.is_empty() {
            return Err(Error::Unreadable {
                item: item_name(item),
            });
        }
        match String::from_utf8(std::mem::take(&mut *bytes)) {
            Ok(value) => Ok(Some(Secret::new(value))),
            Err(error) => {
                error.into_bytes().zeroize();
                Err(Error::Unreadable {
                    item: item_name(item),
                })
            }
        }
    }
    fn exists(&self, item: &Item) -> Result<bool, Error> {
        let status = self
            .item_command("find-generic-password", item)
            .status()
            .map_err(|_| Self::failed(item, "exists", "could not run"))?;
        match status.code() {
            Some(0) => Ok(true),
            Some(44) => Ok(false),
            _ => Err(Self::status(item, "exists", status)),
        }
    }
    fn store(&self, item: &Item, value: &Secret) -> Result<(), Error> {
        if !storable(value, MAX_SECRET) {
            return Err(Error::NotStorable);
        }
        let line = store_line(item, value);
        let mut child = self
            .command()
            .arg("-i")
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|_| Self::failed(item, "store", "could not run"))?;
        // Taking ownership closes stdin before waiting, including when writing fails.
        let written = match child.stdin.take() {
            Some(mut stdin) => stdin.write_all(line.as_bytes()),
            None => Err(std::io::Error::other("missing input pipe")),
        };
        if written.is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Self::failed(item, "store", "could not run"));
        }
        let status = child
            .wait()
            .map_err(|_| Self::failed(item, "store", "could not run"))?;
        if !status.success() {
            return Err(Self::status(item, "store", status));
        }
        read_back(self, item, value)
    }
    fn delete(&self, item: &Item) -> Result<(), Error> {
        let status = self
            .item_command("delete-generic-password", item)
            .status()
            .map_err(|_| Self::failed(item, "delete", "could not run"))?;
        match status.code() {
            Some(0 | 44) => Ok(()),
            _ => Err(Self::status(item, "delete", status)),
        }
    }
}
