use std::path::Path;

use crate::systemd;
use crate::unit::{CalendarTime, Kind, Run, Unit, UnitError};

/// The platform whose service file format to render.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// A macOS launchd property list.
    MacOs,
    /// A systemd user service unit.
    Linux,
}

/// Validates a service unit and renders it in the platform's service file format.
///
/// Linux supports `Run::KeepAlive` and `Run::Once`. Interval and calendar schedules
/// return `UnitError::Unsupported` because systemd requires a separate timer unit. Linux also
/// refuses a nice of 20, Unicode noncharacters, and `$` in the program path, which systemd
/// would reject or read differently.
pub fn render(unit: &Unit, platform: Platform) -> Result<String, UnitError> {
    unit.validate()?;
    match platform {
        Platform::MacOs => Ok(render_plist(unit)),
        Platform::Linux => systemd::render(unit),
    }
}

fn render_plist(unit: &Unit) -> String {
    let mut plist = String::from(concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
        "<plist version=\"1.0\">\n<dict>\n",
    ));
    entry(&mut plist, 1, "Label", "string", &unit.label);
    element(&mut plist, 1, "key", "ProgramArguments");
    line(&mut plist, 1, "<array>");
    for arg in &unit.argv {
        element(&mut plist, 2, "string", arg);
    }
    line(&mut plist, 1, "</array>");

    if !unit.env.is_empty() {
        element(&mut plist, 1, "key", "EnvironmentVariables");
        line(&mut plist, 1, "<dict>");
        for (key, value) in &unit.env {
            entry(&mut plist, 2, key, "string", value);
        }
        line(&mut plist, 1, "</dict>");
    }
    path_entry(&mut plist, "WorkingDirectory", unit.workdir.as_deref());
    element(&mut plist, 1, "key", "RunAtLoad");
    let run_at_load = if matches!(unit.run, Run::Calendar(_)) {
        "<false/>"
    } else {
        "<true/>"
    };
    line(&mut plist, 1, run_at_load);
    match &unit.run {
        Run::KeepAlive => {
            element(&mut plist, 1, "key", "KeepAlive");
            line(&mut plist, 1, "<true/>");
        }
        Run::Interval(seconds) => integer_entry(&mut plist, 1, "StartInterval", *seconds),
        Run::Calendar(times) => calendar_entry(&mut plist, times),
        Run::Once => {}
    }
    path_entry(&mut plist, "StandardOutPath", unit.logs.stdout.as_deref());
    path_entry(&mut plist, "StandardErrorPath", unit.logs.stderr.as_deref());
    if let Some(kind) = unit.kind {
        let kind = match kind {
            Kind::Background => "Background",
            Kind::Standard => "Standard",
            Kind::Interactive => "Interactive",
            Kind::Adaptive => "Adaptive",
        };
        entry(&mut plist, 1, "ProcessType", "string", kind);
    }
    if let Some(nice) = unit.nice {
        integer_entry(&mut plist, 1, "Nice", nice);
    }
    if unit.low_io {
        element(&mut plist, 1, "key", "LowPriorityIO");
        line(&mut plist, 1, "<true/>");
    }
    if let Some(mask) = unit.umask {
        integer_entry(&mut plist, 1, "Umask", mask);
    }
    if let Some(seconds) = unit.exit_timeout {
        integer_entry(&mut plist, 1, "ExitTimeOut", seconds);
    }
    plist.push_str("</dict>\n</plist>\n");
    plist
}

fn calendar_entry(plist: &mut String, times: &[CalendarTime]) {
    element(plist, 1, "key", "StartCalendarInterval");
    if let [time] = times {
        calendar_dict(plist, 1, time);
    } else {
        line(plist, 1, "<array>");
        for time in times {
            calendar_dict(plist, 2, time);
        }
        line(plist, 1, "</array>");
    }
}

fn calendar_dict(plist: &mut String, depth: usize, time: &CalendarTime) {
    line(plist, depth, "<dict>");
    for (key, value) in [
        ("Month", time.month),
        ("Day", time.day),
        ("Weekday", time.weekday),
        ("Hour", time.hour),
        ("Minute", Some(time.minute)),
    ] {
        if let Some(value) = value {
            integer_entry(plist, depth + 1, key, value);
        }
    }
    line(plist, depth, "</dict>");
}

fn path_entry(plist: &mut String, key: &str, path: Option<&Path>) {
    if let Some(path) = path.and_then(Path::to_str) {
        entry(plist, 1, key, "string", path);
    }
}

fn integer_entry(plist: &mut String, depth: usize, key: &str, value: impl ToString) {
    entry(plist, depth, key, "integer", &value.to_string());
}

fn entry(plist: &mut String, depth: usize, key: &str, tag: &str, value: &str) {
    element(plist, depth, "key", key);
    element(plist, depth, tag, value);
}

fn element(plist: &mut String, depth: usize, tag: &str, value: &str) {
    plist.extend(std::iter::repeat_n('\t', depth));
    plist.push('<');
    plist.push_str(tag);
    plist.push('>');
    for character in value.chars() {
        match character {
            '&' => plist.push_str("&amp;"),
            '<' => plist.push_str("&lt;"),
            '>' => plist.push_str("&gt;"),
            '"' => plist.push_str("&quot;"),
            '\'' => plist.push_str("&apos;"),
            _ => plist.push(character),
        }
    }
    plist.push_str("</");
    plist.push_str(tag);
    plist.push_str(">\n");
}

fn line(plist: &mut String, depth: usize, text: &str) {
    plist.extend(std::iter::repeat_n('\t', depth));
    plist.push_str(text);
    plist.push('\n');
}

#[cfg(test)]
mod tests {
    use super::element;

    #[test]
    fn entity_like_text_stays_literal_and_unicode_is_preserved() {
        let mut xml = String::new();
        element(&mut xml, 1, "string", "&lt; café");
        assert_eq!(xml, "\t<string>&amp;lt; café</string>\n");
    }
}
