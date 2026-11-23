use awna_launch::{CalendarTime, Kind, Logs, Platform, Run, Unit, UnitError, render, valid_label};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::atomic::{AtomicU32, Ordering};

const START: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>com.example.alpha</string>
	<key>ProgramArguments</key>
	<array>
		<string>/opt/example/bin/alpha</string>
		<string>serve</string>
	</array>
"#;
const END: &str = "</dict>\n</plist>\n";

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "example-render-{}-{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn unit(run: Run) -> Unit {
    Unit::new(
        "com.example.alpha",
        vec!["/opt/example/bin/alpha".into(), "serve".into()],
        run,
    )
}

fn lint_plist(xml: &str) {
    let program = Path::new("/usr/bin/plutil");
    if program.is_file() {
        let dir = TestDir::new();
        let path = dir.0.join("com.example.alpha.plist");
        fs::write(&path, xml).unwrap();
        let output = Command::new(program)
            .arg("-lint")
            .arg(path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
}

fn assert_render(unit: &Unit, body: &str) {
    let xml = render(unit, Platform::MacOs).unwrap();
    assert_eq!(xml, format!("{START}{body}{END}"));
    lint_plist(&xml);
}

#[test]
fn keep_alive_renders_the_service_with_shared_logs() {
    let mut service = unit(Run::KeepAlive);
    service.logs = Logs::both("/var/tmp/example/alpha.log");
    service.kind = Some(Kind::Background);
    service.umask = Some(0o077);
    assert_render(
        &service,
        "	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<true/>
	<key>StandardOutPath</key>
	<string>/var/tmp/example/alpha.log</string>
	<key>StandardErrorPath</key>
	<string>/var/tmp/example/alpha.log</string>
	<key>ProcessType</key>
	<string>Background</string>
	<key>Umask</key>
	<integer>63</integer>
",
    );
}

#[test]
fn interval_renders_seconds_and_runs_at_load() {
    assert_render(
        &unit(Run::Interval(60)),
        "	<key>RunAtLoad</key>
	<true/>
	<key>StartInterval</key>
	<integer>60</integer>
",
    );
}

fn calendar(minute: u8) -> CalendarTime {
    CalendarTime {
        minute,
        hour: None,
        day: None,
        weekday: None,
        month: None,
    }
}

#[test]
fn one_calendar_time_renders_as_a_dictionary() {
    let time = CalendarTime {
        minute: 15,
        hour: Some(8),
        day: Some(9),
        weekday: Some(7),
        month: Some(10),
    };
    assert_render(
        &unit(Run::Calendar(vec![time])),
        "	<key>RunAtLoad</key>
	<false/>
	<key>StartCalendarInterval</key>
	<dict>
		<key>Month</key>
		<integer>10</integer>
		<key>Day</key>
		<integer>9</integer>
		<key>Weekday</key>
		<integer>7</integer>
		<key>Hour</key>
		<integer>8</integer>
		<key>Minute</key>
		<integer>15</integer>
	</dict>
",
    );
}

#[test]
fn multiple_calendar_times_render_as_an_ordered_array() {
    assert_render(
        &unit(Run::Calendar(vec![calendar(59), calendar(0)])),
        "	<key>RunAtLoad</key>
	<false/>
	<key>StartCalendarInterval</key>
	<array>
		<dict>
			<key>Minute</key>
			<integer>59</integer>
		</dict>
		<dict>
			<key>Minute</key>
			<integer>0</integer>
		</dict>
	</array>
",
    );
}

#[test]
fn once_runs_at_load() {
    assert_render(
        &unit(Run::Once),
        "	<key>RunAtLoad</key>
	<true/>
",
    );
}

#[test]
fn optional_fields_render_in_order_and_environment_keys_are_sorted() {
    let mut service = unit(Run::Once);
    service.env.insert("SECOND".into(), "two".into());
    service.env.insert("FIRST".into(), "one".into());
    service.workdir = Some("/var/tmp/example".into());
    service.logs.stdout = Some("/var/tmp/example/stdout.log".into());
    service.logs.stderr = Some("/var/tmp/example/stderr.log".into());
    service.kind = Some(Kind::Adaptive);
    service.nice = Some(-10);
    service.low_io = true;
    service.umask = Some(0o777);
    service.exit_timeout = Some(5);
    assert_render(
        &service,
        "	<key>EnvironmentVariables</key>
	<dict>
		<key>FIRST</key>
		<string>one</string>
		<key>SECOND</key>
		<string>two</string>
	</dict>
	<key>WorkingDirectory</key>
	<string>/var/tmp/example</string>
	<key>RunAtLoad</key>
	<true/>
	<key>StandardOutPath</key>
	<string>/var/tmp/example/stdout.log</string>
	<key>StandardErrorPath</key>
	<string>/var/tmp/example/stderr.log</string>
	<key>ProcessType</key>
	<string>Adaptive</string>
	<key>Nice</key>
	<integer>-10</integer>
	<key>LowPriorityIO</key>
	<true/>
	<key>Umask</key>
	<integer>511</integer>
	<key>ExitTimeOut</key>
	<integer>5</integer>
",
    );
}

#[test]
fn process_types_use_their_launchd_spelling() {
    for (kind, name) in [
        (Kind::Background, "Background"),
        (Kind::Standard, "Standard"),
        (Kind::Interactive, "Interactive"),
        (Kind::Adaptive, "Adaptive"),
    ] {
        let mut service = unit(Run::Once);
        service.kind = Some(kind);
        assert_render(
            &service,
            &format!(
                "\t<key>RunAtLoad</key>\n\t<true/>\n\t<key>ProcessType</key>\n\t<string>{name}</string>\n"
            ),
        );
    }
}

#[test]
fn xml_metacharacters_are_escaped_in_arguments_environment_and_paths() {
    let mut service = unit(Run::Once);
    let specials = "& < > \" '";
    let escaped = "&amp; &lt; &gt; &quot; &apos;";
    service.argv.push(format!("arg {specials}"));
    service
        .env
        .insert("EXAMPLE".into(), format!("env {specials}"));
    service.workdir = Some(format!("/work/{specials}").into());
    service.logs.stdout = Some(format!("/out/{specials}").into());
    service.logs.stderr = Some(format!("/err/{specials}").into());
    let xml = render(&service, Platform::MacOs).unwrap();
    for prefix in ["arg ", "env ", "/work/", "/out/", "/err/"] {
        assert!(xml.contains(&format!("<string>{prefix}{escaped}</string>")));
    }
    lint_plist(&xml);
}

fn assert_invalid(unit: &Unit, error: UnitError) {
    assert_eq!(unit.validate(), Err(error));
    assert_eq!(render(unit, Platform::MacOs), Err(error));
}

#[test]
fn labels_allow_only_safe_ascii_components_within_the_limit() {
    for label in [
        "",
        "../x",
        "-x",
        "a b",
        "a/b",
        ".x",
        "a..b",
        "é",
        &"x".repeat(201),
    ] {
        assert!(!valid_label(label), "{label:?}");
        let mut service = unit(Run::Once);
        service.label = label.into();
        assert_invalid(&service, UnitError::Label);
    }
    for label in ["A", "0", "a.B_0-9", &"x".repeat(200)] {
        assert!(valid_label(label), "{label:?}");
    }
}

#[test]
fn validation_reports_the_first_invalid_field() {
    let mut service = unit(Run::Interval(0));
    service.label.clear();
    service.argv.clear();
    service.logs.stdout = Some("relative.log".into());
    service.env.insert("1A".into(), String::new());
    service.nice = Some(21);
    service.umask = Some(0o1000);
    service.exit_timeout = Some(0);

    assert_invalid(&service, UnitError::Label);
    service.label = "com.example.alpha".into();
    assert_invalid(&service, UnitError::Argv);
    service.argv = vec!["relative\n".into()];
    assert_invalid(&service, UnitError::Program);
    service.argv = vec!["/opt/example/bin/alpha".into(), "line\nbreak".into()];
    assert_invalid(&service, UnitError::Text);
    service.argv.pop();
    assert_invalid(&service, UnitError::Path);
    service.logs.stdout = None;
    assert_invalid(&service, UnitError::EnvKey);
    service.env.clear();
    assert_invalid(&service, UnitError::Interval);
    service.run = Run::Calendar(vec![]);
    assert_invalid(&service, UnitError::Calendar);
    service.run = Run::Once;
    assert_invalid(&service, UnitError::Nice);
    service.nice = None;
    assert_invalid(&service, UnitError::Umask);
    service.umask = None;
    assert_invalid(&service, UnitError::ExitTimeout);
    service.exit_timeout = None;
    assert_eq!(service.validate(), Ok(()));
}

#[test]
fn text_validation_excludes_control_characters_and_xml_noncharacters() {
    for character in ['\0', '\n', '\u{1f}', '\u{7f}', '\u{fffe}', '\u{ffff}'] {
        let mut service = unit(Run::Once);
        service.argv.push(character.to_string());
        assert_invalid(&service, UnitError::Text);
        service.argv.pop();
        service.env.insert(character.to_string(), "value".into());
        assert_invalid(&service, UnitError::Text);
        service.env.clear();
        service.env.insert("EXAMPLE".into(), character.to_string());
        assert_invalid(&service, UnitError::Text);
    }
}

#[cfg(unix)]
#[test]
fn paths_must_be_absolute_utf8_and_free_of_control_characters() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let mut service = unit(Run::Once);
    service.logs.stdout = Some("/example/out\n".into());
    assert_invalid(&service, UnitError::Path);
    service.logs.stdout = None;
    service.logs.stderr = Some(OsString::from_vec(vec![b'/', 255]).into());
    assert_invalid(&service, UnitError::Path);
    service.logs.stderr = None;
    service.workdir = Some("relative".into());
    assert_invalid(&service, UnitError::Path);
}

#[test]
fn calendar_checks_every_field_and_accepts_range_limits() {
    for time in [
        calendar(60),
        CalendarTime {
            hour: Some(24),
            ..calendar(0)
        },
        CalendarTime {
            day: Some(0),
            ..calendar(0)
        },
        CalendarTime {
            day: Some(32),
            ..calendar(0)
        },
        CalendarTime {
            weekday: Some(8),
            ..calendar(0)
        },
        CalendarTime {
            month: Some(0),
            ..calendar(0)
        },
        CalendarTime {
            month: Some(13),
            ..calendar(0)
        },
    ] {
        assert_invalid(
            &unit(Run::Calendar(vec![calendar(0), time])),
            UnitError::Calendar,
        );
    }
    let minimum = CalendarTime {
        minute: 0,
        hour: Some(0),
        day: Some(1),
        weekday: Some(0),
        month: Some(1),
    };
    let maximum = CalendarTime {
        minute: 59,
        hour: Some(23),
        day: Some(31),
        weekday: Some(7),
        month: Some(12),
    };
    assert_eq!(
        unit(Run::Calendar(vec![minimum, maximum])).validate(),
        Ok(())
    );
}

#[test]
fn nice_checks_both_limits() {
    let mut service = unit(Run::Once);
    for nice in [-21, 21] {
        service.nice = Some(nice);
        assert_invalid(&service, UnitError::Nice);
    }
    for nice in [-20, 20] {
        service.nice = Some(nice);
        assert_eq!(service.validate(), Ok(()));
    }
}

#[test]
fn new_sets_only_the_required_fields() {
    assert_eq!(
        unit(Run::Once),
        Unit {
            label: "com.example.alpha".into(),
            argv: vec!["/opt/example/bin/alpha".into(), "serve".into()],
            run: Run::Once,
            logs: Logs {
                stdout: None,
                stderr: None,
            },
            env: Default::default(),
            workdir: None,
            nice: None,
            low_io: false,
            kind: None,
            umask: None,
            exit_timeout: None,
        }
    );
}

#[test]
fn each_required_setting_is_validated_on_its_own() {
    let service = unit(Run::Once);
    let cases = [
        (
            Unit {
                argv: vec![],
                ..service.clone()
            },
            UnitError::Argv,
        ),
        (
            Unit {
                argv: vec!["relative".into()],
                ..service.clone()
            },
            UnitError::Program,
        ),
        (
            Unit {
                logs: Logs::both("relative.log"),
                ..service.clone()
            },
            UnitError::Path,
        ),
        (
            Unit {
                run: Run::Interval(0),
                ..service.clone()
            },
            UnitError::Interval,
        ),
        (
            Unit {
                run: Run::Calendar(vec![]),
                ..service.clone()
            },
            UnitError::Calendar,
        ),
        (
            Unit {
                umask: Some(0o1000),
                ..service.clone()
            },
            UnitError::Umask,
        ),
        (
            Unit {
                exit_timeout: Some(0),
                ..service
            },
            UnitError::ExitTimeout,
        ),
    ];
    for (service, error) in cases {
        assert_invalid(&service, error);
    }
}

#[test]
fn environment_keys_use_ascii_identifier_syntax() {
    for key in ["", "1A", "A-B", "é", "A B", "A.B"] {
        let mut service = unit(Run::Once);
        service.env.insert(key.into(), String::new());
        assert_invalid(&service, UnitError::EnvKey);
    }
    for key in ["_", "_A0", "aA1"] {
        let mut service = unit(Run::Once);
        service.env.insert(key.into(), String::new());
        assert_eq!(service.validate(), Ok(()));
    }
}
