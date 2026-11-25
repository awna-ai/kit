use std::process::{Command, Output};

fn run(args: &[&str], database: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_awna-intake"));
    command.env_clear().env("PATH", "/usr/bin:/bin").args(args);
    if let Some(database) = database {
        command.env("INTAKE_TEST_DB", database);
    }
    command.output().unwrap()
}

#[test]
fn bad_argv_is_usage() {
    for args in [
        vec![],
        vec!["--db-env", "DB", "pending"],
        vec![
            "--db-env",
            "postgres://someone:secret@localhost/db",
            "pending",
            "--json",
        ],
    ] {
        let result = run(&args, None);
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stdout.is_empty());
        assert!(String::from_utf8(result.stderr).unwrap().contains("Usage:"));
    }
}

#[test]
fn missing_variable_names_only_the_variable() {
    let result = run(&["--db-env", "INTAKE_TEST_DB", "pending", "--json"], None);
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&result.stderr),
        "awna-intake: INTAKE_TEST_DB is not set\n"
    );
    let empty = run(
        &["--db-env", "INTAKE_TEST_DB", "pending", "--json"],
        Some(""),
    );
    assert_eq!(empty.status.code(), Some(1));
    assert_eq!(empty.stderr, result.stderr);
}

#[test]
fn failed_connection_never_prints_credentials() {
    let result = run(
        &["--db-env", "INTAKE_TEST_DB", "pending", "--json"],
        Some("postgres://someone:pw-SENTINEL-7f3a@127.0.0.1:1/none"),
    );
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    let text = String::from_utf8(result.stderr).unwrap();
    assert!(!text.contains("SENTINEL"));
    assert!(!text.contains("someone"));
}

#[cfg(unix)]
#[test]
fn non_utf8_argument_is_usage() {
    use std::os::unix::ffi::OsStringExt;
    let result = Command::new(env!("CARGO_BIN_EXE_awna-intake"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .arg(std::ffi::OsString::from_vec(vec![255]))
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stdout.is_empty());
}

#[test]
fn broken_pipe_is_an_opaque_failure() {
    use std::process::Stdio;
    let (reader, writer) = std::os::unix::net::UnixStream::pair().unwrap();
    drop(reader);
    let child = Command::new(env!("CARGO_BIN_EXE_awna-intake"))
        .env_clear()
        .arg("--help")
        .stdout(Stdio::from(std::os::fd::OwnedFd::from(writer)))
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(result.stderr).unwrap(),
        "awna-intake: output could not be written\n"
    );
}
