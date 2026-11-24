use awna_agents::print::{ArgvError, Flags, Output, Session};

const PROMPT: &str = "Read the number.";
const SYSTEM: &str = "You answer in one line.";
const SCHEMA: &str = r#"{"type":"object"}"#;
const NOTE_SCHEMA: &str = r#"{"type":"object","properties":{"body":{"type":"string"}},"required":["body"],"additionalProperties":false}"#;
const PLAN_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"required":["title"],"properties":{"title":{"type":"string"},"labels":{"type":"array","items":{"type":"string"}}}}"#;
const LABEL_SCHEMA: &str =
    r#"{"properties":{"labels":{"type":"array"}},"required":["labels"],"type":"object"}"#;
const EMPTY_MCP: &str = r#"{"mcpServers":{}}"#;
const LEDGER_MCP: &str =
    r#"{"mcpServers":{"ledger":{"type":"http","url":"https://example.test/mcp"}}}"#;
const SETTINGS: &str = r#"{"enabledPlugins":{"docs-helper@market":false},"skillOverrides":{"format-notes":"off"},"syncRemoteSkills":false}"#;
const UUID: &str = "3f6c1b2e-8d4a-4f0e-9b7c-2a5d6e8f1c3b";
const TOOL: &str = "mcp__ledger__get_total";
const OTHER_TOOL: &str = "mcp__ledger__list_entries";

fn groups(args: &[String]) -> Vec<Vec<String>> {
    // The print-mode group can follow an option prefixed by the caller.
    let mut grouped: Vec<Vec<String>> = Vec::new();
    for arg in args {
        if arg.starts_with("--") || arg == "-p" || grouped.is_empty() {
            grouped.push(Vec::new());
        }
        if let Some(group) = grouped.last_mut() {
            // A reference app spells the session options as `--flag=value`; compare that with
            // the kit's two tokens. Every other `=` form stays one token, so it is pinned exactly.
            if let Some((flag, value)) = arg.split_once('=')
                && matches!(flag, "--resume" | "--session-id")
                && !value.is_empty()
            {
                group.extend([flag.to_owned(), value.to_owned()]);
            } else {
                group.push(arg.clone());
            }
        }
    }
    grouped.sort();
    grouped
}

fn assert_argv(flags: Flags, expected: &[&str], reference: &[&str]) {
    let actual = assert_order(flags, expected);
    let reference: Vec<_> = reference.iter().map(|arg| (*arg).to_owned()).collect();
    assert_eq!(groups(&actual), groups(&reference));
}

fn assert_order(flags: Flags, expected: &[&str]) -> Vec<String> {
    let actual = flags.argv().expect("valid arguments");
    assert_eq!(actual, expected);
    let mut end = actual.as_slice();
    for (flag, tools) in [
        ("--allowedTools", &flags.allowed_tools),
        ("--disallowedTools", &flags.disallowed_tools),
    ] {
        if !tools.is_empty() {
            let (prefix, suffix) = end.split_at(end.len() - tools.len() - 1);
            assert_eq!(suffix[0], flag);
            assert_eq!(&suffix[1..], tools);
            end = prefix;
        }
    }
    actual
}

#[test]
fn a_headless_session_with_a_tool_grant() {
    assert_argv(
        Flags {
            prompt: Some(PROMPT.into()),
            model: Some("sonnet-5-5".into()),
            effort: Some("high".into()),
            output: Output::StreamJson,
            setting_sources: Some("project,local".into()),
            strict_mcp_config: true,
            mcp_config: Some(LEDGER_MCP.into()),
            permission_mode: Some("dontAsk".into()),
            allowed_tools: vec![TOOL.into(), OTHER_TOOL.into()],
            ..Flags::default()
        },
        &[
            "-p",
            PROMPT,
            "--model",
            "sonnet-5-5",
            "--effort",
            "high",
            "--output-format",
            "stream-json",
            "--verbose",
            "--setting-sources",
            "project,local",
            "--strict-mcp-config",
            "--mcp-config",
            LEDGER_MCP,
            "--permission-mode",
            "dontAsk",
            "--allowedTools",
            TOOL,
            OTHER_TOOL,
        ],
        &[
            "-p",
            PROMPT,
            "--setting-sources",
            "project,local",
            "--strict-mcp-config",
            "--mcp-config",
            LEDGER_MCP,
            "--model",
            "sonnet-5-5",
            "--effort",
            "high",
            "--permission-mode",
            "dontAsk",
            "--output-format",
            "stream-json",
            "--verbose",
            "--allowedTools",
            TOOL,
            OTHER_TOOL,
        ],
    );
}

#[test]
fn a_headless_session_with_user_settings() {
    assert_argv(
        Flags {
            prompt: Some(PROMPT.into()),
            model: Some("sonnet-5-5".into()),
            effort: Some("high".into()),
            output: Output::StreamJson,
            setting_sources: Some("user,project,local".into()),
            strict_mcp_config: true,
            mcp_config: Some(LEDGER_MCP.into()),
            permission_mode: Some("auto".into()),
            allowed_tools: vec![TOOL.into(), OTHER_TOOL.into()],
            settings: Some(SETTINGS.into()),
            ..Flags::default()
        },
        &[
            "-p",
            PROMPT,
            "--model",
            "sonnet-5-5",
            "--effort",
            "high",
            "--output-format",
            "stream-json",
            "--verbose",
            "--setting-sources",
            "user,project,local",
            "--settings",
            SETTINGS,
            "--strict-mcp-config",
            "--mcp-config",
            LEDGER_MCP,
            "--permission-mode",
            "auto",
            "--allowedTools",
            TOOL,
            OTHER_TOOL,
        ],
        &[
            "-p",
            PROMPT,
            "--setting-sources",
            "user,project,local",
            "--strict-mcp-config",
            "--mcp-config",
            LEDGER_MCP,
            "--settings",
            SETTINGS,
            "--model",
            "sonnet-5-5",
            "--effort",
            "high",
            "--permission-mode",
            "auto",
            "--output-format",
            "stream-json",
            "--verbose",
            "--allowedTools",
            TOOL,
            OTHER_TOOL,
        ],
    );
}

#[test]
fn a_headless_session_with_stdin() {
    assert_argv(
        Flags {
            model: Some("sonnet-5-5".into()),
            effort: Some("high".into()),
            output: Output::StreamJson,
            setting_sources: Some("user,project,local".into()),
            strict_mcp_config: true,
            mcp_config: Some(LEDGER_MCP.into()),
            permission_mode: Some("auto".into()),
            allowed_tools: vec![TOOL.into(), OTHER_TOOL.into()],
            settings: Some(SETTINGS.into()),
            ..Flags::default()
        },
        &[
            "-p",
            "--model",
            "sonnet-5-5",
            "--effort",
            "high",
            "--output-format",
            "stream-json",
            "--verbose",
            "--setting-sources",
            "user,project,local",
            "--settings",
            SETTINGS,
            "--strict-mcp-config",
            "--mcp-config",
            LEDGER_MCP,
            "--permission-mode",
            "auto",
            "--allowedTools",
            TOOL,
            OTHER_TOOL,
        ],
        &[
            "-p",
            "--setting-sources",
            "user,project,local",
            "--strict-mcp-config",
            "--mcp-config",
            LEDGER_MCP,
            "--settings",
            SETTINGS,
            "--model",
            "sonnet-5-5",
            "--effort",
            "high",
            "--permission-mode",
            "auto",
            "--output-format",
            "stream-json",
            "--verbose",
            "--allowedTools",
            TOOL,
            OTHER_TOOL,
        ],
    );
}

#[test]
fn a_tool_less_structured_call() {
    assert_argv(
        Flags {
            system_prompt: Some(SYSTEM.into()),
            model: Some("sonnet".into()),
            safe_mode: true,
            no_session_persistence: true,
            tools: Some("".into()),
            output: Output::Json,
            json_schema: Some(SCHEMA.into()),
            setting_sources: Some("project,local".into()),
            strict_mcp_config: true,
            mcp_config: Some(EMPTY_MCP.into()),
            permission_mode: Some("dontAsk".into()),
            ..Flags::default()
        },
        &[
            "-p",
            "--system-prompt",
            SYSTEM,
            "--model",
            "sonnet",
            "--safe-mode",
            "--no-session-persistence",
            "--tools",
            "",
            "--output-format",
            "json",
            "--json-schema",
            SCHEMA,
            "--setting-sources",
            "project,local",
            "--strict-mcp-config",
            "--mcp-config",
            EMPTY_MCP,
            "--permission-mode",
            "dontAsk",
        ],
        &[
            "-p",
            "--system-prompt",
            SYSTEM,
            "--model",
            "sonnet",
            "--safe-mode",
            "--no-session-persistence",
            "--tools",
            "",
            "--output-format",
            "json",
            "--json-schema",
            SCHEMA,
            "--setting-sources",
            "project,local",
            "--strict-mcp-config",
            "--mcp-config",
            EMPTY_MCP,
            "--permission-mode",
            "dontAsk",
        ],
    );
}

#[test]
fn a_tool_less_call_without_a_schema() {
    assert_argv(
        Flags {
            system_prompt: Some(SYSTEM.into()),
            model: Some("sonnet".into()),
            safe_mode: true,
            no_session_persistence: true,
            tools: Some("".into()),
            output: Output::Json,
            setting_sources: Some("project,local".into()),
            strict_mcp_config: true,
            mcp_config: Some(EMPTY_MCP.into()),
            permission_mode: Some("dontAsk".into()),
            ..Flags::default()
        },
        &[
            "-p",
            "--system-prompt",
            SYSTEM,
            "--model",
            "sonnet",
            "--safe-mode",
            "--no-session-persistence",
            "--tools",
            "",
            "--output-format",
            "json",
            "--setting-sources",
            "project,local",
            "--strict-mcp-config",
            "--mcp-config",
            EMPTY_MCP,
            "--permission-mode",
            "dontAsk",
        ],
        &[
            "-p",
            "--system-prompt",
            SYSTEM,
            "--model",
            "sonnet",
            "--safe-mode",
            "--no-session-persistence",
            "--tools",
            "",
            "--output-format",
            "json",
            "--setting-sources",
            "project,local",
            "--strict-mcp-config",
            "--mcp-config",
            EMPTY_MCP,
            "--permission-mode",
            "dontAsk",
        ],
    );
}

#[test]
fn a_tool_less_call_with_a_positional_prompt() {
    assert_argv(
        Flags {
            prompt: Some(PROMPT.into()),
            system_prompt: Some(SYSTEM.into()),
            model: Some("sonnet".into()),
            safe_mode: true,
            no_session_persistence: true,
            tools: Some("".into()),
            output: Output::Json,
            json_schema: Some(NOTE_SCHEMA.into()),
            setting_sources: Some("project,local".into()),
            strict_mcp_config: true,
            mcp_config: Some(EMPTY_MCP.into()),
            permission_mode: Some("dontAsk".into()),
            ..Flags::default()
        },
        &[
            "-p",
            PROMPT,
            "--system-prompt",
            SYSTEM,
            "--model",
            "sonnet",
            "--safe-mode",
            "--no-session-persistence",
            "--tools",
            "",
            "--output-format",
            "json",
            "--json-schema",
            NOTE_SCHEMA,
            "--setting-sources",
            "project,local",
            "--strict-mcp-config",
            "--mcp-config",
            EMPTY_MCP,
            "--permission-mode",
            "dontAsk",
        ],
        &[
            "-p",
            PROMPT,
            "--system-prompt",
            SYSTEM,
            "--model",
            "sonnet",
            "--safe-mode",
            "--no-session-persistence",
            "--tools",
            "",
            "--output-format",
            "json",
            "--json-schema",
            NOTE_SCHEMA,
            "--setting-sources",
            "project,local",
            "--strict-mcp-config",
            "--mcp-config",
            EMPTY_MCP,
            "--permission-mode",
            "dontAsk",
        ],
    );
}

#[test]
fn a_new_planning_session() {
    assert_argv(
        Flags {
            prompt: Some(PROMPT.into()),
            model: Some("sonnet".into()),
            restricted: true,
            tools: Some("Read,Grep,Glob,Bash".into()),
            output: Output::Json,
            json_schema: Some(PLAN_SCHEMA.into()),
            strict_mcp_config: true,
            permission_mode: Some("dontAsk".into()),
            allowed_tools: vec![
                "Bash(make check:*)".into(),
                "Bash(ls:*)".into(),
                "Read".into(),
            ],
            session: Session::Id(UUID.into()),
            ..Flags::default()
        },
        &[
            "-p",
            PROMPT,
            "--model",
            "sonnet",
            "--restricted",
            "--tools",
            "Read,Grep,Glob,Bash",
            "--output-format",
            "json",
            "--json-schema",
            PLAN_SCHEMA,
            "--strict-mcp-config",
            "--permission-mode",
            "dontAsk",
            "--session-id",
            UUID,
            "--allowedTools",
            "Bash(make check:*)",
            "Bash(ls:*)",
            "Read",
        ],
        &[
            "-p",
            PROMPT,
            "--output-format",
            "json",
            "--json-schema",
            PLAN_SCHEMA,
            "--model",
            "sonnet",
            "--session-id",
            UUID,
            "--restricted",
            "--strict-mcp-config",
            "--permission-mode",
            "dontAsk",
            "--tools",
            "Read,Grep,Glob,Bash",
            "--allowedTools",
            "Bash(make check:*)",
            "Bash(ls:*)",
            "Read",
        ],
    );
}

#[test]
fn a_resumed_session() {
    assert_argv(
        Flags {
            prompt: Some(PROMPT.into()),
            model: Some("sonnet".into()),
            restricted: true,
            tools: Some("Read,Grep,Glob,Bash".into()),
            output: Output::Json,
            json_schema: Some(PLAN_SCHEMA.into()),
            strict_mcp_config: true,
            permission_mode: Some("dontAsk".into()),
            allowed_tools: vec![
                "Bash(make check:*)".into(),
                "Bash(ls:*)".into(),
                "Read".into(),
            ],
            session: Session::Resume(UUID.into()),
            ..Flags::default()
        },
        &[
            "-p",
            PROMPT,
            "--model",
            "sonnet",
            "--restricted",
            "--tools",
            "Read,Grep,Glob,Bash",
            "--output-format",
            "json",
            "--json-schema",
            PLAN_SCHEMA,
            "--strict-mcp-config",
            "--permission-mode",
            "dontAsk",
            "--resume",
            UUID,
            "--allowedTools",
            "Bash(make check:*)",
            "Bash(ls:*)",
            "Read",
        ],
        &[
            "-p",
            PROMPT,
            "--output-format",
            "json",
            "--json-schema",
            PLAN_SCHEMA,
            "--model",
            "sonnet",
            "--resume",
            UUID,
            "--restricted",
            "--strict-mcp-config",
            "--permission-mode",
            "dontAsk",
            "--tools",
            "Read,Grep,Glob,Bash",
            "--allowedTools",
            "Bash(make check:*)",
            "Bash(ls:*)",
            "Read",
        ],
    );
}

#[test]
fn a_structured_stream_with_a_budget() {
    assert_argv(
        Flags {
            system_prompt: Some(SYSTEM.into()),
            model: Some("sonnet".into()),
            safe_mode: true,
            no_session_persistence: true,
            tools: Some("".into()),
            output: Output::StreamJson,
            json_schema: Some(LABEL_SCHEMA.into()),
            setting_sources: Some("project,local".into()),
            strict_mcp_config: true,
            mcp_config: Some(EMPTY_MCP.into()),
            permission_mode: Some("dontAsk".into()),
            max_budget_usd: Some(0.3),
            ..Flags::default()
        },
        &[
            "-p",
            "--system-prompt",
            SYSTEM,
            "--model",
            "sonnet",
            "--safe-mode",
            "--no-session-persistence",
            "--tools",
            "",
            "--output-format",
            "stream-json",
            "--verbose",
            "--json-schema",
            LABEL_SCHEMA,
            "--setting-sources",
            "project,local",
            "--strict-mcp-config",
            "--mcp-config",
            EMPTY_MCP,
            "--permission-mode",
            "dontAsk",
            "--max-budget-usd",
            "0.30",
        ],
        &[
            "--setting-sources",
            "project,local",
            "-p",
            "--system-prompt",
            SYSTEM,
            "--model",
            "sonnet",
            "--safe-mode",
            "--no-session-persistence",
            "--tools",
            "",
            "--output-format",
            "stream-json",
            "--verbose",
            "--json-schema",
            LABEL_SCHEMA,
            "--strict-mcp-config",
            "--mcp-config",
            EMPTY_MCP,
            "--permission-mode",
            "dontAsk",
            "--max-budget-usd",
            "0.30",
        ],
    );
}

#[test]
fn a_structured_stream_without_a_budget() {
    assert_argv(
        Flags {
            system_prompt: Some(SYSTEM.into()),
            model: Some("sonnet".into()),
            safe_mode: true,
            no_session_persistence: true,
            tools: Some("".into()),
            output: Output::StreamJson,
            json_schema: Some(LABEL_SCHEMA.into()),
            setting_sources: Some("project,local".into()),
            strict_mcp_config: true,
            mcp_config: Some(EMPTY_MCP.into()),
            permission_mode: Some("dontAsk".into()),
            ..Flags::default()
        },
        &[
            "-p",
            "--system-prompt",
            SYSTEM,
            "--model",
            "sonnet",
            "--safe-mode",
            "--no-session-persistence",
            "--tools",
            "",
            "--output-format",
            "stream-json",
            "--verbose",
            "--json-schema",
            LABEL_SCHEMA,
            "--setting-sources",
            "project,local",
            "--strict-mcp-config",
            "--mcp-config",
            EMPTY_MCP,
            "--permission-mode",
            "dontAsk",
        ],
        &[
            "--setting-sources",
            "project,local",
            "-p",
            "--system-prompt",
            SYSTEM,
            "--model",
            "sonnet",
            "--safe-mode",
            "--no-session-persistence",
            "--tools",
            "",
            "--output-format",
            "stream-json",
            "--verbose",
            "--json-schema",
            LABEL_SCHEMA,
            "--strict-mcp-config",
            "--mcp-config",
            EMPTY_MCP,
            "--permission-mode",
            "dontAsk",
        ],
    );
}

#[test]
fn a_fixed_toolset_chat_turn() {
    assert_argv(
        Flags {
            system_prompt: Some(SYSTEM.into()),
            model: Some("haiku".into()),
            tools: Some("".into()),
            output: Output::StreamJson,
            setting_sources: Some("".into()),
            strict_mcp_config: true,
            mcp_config: Some("{}".into()),
            permission_mode: Some("default".into()),
            permission_prompts: Some("none".into()),
            max_turns: Some(12),
            disallowed_tools: vec!["Read,Write,Edit,Bash".into()],
            allowed_tools: vec![TOOL.into()],
            ..Flags::default()
        },
        &[
            "-p",
            "--system-prompt",
            SYSTEM,
            "--model",
            "haiku",
            "--tools",
            "",
            "--output-format",
            "stream-json",
            "--verbose",
            "--setting-sources=",
            "--strict-mcp-config",
            "--mcp-config",
            "{}",
            "--permission-mode",
            "default",
            "--permission-prompts",
            "none",
            "--max-turns",
            "12",
            "--disallowedTools",
            "Read,Write,Edit,Bash",
            "--allowedTools",
            TOOL,
        ],
        &[
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--system-prompt",
            SYSTEM,
            "--model",
            "haiku",
            "--max-turns",
            "12",
            "--mcp-config",
            "{}",
            "--strict-mcp-config",
            "--setting-sources=",
            "--tools",
            "",
            "--allowedTools",
            TOOL,
            "--disallowedTools",
            "Read,Write,Edit,Bash",
            "--permission-mode",
            "default",
            "--permission-prompts",
            "none",
        ],
    );
}

#[test]
fn a_resumed_fixed_toolset_chat_turn() {
    assert_argv(
        Flags {
            system_prompt: Some(SYSTEM.into()),
            model: Some("haiku".into()),
            tools: Some("".into()),
            output: Output::StreamJson,
            setting_sources: Some("".into()),
            strict_mcp_config: true,
            mcp_config: Some("{}".into()),
            permission_mode: Some("default".into()),
            permission_prompts: Some("none".into()),
            max_turns: Some(12),
            session: Session::Resume(UUID.into()),
            disallowed_tools: vec!["Read,Write,Edit,Bash".into()],
            allowed_tools: vec![TOOL.into()],
            ..Flags::default()
        },
        &[
            "-p",
            "--system-prompt",
            SYSTEM,
            "--model",
            "haiku",
            "--tools",
            "",
            "--output-format",
            "stream-json",
            "--verbose",
            "--setting-sources=",
            "--strict-mcp-config",
            "--mcp-config",
            "{}",
            "--permission-mode",
            "default",
            "--permission-prompts",
            "none",
            "--max-turns",
            "12",
            "--resume",
            UUID,
            "--disallowedTools",
            "Read,Write,Edit,Bash",
            "--allowedTools",
            TOOL,
        ],
        &[
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--system-prompt",
            SYSTEM,
            "--model",
            "haiku",
            "--max-turns",
            "12",
            "--mcp-config",
            "{}",
            "--strict-mcp-config",
            "--setting-sources=",
            "--tools",
            "",
            "--allowedTools",
            TOOL,
            "--disallowedTools",
            "Read,Write,Edit,Bash",
            "--permission-mode",
            "default",
            "--permission-prompts",
            "none",
            &format!("--resume={UUID}"),
        ],
    );
}

#[test]
fn an_ephemeral_chat_turn() {
    assert_argv(
        Flags {
            system_prompt: Some("Summarize the note.".into()),
            model: Some("haiku".into()),
            tools: Some("".into()),
            output: Output::StreamJson,
            setting_sources: Some("".into()),
            strict_mcp_config: true,
            mcp_config: Some("{}".into()),
            permission_mode: Some("default".into()),
            permission_prompts: Some("none".into()),
            max_turns: Some(1),
            disallowed_tools: vec!["Read,Write,Edit,Bash".into()],
            no_session_persistence: true,
            ..Flags::default()
        },
        &[
            "-p",
            "--system-prompt",
            "Summarize the note.",
            "--model",
            "haiku",
            "--no-session-persistence",
            "--tools",
            "",
            "--output-format",
            "stream-json",
            "--verbose",
            "--setting-sources=",
            "--strict-mcp-config",
            "--mcp-config",
            "{}",
            "--permission-mode",
            "default",
            "--permission-prompts",
            "none",
            "--max-turns",
            "1",
            "--disallowedTools",
            "Read,Write,Edit,Bash",
        ],
        &[
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--no-session-persistence",
            "--system-prompt",
            "Summarize the note.",
            "--model",
            "haiku",
            "--max-turns",
            "1",
            "--mcp-config",
            "{}",
            "--strict-mcp-config",
            "--setting-sources=",
            "--tools",
            "",
            "--disallowedTools",
            "Read,Write,Edit,Bash",
            "--permission-mode",
            "default",
            "--permission-prompts",
            "none",
        ],
    );
}

#[test]
fn all_options_are_ordered_in_one_call() {
    assert_order(
        Flags {
            system_prompt: Some(SYSTEM.into()),
            model: Some("opus-5-5".into()),
            tools: Some("".into()),
            output: Output::StreamJson,
            setting_sources: Some("".into()),
            strict_mcp_config: true,
            mcp_config: Some("{}".into()),
            permission_mode: Some("default".into()),
            permission_prompts: Some("none".into()),
            max_turns: Some(12),
            disallowed_tools: vec!["Read".into(), "Write".into()],
            allowed_tools: vec![TOOL.into(), OTHER_TOOL.into()],
            prompt: Some(PROMPT.into()),
            effort: Some("high".into()),
            safe_mode: true,
            restricted: true,
            no_session_persistence: true,
            json_schema: Some(SCHEMA.into()),
            settings: Some(SETTINGS.into()),
            max_budget_usd: Some(0.3),
            session: Session::Id(UUID.into()),
        },
        &[
            "-p",
            PROMPT,
            "--system-prompt",
            SYSTEM,
            "--model",
            "opus-5-5",
            "--effort",
            "high",
            "--safe-mode",
            "--restricted",
            "--no-session-persistence",
            "--tools",
            "",
            "--output-format",
            "stream-json",
            "--verbose",
            "--json-schema",
            SCHEMA,
            "--setting-sources=",
            "--settings",
            SETTINGS,
            "--strict-mcp-config",
            "--mcp-config",
            "{}",
            "--permission-mode",
            "default",
            "--permission-prompts",
            "none",
            "--max-turns",
            "12",
            "--max-budget-usd",
            "0.30",
            "--session-id",
            UUID,
            "--disallowedTools",
            "Read",
            "Write",
            "--allowedTools",
            TOOL,
            OTHER_TOOL,
        ],
    );
}

#[test]
fn defaults_only_enable_print_mode() {
    assert_argv(Flags::default(), &["-p"], &["-p"]);
}

#[test]
fn a_positional_prompt_cannot_look_like_an_option() {
    assert_eq!(
        Flags {
            prompt: Some("-x".into()),
            ..Flags::default()
        }
        .argv(),
        Err(ArgvError::PromptLooksLikeAFlag)
    );
}

#[test]
fn a_session_id_has_uuid_syntax() {
    for session in [
        Session::Id("not-a-uuid".into()),
        Session::Resume("3f6c1b2e-8d4a-4f0e-9b7c-2a5d6e8f1c3z".into()),
    ] {
        assert_eq!(
            Flags {
                session,
                ..Flags::default()
            }
            .argv(),
            Err(ArgvError::SessionNotUuid)
        );
    }
}

#[test]
fn tool_list_entries_refuse_empty_values_flags_and_controls() {
    for entry in ["", "-x", "Read\nWrite", "Read\tWrite"] {
        for flags in [
            Flags {
                allowed_tools: vec![entry.into()],
                ..Flags::default()
            },
            Flags {
                disallowed_tools: vec![entry.into()],
                ..Flags::default()
            },
        ] {
            assert_eq!(flags.argv(), Err(ArgvError::ToolLooksLikeAFlag));
        }
    }
}

#[test]
fn a_budget_must_be_finite_and_nonnegative() {
    for budget in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.3, -0.0] {
        assert_eq!(
            Flags {
                max_budget_usd: Some(budget),
                ..Flags::default()
            }
            .argv(),
            Err(ArgvError::BudgetNotFinite)
        );
    }
}

#[test]
fn hyphen_prefixed_option_values_share_the_flag_token() {
    assert_order(
        Flags {
            system_prompt: Some("- answer tersely".into()),
            model: Some("--x".into()),
            ..Flags::default()
        },
        &["-p", "--system-prompt=- answer tersely", "--model=--x"],
    );
}

#[test]
fn nul_is_rejected_before_other_argument_errors() {
    let value = "-x\0";
    let some = || Some(value.to_owned());
    let defaults = Flags::default();
    let string_options: [fn(&mut Flags, Option<String>); 11] = [
        |flags, value| flags.prompt = value,
        |flags, value| flags.system_prompt = value,
        |flags, value| flags.model = value,
        |flags, value| flags.effort = value,
        |flags, value| flags.tools = value,
        |flags, value| flags.json_schema = value,
        |flags, value| flags.setting_sources = value,
        |flags, value| flags.settings = value,
        |flags, value| flags.mcp_config = value,
        |flags, value| flags.permission_mode = value,
        |flags, value| flags.permission_prompts = value,
    ];
    let mut cases: Vec<Flags> = string_options
        .iter()
        .map(|set| {
            let mut flags = defaults.clone();
            set(&mut flags, some());
            flags
        })
        .collect();
    cases.extend([
        Flags {
            session: Session::Id(value.into()),
            ..defaults.clone()
        },
        Flags {
            session: Session::Resume(value.into()),
            ..defaults.clone()
        },
        Flags {
            disallowed_tools: vec![value.into()],
            ..defaults.clone()
        },
        Flags {
            allowed_tools: vec![value.into()],
            ..defaults
        },
    ]);
    for flags in cases {
        let error = flags.argv().expect_err("NUL argument");
        assert_eq!(
            error.to_string(),
            "A value contains a NUL character.",
            "{flags:?}"
        );
        assert_eq!(error, ArgvError::ContainsNul, "{flags:?}");
    }
}
