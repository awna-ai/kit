use std::fmt::Write;
use std::path::Path;

use crate::unit::{Run, Unit, UnitError};

pub(crate) fn render(unit: &Unit) -> Result<String, UnitError> {
    let restart = match unit.run {
        Run::KeepAlive => "Restart=always\nRestartSec=1\n",
        Run::Once => "Restart=no\n",
        Run::Interval(_) | Run::Calendar(_) => return Err(UnitError::Unsupported),
    };
    let mut text = format!(
        "[Unit]\nDescription={}\n\n[Service]\nType=simple\nExecStart=",
        unit.label,
    );
    for (index, arg) in unit.argv.iter().enumerate() {
        if index != 0 {
            text.push(' ');
        }
        if !arg.is_empty()
            && arg
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"/._-+,:=@".contains(&byte))
        {
            text.push_str(arg);
        } else {
            text.push('"');
            escape(&mut text, arg, true);
            text.push('"');
        }
    }
    text.push('\n');
    text.push_str(restart);

    for (key, value) in &unit.env {
        text.push_str("Environment=\"");
        text.push_str(key);
        text.push('=');
        escape(&mut text, value, false);
        text.push_str("\"\n");
    }
    if let Some(path) = &unit.workdir {
        writeln!(text, "WorkingDirectory={}", checked_path(path)?).unwrap();
    }
    log_entry(&mut text, "StandardOutput", unit.logs.stdout.as_deref())?;
    log_entry(&mut text, "StandardError", unit.logs.stderr.as_deref())?;
    if let Some(nice) = unit.nice {
        writeln!(text, "Nice={nice}").unwrap();
    }
    if unit.low_io {
        text.push_str("IOSchedulingClass=idle\n");
    }
    if let Some(mask) = unit.umask {
        writeln!(text, "UMask={mask:04o}").unwrap();
    }
    if let Some(seconds) = unit.exit_timeout {
        writeln!(text, "TimeoutStopSec={seconds}").unwrap();
    }
    text.push_str("\n[Install]\nWantedBy=default.target\n");
    Ok(text)
}

fn escape(text: &mut String, value: &str, expand_dollars: bool) {
    for character in value.chars() {
        match character {
            '\\' | '"' => text.push('\\'),
            '%' => text.push('%'),
            '$' if expand_dollars => text.push('$'),
            _ => {}
        }
        text.push(character);
    }
}

fn checked_path(path: &Path) -> Result<&str, UnitError> {
    path.to_str()
        .filter(|path| {
            path.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"/._-+".contains(&byte))
        })
        .ok_or(UnitError::Path)
}

fn log_entry(text: &mut String, key: &str, path: Option<&Path>) -> Result<(), UnitError> {
    if let Some(path) = path {
        let path = checked_path(path)?;
        if path == "/dev/null" {
            writeln!(text, "{key}=null").unwrap();
        } else {
            writeln!(text, "{key}=append:{path}").unwrap();
        }
    }
    Ok(())
}
