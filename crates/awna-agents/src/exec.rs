//! Summaries of an exec-mode CLI's JSON Lines events: one JSON object per line, typed
//! `item.completed`, `turn.completed`, `turn.failed` or `error`.
//!
//! The caller must limit how much it reads because each line is parsed in full, regardless of
//! length. The summary is the stream's own advisory account. It has no authentication and a later
//! line can override an earlier one.

use serde_json::{Map, Value};

use crate::summary::{count, string};
use crate::{ResultLine, Spend, Summary, Tokens};

/// Summarizes the JSON events in an exec stream. Each line is parsed whole, so the caller limits
/// how much it passes in.
pub fn summarize(output: &[u8]) -> Summary {
    summarize_after(output, 0)
}

/// Summarizes `output` as if `non_json_lines` lines had already been read.
fn summarize_after(output: &[u8], non_json_lines: usize) -> Summary {
    let mut summary = Summary {
        non_json_lines,
        ..Summary::default()
    };
    let mut reply = None;
    let mut failed = None;

    for line in output.split(|byte| *byte == b'\n') {
        let line = line.trim_ascii();
        if line.is_empty() {
            continue;
        }
        let Ok(Value::Object(event)) = serde_json::from_slice(line) else {
            summary.non_json_lines = summary.non_json_lines.saturating_add(1);
            continue;
        };

        match string(&event, "type") {
            Some("item.completed") => {
                if let Some(item) = event.get("item").and_then(Value::as_object)
                    && string(item, "type") == Some("agent_message")
                {
                    reply = string(item, "text").map(str::to_owned);
                }
            }
            Some("turn.completed") => {
                failed = Some(false);
                summary.api_error = None;
                if let Some(usage) = event.get("usage").and_then(Value::as_object) {
                    let spend = summary.spend.get_or_insert_with(Spend::default);
                    add_usage(spend, usage);
                }
            }
            Some("turn.failed") => {
                failed = Some(true);
                let message = event
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str);
                summary.api_error = Some(category(message).to_owned());
            }
            Some("error") => {
                summary.api_error = Some(category(string(&event, "message")).to_owned());
            }
            _ => {}
        }
    }

    summary.result = failed.map(|is_error| ResultLine {
        is_error: Some(is_error),
        text: reply,
        spend: summary.spend.clone(),
        ..ResultLine::default()
    });
    summary
}

fn add_usage(spend: &mut Spend, usage: &Map<String, Value>) {
    let cache_read = count(usage, "cached_input_tokens");
    let tokens = Tokens {
        input: count(usage, "input_tokens").saturating_sub(cache_read),
        cache_read,
        cache_write: count(usage, "cache_write_input_tokens"),
        output: count(usage, "output_tokens"),
    };
    spend.usage = Some(spend.usage.unwrap_or_default().saturating_add(tokens));
    spend.turns = Some(spend.turns.unwrap_or(0).saturating_add(1));
}

fn category(message: Option<&str>) -> &'static str {
    let status = message.and_then(|message| {
        let value = serde_json::from_str::<Value>(message).ok();
        value
            .as_ref()
            .and_then(|value| value.get("status"))
            .and_then(Value::as_f64)
            .or_else(|| {
                let rest = message.strip_prefix("unexpected status ")?;
                let digits = rest.as_bytes().get(..3)?;
                if !digits.iter().all(u8::is_ascii_digit)
                    || rest.as_bytes().get(3).is_some_and(u8::is_ascii_digit)
                {
                    return None;
                }
                std::str::from_utf8(digits).ok()?.parse().ok()
            })
    });
    match status {
        Some(401.0 | 403.0) => "authentication_failed",
        Some(429.0) => "rate_limit",
        _ => "turn_failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_json_line_count_saturates() {
        assert_eq!(
            summarize_after(b"plain text", usize::MAX).non_json_lines,
            usize::MAX
        );
    }
}
