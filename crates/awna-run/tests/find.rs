use awna_run::find_program;
use std::{
    ffi::OsStr,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("awna-run-{}-{name}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn executable(&self, name: &str) -> PathBuf {
        let p = self.0.join(name);
        fs::write(&p, b"#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        p
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn an_absolute_runnable_path_is_returned_as_is() {
    let f = Fixture::new("quartz");
    let p = f.executable("spark");
    assert_eq!(
        find_program(p.to_str().unwrap(), None, &[]),
        Some(p.clone())
    );
    fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(find_program(p.to_str().unwrap(), None, &[]), None);
    assert_eq!(
        find_program(f.0.join("missing").to_str().unwrap(), None, &[]),
        None
    );
    assert_eq!(find_program("bin/spark", None, &[p]), None);
}

#[test]
fn a_bare_name_uses_only_absolute_runnable_search_entries() {
    let f = Fixture::new("pebble");
    for dir in ["folder", "quiet", "first", "last"] {
        fs::create_dir(f.0.join(dir)).unwrap();
    }
    fs::create_dir(f.0.join("folder/spark")).unwrap();
    fs::write(f.0.join("quiet/spark"), "plain").unwrap();
    fs::set_permissions(f.0.join("quiet/spark"), fs::Permissions::from_mode(0o644)).unwrap();
    let first = f.executable("first/spark");
    f.executable("last/spark");
    let search = std::env::join_paths([
        Path::new(""),
        Path::new("."),
        Path::new("bin"),
        &f.0.join("folder"),
        &f.0.join("quiet"),
        &f.0.join("first"),
        &f.0.join("last"),
    ])
    .unwrap();
    assert_eq!(find_program("spark", Some(&search), &[]), Some(first));
    let relative: PathBuf = std::env::current_dir()
        .unwrap()
        .components()
        .skip(1)
        .map(|_| "..")
        .collect::<PathBuf>()
        .join("bin");
    assert!(relative.join("sh").is_file());
    let search = std::env::join_paths([relative.as_path(), Path::new(""), Path::new(".")]).unwrap();
    assert_eq!(find_program("sh", Some(&search), &[]), None);
}

#[test]
fn fallbacks_are_tried_in_order_when_search_has_no_match() {
    let f = Fixture::new("cinder");
    let first = f.executable("first");
    let second = f.executable("second");
    let candidates = [f.0.join("absent"), f.0.clone(), first.clone(), second];
    assert_eq!(
        find_program("spark", Some(OsStr::new(".:bin::")), &candidates),
        Some(first.clone())
    );
    assert_eq!(find_program("spark", None, &candidates), Some(first));
    assert_eq!(find_program("spark", None, &[]), None);
}
