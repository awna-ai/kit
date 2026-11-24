//! Summarizes a print-mode CLI's JSON result and its line-delimited `stream-json` events.
//!
//! The caller must limit how much it reads because each line is parsed in full, regardless of
//! length. The summary is the stream's own advisory account. It has no authentication and a later
//! line can override an earlier one.

use serde_json::{Map, Value};

use crate::summary::{print_spend, string};
use crate::{RESETS_AT_MAX, RateLimit, ResultLine, Summary, Window, Windows, is_category, is_uuid};

/// The CLI's warning when background tasks prevent a clean exit.
pub const BACKGROUND_CUT_OFF: &str = "Background tasks still running";

/// Parses one whole line at a time, so the caller must limit how much it reads. The stream
/// supplies an advisory, unauthenticated account, and later lines can override earlier ones.
#[derive(Debug, Default, Clone)]
pub struct Summarizer {
    summary: Summary,
}

impl Summarizer {
    /// Creates an empty collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads one line, with or without its line ending.
    pub fn push(&mut self, line: &[u8]) {
        let line = line.trim_ascii();
        if line.is_empty() {
            return;
        }

        let Ok(Value::Object(object)) = serde_json::from_slice(line) else {
            self.summary.non_json_lines = self.summary.non_json_lines.saturating_add(1);
            self.summary.background_cut_off |= has_background_warning(line);
            return;
        };

        match string(&object, "type") {
            Some("system") => {
                self.summary.background_cut_off |= has_background_warning(line);
                if string(&object, "subtype") == Some("init")
                    && self.summary.permission_mode.is_none()
                {
                    self.summary.permission_mode = string(&object, "permissionMode")
                        .filter(|mode| is_category(mode))
                        .map(str::to_owned);
                }
            }
            Some("assistant") => {
                if let Some(error) = string(&object, "error") {
                    self.summary.api_error = is_category(error).then(|| error.to_owned());
                }
            }
            Some("rate_limit_event") => {
                if let Some(info) = object.get("rate_limit_info").and_then(Value::as_object) {
                    record_rate_limit(&mut self.summary.rate_limit, info);
                }
            }
            Some("result") => self.record_result(&object),
            _ => {}
        }
    }

    /// Returns the facts collected from all lines.
    pub fn finish(self) -> Summary {
        self.summary
    }

    fn record_result(&mut self, object: &Map<String, Value>) {
        let subtype = string(object, "subtype").filter(|subtype| is_category(subtype));
        let is_error = object.get("is_error").and_then(Value::as_bool);
        if subtype == Some("success") && is_error == Some(false) {
            self.summary.api_error = None;
        }
        let spend = print_spend(object);
        if let Some(spend) = &spend {
            self.summary.spend = Some(spend.clone());
        }
        self.summary.result = Some(ResultLine {
            subtype: subtype.map(str::to_owned),
            is_error,
            text: string(object, "result").map(str::to_owned),
            session_id: string(object, "session_id")
                .filter(|id| is_uuid(id))
                .map(str::to_owned),
            structured_output: object.get("structured_output").cloned(),
            spend,
        });
    }
}

/// Collects facts from a complete output buffer.
pub fn summarize(output: &[u8]) -> Summary {
    let mut summarizer = Summarizer::new();
    for line in output.split(|&byte| byte == b'\n') {
        summarizer.push(line);
    }
    summarizer.finish()
}

fn has_background_warning(line: &[u8]) -> bool {
    line.windows(BACKGROUND_CUT_OFF.len())
        .any(|part| part == BACKGROUND_CUT_OFF.as_bytes())
}

fn record_rate_limit(rate_limit: &mut RateLimit, info: &Map<String, Value>) {
    match string(info, "status") {
        Some("rejected") => {
            rate_limit.rejected = true;
            if let Some(until) = info.get("resetsAt").and_then(resets_at) {
                rate_limit.rejected_until =
                    Some(rate_limit.rejected_until.map_or(until, |at| at.max(until)));
            }
        }
        Some("allowed_warning") => rate_limit.warned = true,
        _ => {}
    }
    if let Some(windows) = info.get("unifiedWindows").and_then(windows) {
        rate_limit.first_windows.get_or_insert(windows);
        rate_limit.windows = Some(windows);
    }
}

fn windows(value: &Value) -> Option<Windows> {
    let windows = Windows {
        five_hour: value.get("five_hour").and_then(window),
        seven_day: value.get("seven_day").and_then(window),
    };
    (windows.five_hour.is_some() || windows.seven_day.is_some()).then_some(windows)
}

fn window(value: &Value) -> Option<Window> {
    let utilization = value.get("utilization")?.as_f64()?;
    (utilization.is_finite() && utilization >= 0.0).then_some(())?;
    Some(Window {
        utilization: utilization.min(1.0),
        resets_at: resets_at(value.get("resetsAt")?)?,
    })
}

fn resets_at(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .filter(|at| at.is_finite() && *at > 0.0 && *at < RESETS_AT_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_json_line_count_saturates() {
        let mut summarizer = Summarizer::new();
        summarizer.summary.non_json_lines = usize::MAX;
        summarizer.push(b"plain text");
        assert_eq!(summarizer.finish().non_json_lines, usize::MAX);
    }
}
