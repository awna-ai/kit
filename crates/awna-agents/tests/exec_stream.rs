use awna_agents::exec::summarize;
use awna_agents::{RateLimit, ResultLine, Spend, Summary, TOKENS_MAX, Tokens};

const TWO_TURNS: &[u8] = include_bytes!("fixtures/exec-two-turns.jsonl");

#[test]
fn a_completed_turn_reports_its_reply_and_uncached_input() {
    let summary = summarize(
        br#"{"type":"item.completed","item":{"type":"agent_message","text":"Recorded 7."}}
{"type":"turn.completed","usage":{"input_tokens":33702,"cached_input_tokens":12288,"output_tokens":5}}"#,
    );
    let spend = Spend {
        usage: Some(Tokens {
            input: 21414,
            cache_read: 12288,
            cache_write: 0,
            output: 5,
        }),
        turns: Some(1),
        ..Spend::default()
    };
    assert_eq!(
        summary,
        Summary {
            result: Some(ResultLine {
                is_error: Some(false),
                text: Some("Recorded 7.".into()),
                spend: Some(spend.clone()),
                ..ResultLine::default()
            }),
            spend: Some(spend),
            ..Summary::default()
        }
    );
}

#[test]
fn a_model_failure_keeps_only_its_category() {
    let summary = summarize(
        br#"{"type":"error","message":"The selected model is missing."}
{"type":"turn.failed","error":{"message":"The selected model is missing."}}"#,
    );
    assert_eq!(
        summary,
        Summary {
            result: Some(ResultLine {
                is_error: Some(true),
                ..ResultLine::default()
            }),
            api_error: Some("turn_failed".into()),
            ..Summary::default()
        }
    );
}

#[test]
fn an_unauthorized_stream_counts_non_objects() {
    let summary = summarize(
        b"\n \r\nConnecting.\n[]\n\xff\n{\"type\":\"error\",\"message\":\"unexpected status 401 Unauthorized\"}\r\n",
    );
    assert_eq!(summary.api_error.as_deref(), Some("authentication_failed"));
    assert_eq!(summary.non_json_lines, 3);
    assert_eq!(summary.result, None);
}

#[test]
fn reconnection_can_recover_and_errors_have_fixed_categories() {
    let recovered = summarize(
        br#"{"type":"error","message":"Reconnecting... 1/5"}
{"type":"item.completed","item":{"type":"agent_message","text":"Recorded 7."}}
{"type":"item.completed","item":{"type":"error","message":"Notice."}}
{"type":"turn.completed"}"#,
    );
    assert_eq!(
        recovered
            .result
            .as_ref()
            .and_then(|result| result.text.as_deref()),
        Some("Recorded 7.")
    );
    assert_eq!(recovered.api_error, None);
    assert_eq!(recovered.spend, None);

    let limited = summarize(
        br#"{"type":"turn.failed","error":{"message":"{\"status\":429,\"detail\":\"Try again later.\"}"}}"#,
    );
    assert_eq!(limited.api_error.as_deref(), Some("rate_limit"));
    let forbidden = summarize(br#"{"type":"error","message":"unexpected status 403 Forbidden"}"#);
    assert_eq!(
        forbidden.api_error.as_deref(),
        Some("authentication_failed")
    );
    assert_eq!(forbidden.result, None);
}

#[test]
fn usage_accumulates_across_turns_and_the_last_reply_wins() {
    let summary = summarize(TWO_TURNS);
    assert_eq!(
        summary.spend.as_ref().and_then(|spend| spend.usage),
        Some(Tokens {
            input: 10,
            cache_read: 6,
            cache_write: 4,
            output: 8,
        })
    );
    assert_eq!(
        summary.spend.as_ref().and_then(|spend| spend.turns),
        Some(2)
    );
    let result = summary.result.unwrap();
    assert_eq!(result.text.as_deref(), Some("Recorded 7."));
    assert_eq!(result.spend, summary.spend);
    assert_eq!(summary.rate_limit, RateLimit::default());
    assert_eq!(summary.permission_mode, None);
    assert_eq!(
        summarize(br#"{"type":"thread.started"}"#),
        Summary::default()
    );
}

#[test]
fn out_of_range_counts_read_zero_and_reply_text_is_whole() {
    let text = "é".repeat(4010);
    let stream = format!(
        r#"{{"type":"item.completed","item":{{"type":"agent_message","text":"{text}"}}}}
{{"type":"turn.completed","usage":{{"input_tokens":{},"cached_input_tokens":-1,"cache_write_input_tokens":"3","output_tokens":null}}}}"#,
        TOKENS_MAX + 1
    );
    let summary = summarize(stream.as_bytes());
    assert_eq!(
        summary
            .result
            .as_ref()
            .and_then(|result| result.text.as_deref()),
        Some(text.as_str())
    );
    assert_eq!(
        summary.spend.as_ref().and_then(|spend| spend.usage),
        Some(Tokens::default())
    );
}

#[test]
fn cached_input_can_exceed_reported_input() {
    let summary = summarize(
        br#"{"type":"turn.completed","usage":{"input_tokens":1,"cached_input_tokens":5}}"#,
    );
    assert_eq!(
        summary.spend.as_ref().and_then(|spend| spend.usage),
        Some(Tokens {
            cache_read: 5,
            ..Tokens::default()
        })
    );
}

#[test]
fn the_last_turn_determines_result_and_later_errors_are_retained() {
    let completed = r#"{"type":"turn.completed"}"#;
    let failed = r#"{"type":"turn.failed","error":{"message":"Try again."}}"#;
    let recovered = summarize(format!("{failed}\n{completed}").as_bytes());
    assert_eq!(
        recovered,
        Summary {
            result: Some(ResultLine {
                is_error: Some(false),
                ..ResultLine::default()
            }),
            ..Summary::default()
        }
    );
    let last_failed = summarize(format!("{completed}\n{failed}").as_bytes());
    assert_eq!(last_failed.result.unwrap().is_error, Some(true));
    assert_eq!(last_failed.api_error.as_deref(), Some("turn_failed"));

    let error = r#"{"type":"error","message":"unexpected status 429 Too Many Requests"}"#;
    let later_error = summarize(format!("{completed}\n{error}").as_bytes());
    assert_eq!(later_error.result.unwrap().is_error, Some(false));
    assert_eq!(later_error.api_error.as_deref(), Some("rate_limit"));
}

#[test]
fn status_accepts_json_decimal_but_requires_a_number() {
    let decimal = summarize(br#"{"type":"turn.failed","error":{"message":"{\"status\":401.0}"}}"#);
    assert_eq!(decimal.api_error.as_deref(), Some("authentication_failed"));
    for stream in [
        br#"{"type":"error","message":"{\"status\":\"401\"}"}"#.as_slice(),
        br#"{"type":"error","message":"unexpected status 42x"}"#.as_slice(),
        r#"{"type":"error","message":"unexpected status 4é"}"#.as_bytes(),
    ] {
        assert_eq!(summarize(stream).api_error.as_deref(), Some("turn_failed"));
    }
}

#[test]
fn a_text_status_requires_exactly_three_digits() {
    for (message, expected) in [
        ("unexpected status 4011", "turn_failed"),
        (
            "unexpected status 401 Unauthorized",
            "authentication_failed",
        ),
        ("unexpected status 401", "authentication_failed"),
    ] {
        let line = serde_json::json!({"type": "error", "message": message}).to_string();
        assert_eq!(
            summarize(line.as_bytes()).api_error.as_deref(),
            Some(expected)
        );
    }
}
