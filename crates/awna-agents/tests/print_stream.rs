use awna_agents::print::stream::{BACKGROUND_CUT_OFF, Summarizer, summarize};
use awna_agents::{ModelUsage, ResultLine, Spend, Summary, Tokens, Window, Windows};
use serde_json::{Value, json};

const CLEAN: &str = include_str!("fixtures/print-clean.jsonl");
const REFUSED: &[u8] = include_bytes!("fixtures/print-refused.jsonl");
const JSON: &[u8] = include_bytes!("fixtures/print-json.jsonl");
const SESSION: &str = "3f6c1b2e-8d4a-4f0e-9b7c-2a5d6e8f1c3b";
const INIT_AUTO: &str = r#"{"type":"system","subtype":"init","cwd":"/work","session_id":"3f6c1b2e-8d4a-4f0e-9b7c-2a5d6e8f1c3b","permissionMode":"auto"}"#;
const ASSISTANT: &str = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Working."}]},"session_id":"3f6c1b2e-8d4a-4f0e-9b7c-2a5d6e8f1c3b"}"#;
const RESULT_OK: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"Recorded 7.","session_id":"3f6c1b2e-8d4a-4f0e-9b7c-2a5d6e8f1c3b"}"#;
const LIMIT_EVENT: &str = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","rateLimitType":"seven_day","resetsAt":1791504000,"overageStatus":"rejected"},"session_id":"3f6c1b2e-8d4a-4f0e-9b7c-2a5d6e8f1c3b"}"#;
const LIMIT_ASSISTANT: &str = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Weekly limit reached."}]},"error":"rate_limit","is_api_error_message":true,"api_error_status":429,"session_id":"3f6c1b2e-8d4a-4f0e-9b7c-2a5d6e8f1c3b"}"#;
const LIMIT_RESULT: &str = r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":429,"result":"Weekly limit reached.","session_id":"3f6c1b2e-8d4a-4f0e-9b7c-2a5d6e8f1c3b"}"#;
const SERVER_ERROR: &str = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"API Error"}]},"error":"server_error","session_id":"3f6c1b2e-8d4a-4f0e-9b7c-2a5d6e8f1c3b"}"#;
const WINDOWS_EVENT: &str = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","resetsAt":1791806400,"rateLimitType":"seven_day","utilization":0.7,"isUsingOverage":false,"unifiedWindows":{"five_hour":{"utilization":0.32,"resetsAt":1791362400},"seven_day":{"utilization":0.7,"resetsAt":1791806400}}},"uuid":"4f6c1b2e-8d4a-4f0e-9b7c-2a5d6e8f1c3b","session_id":"3f6c1b2e-8d4a-4f0e-9b7c-2a5d6e8f1c3b"}"#;

fn parse(lines: &[&str]) -> Summary {
    summarize(lines.join("\n").as_bytes())
}

fn tokens(input: u64, cache_write: u64, cache_read: u64, output: u64) -> Tokens {
    Tokens {
        input,
        cache_write,
        cache_read,
        output,
    }
}

fn required<T>(value: Option<T>) -> T {
    assert!(value.is_some(), "expected a parsed field");
    value.unwrap()
}

fn expected_result() -> ResultLine {
    ResultLine {
        subtype: Some("success".into()),
        is_error: Some(false),
        text: Some("Recorded 7.".into()),
        session_id: Some(SESSION.into()),
        ..ResultLine::default()
    }
}

fn window(utilization: f64, resets_at: f64) -> Window {
    Window {
        utilization,
        resets_at,
    }
}

fn expected_windows() -> Windows {
    Windows {
        five_hour: Some(window(0.32, 1_791_362_400.0)),
        seven_day: Some(window(0.7, 1_791_806_400.0)),
    }
}

fn models(opus: Option<f64>, sonnet: Option<f64>) -> Vec<ModelUsage> {
    vec![
        ModelUsage {
            model: "opus-5-5".into(),
            tokens: tokens(222, 319_929, 17_170_803, 114_633),
            cost_usd: opus,
        },
        ModelUsage {
            model: "sonnet-5-5".into(),
            tokens: tokens(1034, 332_252, 2_818_497, 69505),
            cost_usd: sonnet,
        },
    ]
}

#[test]
fn a_clean_run_reports_its_mode_and_result_and_no_error() {
    let summary = parse(&[INIT_AUTO, ASSISTANT, RESULT_OK]);
    assert_eq!(summary.permission_mode.as_deref(), Some("auto"));
    assert_eq!(summary.result, Some(expected_result()));
    assert_eq!(summary.api_error, None);
}

#[test]
fn the_first_usable_mode_is_kept_and_a_missing_mode_stays_missing() {
    let default = INIT_AUTO.replace("\"auto\"", "\"default\"");
    let narrow = INIT_AUTO.replace("\"auto\"", "\"dontAsk\"");
    let summary = parse(&[&default, INIT_AUTO, RESULT_OK]);
    assert_eq!(summary.permission_mode.as_deref(), Some("default"));
    assert_eq!(parse(&[RESULT_OK]).permission_mode, None);
    assert_eq!(
        parse(&[&narrow, RESULT_OK]).permission_mode.as_deref(),
        Some("dontAsk")
    );
    assert_eq!(
        parse(&[r#"{"type":"system","subtype":"init"}"#, &narrow, RESULT_OK])
            .permission_mode
            .as_deref(),
        Some("dontAsk")
    );
}

#[test]
fn a_limit_hit_is_kept_with_its_raw_result() {
    let summary = parse(&[INIT_AUTO, LIMIT_EVENT, LIMIT_ASSISTANT, LIMIT_RESULT]);
    assert!(summary.rate_limit.rejected);
    assert_eq!(summary.api_error.as_deref(), Some("rate_limit"));
    let result = required(summary.result);
    assert_eq!(result.subtype.as_deref(), Some("success"));
    assert_eq!(result.is_error, Some(true));

    let result_only = parse(&[INIT_AUTO, LIMIT_RESULT]);
    assert_eq!(result_only.api_error, None);
    assert_eq!(required(result_only.result).is_error, Some(true));
}

#[test]
fn a_rate_limit_event_reports_rejection_without_a_result() {
    let summary = parse(&[INIT_AUTO, LIMIT_EVENT]);
    assert!(summary.rate_limit.rejected);
    assert_eq!(summary.result, None);
    let allowed = LIMIT_EVENT.replacen("\"rejected\"", "\"allowed\"", 1);
    let summary = parse(&[INIT_AUTO, &allowed, RESULT_OK]);
    assert!(!summary.rate_limit.rejected);
    assert!(!summary.rate_limit.warned);
}

#[test]
fn an_assistant_error_and_a_rejection_are_both_kept() {
    let expired = LIMIT_ASSISTANT.replace("rate_limit", "authentication_failed");
    let summary = parse(&[INIT_AUTO, LIMIT_EVENT, &expired, LIMIT_RESULT]);
    assert_eq!(summary.api_error.as_deref(), Some("authentication_failed"));
    assert!(summary.rate_limit.rejected);
}

#[test]
fn a_successful_result_clears_an_assistant_error() {
    let summary = parse(&[INIT_AUTO, ASSISTANT, SERVER_ERROR, ASSISTANT, RESULT_OK]);
    assert_eq!(summary.result, Some(expected_result()));
    assert_eq!(summary.api_error, None);
}

#[test]
fn an_assistant_error_remains_until_a_later_successful_result() {
    let last = parse(&[INIT_AUTO, ASSISTANT, SERVER_ERROR]);
    assert_eq!(last.api_error.as_deref(), Some("server_error"));

    let after = parse(&[INIT_AUTO, RESULT_OK, SERVER_ERROR]);
    assert_eq!(after.api_error.as_deref(), Some("server_error"));
    for result in [
        LIMIT_RESULT.to_owned(),
        RESULT_OK.replace("\"success\"", "\"error_during_execution\""),
    ] {
        let summary = parse(&[INIT_AUTO, SERVER_ERROR, &result]);
        assert_eq!(
            summary.api_error.as_deref(),
            Some("server_error"),
            "{result}"
        );
    }
}

#[test]
fn an_invalid_category_clears_a_prior_error() {
    for raw in ["rate limit", "", "<b>x</b>", "9lives", &"x".repeat(41)] {
        let line = LIMIT_ASSISTANT.replace("\"rate_limit\"", &json!(raw).to_string());
        let summary = parse(&[INIT_AUTO, SERVER_ERROR, &line, LIMIT_RESULT]);
        assert_eq!(summary.permission_mode.as_deref(), Some("auto"));
        assert_eq!(summary.api_error, None, "{raw:?}");
    }
    let odd_mode = INIT_AUTO.replace("\"auto\"", "\"auto mode\"");
    let summary = parse(&[&odd_mode, RESULT_OK]);
    assert_eq!(summary.permission_mode, None);
    let summary = parse(&[&odd_mode, INIT_AUTO, RESULT_OK]);
    assert_eq!(summary.permission_mode.as_deref(), Some("auto"));
}

#[test]
fn only_a_json_boolean_is_kept_as_the_error_flag() {
    for result in [
        r#"{"type":"result","subtype":"success","result":"Done."}"#,
        r#"{"type":"result","subtype":"success","is_error":"false","result":"Done."}"#,
    ] {
        let summary = parse(&[INIT_AUTO, SERVER_ERROR, result]);
        assert_eq!(required(summary.result).is_error, None, "{result}");
        assert_eq!(summary.api_error.as_deref(), Some("server_error"));
    }
}

#[test]
fn a_stream_without_a_result_leaves_it_absent() {
    let summary = parse(&[INIT_AUTO, ASSISTANT]);
    assert_eq!(summary.permission_mode.as_deref(), Some("auto"));
    assert_eq!(summary.result, None);
}

#[test]
fn the_background_warning_is_found_in_plain_text_or_a_system_event() {
    let warning = format!("{BACKGROUND_CUT_OFF} after 600s; terminating");
    let system = json!({"type":"system","subtype":"notice","text":warning}).to_string();
    for line in [&warning, &system] {
        let summary = parse(&[INIT_AUTO, line, RESULT_OK]);
        assert!(summary.background_cut_off);
    }
}

#[test]
fn the_background_warning_quoted_in_conversation_is_ignored() {
    let summary = parse(&[
        INIT_AUTO,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"It prints \"Background tasks still running\"."}]}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"It says Background tasks still running."}]}}"#,
        RESULT_OK,
    ]);
    assert_eq!(summary.result, Some(expected_result()));
    assert!(!summary.background_cut_off);
}

#[test]
fn non_objects_and_broken_lines_are_counted_but_blanks_are_skipped() {
    let noisy = parse(&[
        "Preparing workspace.",
        "{not json",
        r#"["type","result"]"#,
        r#""text""#,
        "  \t\r",
        INIT_AUTO,
        RESULT_OK,
    ]);
    assert_eq!(noisy.result, Some(expected_result()));
    assert_eq!(noisy.non_json_lines, 4);

    let summary = summarize(b" \n\t\r\nBad output.\n");
    assert_eq!(summary.non_json_lines, 1);
}

#[test]
fn invalid_utf8_is_counted_and_can_carry_the_background_warning() {
    let summary = summarize(b"\xff Background tasks still running\r\n");
    assert_eq!(summary.non_json_lines, 1);
    assert!(summary.background_cut_off);
}

#[test]
fn result_text_is_kept_whole_including_blank_text() {
    let full = "é".repeat(4010);
    for expected in [full.as_str(), "  ", ""] {
        let result = RESULT_OK.replace("Recorded 7.", expected);
        assert_eq!(
            required(parse(&[INIT_AUTO, &result]).result)
                .text
                .as_deref(),
            Some(expected)
        );
    }
}

#[test]
fn a_rate_limit_event_gives_both_windows_and_can_warn() {
    let summary = parse(&[INIT_AUTO, WINDOWS_EVENT, RESULT_OK]);
    assert_eq!(summary.rate_limit.windows, Some(expected_windows()));
    assert_eq!(summary.rate_limit.rejected_until, None);
    assert!(summary.rate_limit.warned);
    assert!(!summary.rate_limit.rejected);
}

#[test]
fn the_last_event_with_usable_windows_wins() {
    let later = WINDOWS_EVENT.replace("\"utilization\":0.32", "\"utilization\":0.41");
    let summary = parse(&[WINDOWS_EVENT, &later, LIMIT_EVENT]);
    assert_eq!(
        summary
            .rate_limit
            .windows
            .and_then(|windows| windows.five_hour),
        Some(window(0.41, 1_791_362_400.0))
    );
}

#[test]
fn the_latest_valid_rejected_reset_is_kept() {
    let earlier = LIMIT_EVENT.replace("1791504000", "1791400000");
    for lines in [
        vec![LIMIT_EVENT, LIMIT_ASSISTANT, LIMIT_RESULT],
        vec![LIMIT_EVENT, &earlier],
        vec![&earlier, LIMIT_EVENT],
    ] {
        let summary = parse(&lines);
        assert!(summary.rate_limit.rejected);
        assert_eq!(summary.rate_limit.rejected_until, Some(1_791_504_000.0));
    }
    assert_eq!(parse(&[WINDOWS_EVENT]).rate_limit.rejected_until, None);
}

#[test]
fn malformed_windows_and_resets_are_dropped_and_utilization_is_clamped() {
    let with = |five_hour: &str| {
        let event = format!(
            r#"{{"type":"rate_limit_event","rate_limit_info":{{"status":"allowed","unifiedWindows":{{"five_hour":{five_hour},"seven_day":{{"utilization":0.5,"resetsAt":1791806400}}}}}}}}"#
        );
        parse(&[&event]).rate_limit.windows
    };
    for bad in [
        r#"{"utilization":-0.1,"resetsAt":1791362400}"#,
        r#"{"utilization":"0.3","resetsAt":1791362400}"#,
        r#"{"utilization":0.3,"resetsAt":0}"#,
        r#"{"utilization":0.3,"resetsAt":-5}"#,
        r#"{"utilization":0.3,"resetsAt":1e300}"#,
        r#"{"utilization":0.3,"resetsAt":32503680000}"#,
        r#"{"utilization":0.3}"#,
        r#"{"resetsAt":1791362400}"#,
        "null",
        r#""full""#,
    ] {
        assert_eq!(
            with(bad),
            Some(Windows {
                five_hour: None,
                seven_day: Some(window(0.5, 1_791_806_400.0)),
            }),
            "{bad}"
        );
    }
    assert_eq!(
        with(r#"{"utilization":1.4,"resetsAt":1791362400}"#).and_then(|w| w.five_hour),
        Some(window(1.0, 1_791_362_400.0))
    );
    let empty = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","unifiedWindows":{"five_hour":null}}}"#;
    assert_eq!(parse(&[empty]).rate_limit.windows, None);
    let bad_reset = LIMIT_EVENT.replace("1791504000", "\"Thu\"");
    let summary = parse(&[&bad_reset]);
    assert!(summary.rate_limit.rejected);
    assert_eq!(summary.rate_limit.rejected_until, None);
}

#[test]
fn the_first_windows_are_kept_beside_the_last() {
    let later = WINDOWS_EVENT.replace("\"utilization\":0.32", "\"utilization\":0.41");
    let summary = parse(&[INIT_AUTO, WINDOWS_EVENT, &later, RESULT_OK]);
    assert_eq!(summary.rate_limit.first_windows, Some(expected_windows()));
    assert_eq!(
        summary.rate_limit.windows.and_then(|w| w.five_hour),
        Some(window(0.41, 1_791_362_400.0))
    );
    assert_eq!(parse(&[RESULT_OK]).rate_limit.first_windows, None);
}

#[test]
fn a_finished_run_reports_both_raw_usage_and_usage_by_model() {
    let summary = summarize(CLEAN.as_bytes());
    let spend = Spend {
        usage: Some(tokens(210, 264_826, 0, 0)),
        model_usage: models(Some(8.121_831_599_999_998), Some(8.343_364_249_999_999)),
        has_model_usage: true,
        cost_usd: Some(16.465_195_849_999_997),
        turns: Some(138),
        duration_ms: Some(3_017_501),
    };
    assert_eq!(summary.permission_mode.as_deref(), Some("auto"));
    assert_eq!(summary.rate_limit.windows, Some(expected_windows()));
    assert_eq!(summary.api_error, None);
    assert_eq!(summary.non_json_lines, 0);
    assert_eq!(summary.spend, Some(spend.clone()));
    assert_eq!(
        summary.result,
        Some(ResultLine {
            spend: Some(spend),
            ..expected_result()
        })
    );
}

#[test]
fn a_refused_run_still_reports_what_it_spent() {
    let summary = summarize(REFUSED);
    assert!(summary.rate_limit.rejected);
    assert_eq!(summary.api_error.as_deref(), Some("rate_limit"));
    let spend = required(summary.spend);
    assert_eq!(
        required(spend.model_usage.first()).tokens,
        tokens(902, 220_612, 6_793_258, 38453)
    );
    assert_eq!(spend.cost_usd, Some(5.146_672_75));
    assert_eq!(spend.turns, Some(1));
    assert_eq!(required(summary.result).is_error, Some(true));
}

#[test]
fn a_cut_off_run_reports_its_spend() {
    let result = r#"{"type":"result","subtype":"success","is_error":false,"num_turns":51,"duration_ms":263481,"total_cost_usd":1.9575385999999997,"modelUsage":{"opus-5-5":{"inputTokens":92,"cacheCreationInputTokens":107995,"cacheReadInputTokens":3761253,"outputTokens":17048,"costUSD":1.9575385999999997}}}"#;
    let summary = parse(&[INIT_AUTO, result, "Background tasks still running"]);
    assert!(summary.background_cut_off);
    assert_eq!(
        summary.spend,
        Some(Spend {
            model_usage: vec![ModelUsage {
                model: "opus-5-5".into(),
                tokens: tokens(92, 107_995, 3_761_253, 17048),
                cost_usd: Some(1.957_538_599_999_999_7),
            }],
            has_model_usage: true,
            cost_usd: Some(1.957_538_599_999_999_7),
            turns: Some(51),
            duration_ms: Some(263_481),
            ..Spend::default()
        })
    );
}

#[test]
fn a_run_without_a_result_reports_no_spend() {
    let summary = parse(&[INIT_AUTO, ASSISTANT, WINDOWS_EVENT]);
    assert!(summary.rate_limit.warned);
    assert_eq!(summary.spend, None);
}

#[test]
fn a_run_without_cost_fields_keeps_its_token_counts() {
    let mut events: Vec<Value> = CLEAN
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let result = events.last_mut().unwrap().as_object_mut().unwrap();
    result.remove("total_cost_usd");
    for model in result
        .get_mut("modelUsage")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        model.as_object_mut().unwrap().remove("costUSD");
    }
    let lines: Vec<String> = events.iter().map(Value::to_string).collect();
    let spend = required(summarize(lines.join("\n").as_bytes()).spend);
    assert_eq!(spend.cost_usd, None);
    assert_eq!(spend.model_usage, models(None, None));
    assert_eq!(spend.usage, Some(tokens(210, 264_826, 0, 0)));
}

#[test]
fn a_result_with_usage_and_no_model_usage_reports_its_counts() {
    let line = r#"{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":3,"cache_creation_input_tokens":40,"cache_read_input_tokens":500,"output_tokens":6},"session_id":"3f6c1b2e-8d4a-4f0e-9b7c-2a5d6e8f1c3b"}"#;
    assert_eq!(
        parse(&[line]).spend,
        Some(Spend {
            usage: Some(tokens(3, 40, 500, 6)),
            ..Spend::default()
        })
    );
}

#[test]
fn the_last_result_wins_and_spend_is_kept_separately() {
    let first = r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":1.5,"num_turns":3,"result":"Started."}"#;
    let last = r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":2.5,"num_turns":4,"result":"Finished."}"#;
    let summary = parse(&[first, last]);
    let result = required(summary.result);
    let spend = required(summary.spend);
    assert_eq!(result.text.as_deref(), Some("Finished."));
    assert_eq!(result.spend.as_ref(), Some(&spend));
    assert_eq!((spend.cost_usd, spend.turns), (Some(2.5), Some(4)));

    let summary = parse(&[first, RESULT_OK]);
    assert_eq!(summary.result, Some(expected_result()));
    assert_eq!(required(summary.spend).cost_usd, Some(1.5));
}

#[test]
fn out_of_bounds_spend_values_are_dropped() {
    let line = r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":-1,"num_turns":-2,"duration_ms":1.5,"modelUsage":{"bad model\nid":{"inputTokens":7,"costUSD":1},"opus-5-5":{"inputTokens":1e30,"outputTokens":"9","cacheReadInputTokens":4,"costUSD":1e9}},"session_id":"3f6c1b2e-8d4a-4f0e-9b7c-2a5d6e8f1c3b"}"#;
    assert_eq!(
        parse(&[line]).spend,
        Some(Spend {
            model_usage: vec![ModelUsage {
                model: "opus-5-5".into(),
                tokens: tokens(0, 0, 4, 0),
                cost_usd: None,
            }],
            has_model_usage: true,
            ..Spend::default()
        })
    );
}

#[test]
fn malformed_costs_and_duration_alone_report_no_spend() {
    for cost in ["-1", r#""x""#] {
        let line = format!(r#"{{"type":"result","total_cost_usd":{cost},"duration_ms":1}}"#);
        let summary = parse(&[&line]);
        assert_eq!(required(summary.result).spend, None);
        assert_eq!(summary.spend, None);
    }
}

#[test]
fn model_entries_are_sorted_and_only_objects_are_kept() {
    let ids = [
        "haiku",
        "haiku-4-5",
        "opus",
        "opus-4-5",
        "opus-4-6",
        "opus-5-5",
        "sonnet",
        "sonnet-4-5",
        "sonnet-4-6",
        "sonnet-5-5",
    ];
    let mut entries: Vec<String> = ids
        .iter()
        .rev()
        .map(|id| format!(r#""{id}":{{"inputTokens":1}}"#))
        .collect();
    entries.push(r#""model":2"#.into());
    let line = format!(
        r#"{{"type":"result","subtype":"success","is_error":false,"modelUsage":{{{}}}}}"#,
        entries.join(",")
    );
    let spend = required(parse(&[&line]).spend);
    assert_eq!(
        spend
            .model_usage
            .iter()
            .map(|m| m.model.as_str())
            .collect::<Vec<_>>(),
        ids
    );
    assert!(spend.model_usage.iter().all(|m| m.tokens.input == 1));
}

#[test]
fn empty_usage_objects_still_report_spend() {
    let summary = parse(&[r#"{"type":"result","usage":{},"modelUsage":{}}"#]);
    assert_eq!(
        summary.spend,
        Some(Spend {
            usage: Some(Tokens::default()),
            has_model_usage: true,
            ..Spend::default()
        })
    );
}

#[test]
fn json_output_gives_a_result_with_partial_usage_and_structured_output() {
    let summary = summarize(JSON);
    assert_eq!(
        summary.result,
        Some(ResultLine {
            spend: Some(Spend {
                usage: Some(tokens(12, 0, 0, 8)),
                ..Spend::default()
            }),
            structured_output: Some(json!({"total":7})),
            ..expected_result()
        })
    );
}

#[test]
fn structured_output_keeps_arrays_and_explicit_null() {
    for (line, expected) in [
        (
            r#"{"type":"result","structured_output":[{"total":7},8]}"#,
            json!([{"total":7}, 8]),
        ),
        (r#"{"type":"result","structured_output":null}"#, Value::Null),
    ] {
        assert_eq!(
            required(parse(&[line]).result).structured_output,
            Some(expected)
        );
    }
}

#[test]
fn the_last_results_session_and_status_are_kept_when_valid() {
    let signed_out = r#"{"type":"result","subtype":"success","is_error":true,"result":"Please sign in.","session_id":"4f6c1b2e-8d4a-4f0e-9b7c-2a5d6e8f1c3b"}"#;
    let summary = parse(&[RESULT_OK, "Preparing workspace.", signed_out]);
    assert_eq!(summary.non_json_lines, 1);
    assert_eq!(
        summary.result,
        Some(ResultLine {
            subtype: Some("success".into()),
            is_error: Some(true),
            text: Some("Please sign in.".into()),
            session_id: Some("4f6c1b2e-8d4a-4f0e-9b7c-2a5d6e8f1c3b".into()),
            ..ResultLine::default()
        })
    );
    let no_id = r#"{"type":"result","subtype":"error_max_turns"}"#;
    let result = required(parse(&[RESULT_OK, no_id]).result);
    assert_eq!(result.subtype.as_deref(), Some("error_max_turns"));
    assert_eq!(result.session_id, None);
    assert_eq!(result.is_error, None);
}

#[test]
fn incremental_and_whole_buffer_parsing_agree_with_or_without_crlf() {
    let crlf = CLEAN.replace('\n', "\r\n");
    for stream in [CLEAN.as_bytes(), crlf.as_bytes()] {
        let mut incremental = Summarizer::new();
        for line in stream.split_inclusive(|&byte| byte == b'\n') {
            incremental.push(line);
        }
        let whole = summarize(stream);
        assert_eq!(whole.permission_mode.as_deref(), Some("auto"));
        assert_eq!(incremental.finish(), whole);
    }
}

#[test]
fn invalid_result_identifiers_are_dropped() {
    let result = summarize(br#"{"type":"result","session_id":"../../x","subtype":"a b"}"#)
        .result
        .expect("result line");
    assert_eq!((result.session_id, result.subtype), (None, None));
}
