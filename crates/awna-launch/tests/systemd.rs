use awna_launch::{CalendarTime, Kind, Logs, Platform, Run, Unit, UnitError, render};

fn unit(run: Run) -> Unit {
    Unit::new(
        "com.example.alpha",
        vec!["/opt/example/bin/alpha".into(), "serve".into()],
        run,
    )
}

#[test]
fn keep_alive_unit_renders_byte_for_byte() {
    let mut service = unit(Run::KeepAlive);
    service.logs = Logs::both("/var/tmp/example/alpha.log");
    assert_eq!(
        render(&service, Platform::Linux),
        Ok(concat!(
            "[Unit]\nDescription=com.example.alpha\nStartLimitIntervalSec=0\n\n",
            "[Service]\nType=simple\nExecStart=/opt/example/bin/alpha serve\n",
            "Restart=always\nRestartSec=1\n",
            "StandardOutput=append:/var/tmp/example/alpha.log\n",
            "StandardError=append:/var/tmp/example/alpha.log\n\n",
            "[Install]\nWantedBy=default.target\n",
        )
        .into()),
    );
}

#[test]
fn once_unit_does_not_restart() {
    let rendered = render(&unit(Run::Once), Platform::Linux).unwrap();
    assert!(rendered.lines().any(|line| line == "Restart=no"));
    assert!(!rendered.contains("RestartSec"));
    assert!(!rendered.contains("Standard"));
    assert!(!rendered.contains("StartLimitIntervalSec"));
}

#[test]
fn optional_fields_render_in_order() {
    let mut service = unit(Run::KeepAlive);
    service.env.insert("ZETA".into(), "last".into());
    service.env.insert("ALPHA".into(), "first".into());
    service.workdir = Some("/var/tmp/example/work".into());
    service.logs = Logs {
        stdout: Some("/dev/null".into()),
        stderr: Some("/var/tmp/example/err.log".into()),
    };
    service.nice = Some(-5);
    service.low_io = true;
    service.umask = Some(0o77);
    service.exit_timeout = Some(30);
    service.kind = Some(Kind::Background);
    assert_eq!(
        render(&service, Platform::Linux),
        Ok(concat!(
            "[Unit]\nDescription=com.example.alpha\nStartLimitIntervalSec=0\n\n",
            "[Service]\nType=simple\nExecStart=/opt/example/bin/alpha serve\n",
            "Restart=always\nRestartSec=1\n",
            "Environment=\"ALPHA=first\"\nEnvironment=\"ZETA=last\"\n",
            "WorkingDirectory=/var/tmp/example/work\n",
            "StandardOutput=null\nStandardError=append:/var/tmp/example/err.log\n",
            "Nice=-5\nIOSchedulingClass=idle\nUMask=0077\nTimeoutStopSec=30\n\n",
            "[Install]\nWantedBy=default.target\n",
        )
        .into()),
    );
}

#[test]
fn exec_and_environment_values_are_quoted_and_escaped() {
    let mut service = unit(Run::Once);
    service.argv = [
        "/opt/example/bin/alpha",
        "two words",
        "100%",
        "$HOME",
        r#"say "hi""#,
        r"back\slash",
        "",
        "--flag=a,b:c@d+e",
    ]
    .map(String::from)
    .into();
    service
        .env
        .insert("SAMPLE".into(), r#"a "b" 50% c\d $e"#.into());
    let rendered = render(&service, Platform::Linux).unwrap();
    assert_eq!(
        rendered.lines().find(|line| line.starts_with("ExecStart=")),
        Some(
            r#"ExecStart=/opt/example/bin/alpha "two words" "100%%" "$$HOME" "say \"hi\"" "back\\slash" "" --flag=a,b:c@d+e"#
        ),
    );
    assert!(
        rendered
            .lines()
            .any(|line| line == r#"Environment="SAMPLE=a \"b\" 50%% c\\d $e""#)
    );
}

#[test]
fn timer_runs_are_unsupported_on_linux() {
    for run in [
        Run::Interval(60),
        Run::Calendar(vec![CalendarTime {
            minute: 5,
            hour: None,
            day: None,
            weekday: None,
            month: None,
        }]),
    ] {
        let service = unit(run);
        assert_eq!(
            render(&service, Platform::Linux),
            Err(UnitError::Unsupported)
        );
        assert!(render(&service, Platform::MacOs).is_ok());
    }
}

#[test]
fn linux_refuses_paths_systemd_would_reinterpret() {
    for character in [' ', '%', '$', '"', '\'', 'é'] {
        let path = format!("/var/tmp/example/{character}path");
        let mut service = unit(Run::Once);
        service.logs = Logs::both(&path);
        assert_eq!(
            render(&service, Platform::Linux),
            Err(UnitError::Path),
            "log path containing {character:?}",
        );
        if character == ' ' {
            assert!(render(&service, Platform::MacOs).is_ok());
        }

        service.logs = Logs::default();
        service.workdir = Some(path.into());
        assert_eq!(
            render(&service, Platform::Linux),
            Err(UnitError::Path),
            "working directory containing {character:?}",
        );
        if character == ' ' {
            assert!(render(&service, Platform::MacOs).is_ok());
        }
    }
}

#[test]
fn linux_refuses_values_systemd_would_refuse_or_not_expand() {
    let mut cases = Vec::new();
    for argument in ["a\u{1fffe}b", "a\u{fdd0}b"] {
        let mut service = unit(Run::KeepAlive);
        service.argv.push(argument.into());
        cases.push((service, UnitError::Text));
    }
    let mut service = unit(Run::KeepAlive);
    service.env.insert("SAMPLE".into(), "a\u{fdef}b".into());
    cases.push((service, UnitError::Text));
    let mut service = unit(Run::KeepAlive);
    service.nice = Some(20);
    cases.push((service, UnitError::Nice));
    let mut service = unit(Run::KeepAlive);
    service.argv[0] = "/opt/example/$bin/alpha".into();
    cases.push((service, UnitError::Program));

    for (service, error) in cases {
        assert_eq!(render(&service, Platform::Linux), Err(error), "{service:?}");
        assert!(render(&service, Platform::MacOs).is_ok(), "{service:?}");
    }
}

#[test]
fn linux_accepts_text_and_nice_systemd_allows() {
    let mut service = unit(Run::KeepAlive);
    for argument in ["a\u{fdcf}b", "a\u{fdf0}b", "a\u{fffd}b", "a\u{1f600}b"] {
        service.argv.push(argument.into());
    }
    service.nice = Some(19);
    let rendered = render(&service, Platform::Linux).unwrap();
    assert!(rendered.contains("\"a\u{1f600}b\""));
    assert!(rendered.contains("\nNice=19\n"));
}
