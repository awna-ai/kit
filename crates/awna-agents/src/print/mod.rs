//! Arguments for a print-mode CLI: one non-interactive call (`-p`) whose output is text, a
//! JSON result, or a stream of JSON events (`stream-json`).

use std::fmt;

use crate::is_uuid;

/// Incremental and whole-stream parsing.
pub mod stream;

/// Options for a print-mode call.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Flags {
    /// Positional prompt. Omit it to supply the prompt on standard input.
    pub prompt: Option<String>,
    /// System prompt for the call.
    pub system_prompt: Option<String>,
    /// Model identifier or alias.
    pub model: Option<String>,
    /// Effort level for the call.
    pub effort: Option<String>,
    /// Enable safe mode.
    pub safe_mode: bool,
    /// Enable restricted mode.
    pub restricted: bool,
    /// Disable session persistence.
    pub no_session_persistence: bool,
    /// Built-in tools to load. An empty string disables them.
    pub tools: Option<String>,
    /// Output format for the call.
    pub output: Output,
    /// JSON schema for structured output.
    pub json_schema: Option<String>,
    /// Setting sources to load. An empty string disables them.
    pub setting_sources: Option<String>,
    /// JSON settings to apply to the call.
    pub settings: Option<String>,
    /// Load MCP servers only from the supplied configuration.
    pub strict_mcp_config: bool,
    /// JSON configuration for MCP servers.
    pub mcp_config: Option<String>,
    /// Permission mode for the call.
    pub permission_mode: Option<String>,
    /// Permission prompt behavior for the call.
    pub permission_prompts: Option<String>,
    /// Maximum number of turns.
    pub max_turns: Option<u32>,
    /// Maximum budget in dollars, formatted to two decimal places.
    pub max_budget_usd: Option<f64>,
    /// Session selection for the call.
    pub session: Session,
    /// Tools to disallow, each supplied as a separate argument. The CLI splits entries on commas
    /// and spaces, so one entry can name several tools. Never build entries from untrusted text.
    pub disallowed_tools: Vec<String>,
    /// Tools to allow, each supplied as a separate argument. The CLI splits entries on commas
    /// and spaces, so one entry can name several tools. Never build entries from untrusted text.
    pub allowed_tools: Vec<String>,
}

/// Output formats for a print-mode call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Output {
    /// Use the default text output.
    #[default]
    Text,
    /// Return a single JSON result.
    Json,
    /// Stream JSON events with verbose output.
    StreamJson,
}

/// Session selection for a print-mode call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Session {
    /// Start a session with an automatically assigned ID.
    #[default]
    New,
    /// Start a session with the supplied UUID.
    Id(String),
    /// Resume a session with the supplied UUID.
    Resume(String),
}

/// Reasons why arguments could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgvError {
    /// An argument value contains a NUL character.
    ContainsNul,
    /// The positional prompt starts with a hyphen.
    PromptLooksLikeAFlag,
    /// The session ID does not have UUID syntax.
    SessionNotUuid,
    /// A tool list entry is empty, starts with a hyphen or holds a control character.
    ToolLooksLikeAFlag,
    /// The budget is non-finite or has a negative sign, including negative zero.
    BudgetNotFinite,
}

impl fmt::Display for ArgvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ContainsNul => "A value contains a NUL character.",
            Self::PromptLooksLikeAFlag => "The prompt starts with a hyphen.",
            Self::SessionNotUuid => "The session ID must be a UUID.",
            Self::ToolLooksLikeAFlag => {
                "A tool entry is empty, starts with a hyphen or holds a control character."
            }
            Self::BudgetNotFinite => "The budget must be finite and nonnegative.",
        })
    }
}

impl std::error::Error for ArgvError {}

impl Flags {
    /// Build print-mode arguments in CLI order, or report an invalid option value.
    ///
    /// Pass these arguments directly to an exec-style spawn. Never join them into a shell string.
    /// Append nothing after them because the trailing tool lists would consume extra arguments.
    /// Send untrusted prompts on standard input with `prompt: None` because the CLI may read a
    /// positional prompt as a subcommand name.
    pub fn argv(&self) -> Result<Vec<String>, ArgvError> {
        let session = match &self.session {
            Session::New => None,
            Session::Id(id) => Some(("--session-id", id)),
            Session::Resume(id) => Some(("--resume", id)),
        };
        let mut tools = self.disallowed_tools.iter().chain(&self.allowed_tools);
        if [
            &self.prompt,
            &self.system_prompt,
            &self.model,
            &self.effort,
            &self.tools,
            &self.json_schema,
            &self.setting_sources,
            &self.settings,
            &self.mcp_config,
            &self.permission_mode,
            &self.permission_prompts,
        ]
        .into_iter()
        .flatten()
        .chain(session.map(|(_, id)| id))
        .chain(tools.clone())
        .any(|value| value.contains('\0'))
        {
            return Err(ArgvError::ContainsNul);
        }
        if self
            .prompt
            .as_deref()
            .is_some_and(|prompt| prompt.starts_with('-'))
        {
            return Err(ArgvError::PromptLooksLikeAFlag);
        }
        if session.is_some_and(|(_, id)| !is_uuid(id)) {
            return Err(ArgvError::SessionNotUuid);
        }
        if tools.any(|tool| {
            tool.is_empty() || tool.starts_with('-') || tool.chars().any(char::is_control)
        }) {
            return Err(ArgvError::ToolLooksLikeAFlag);
        }
        if self
            .max_budget_usd
            .is_some_and(|budget| !budget.is_finite() || budget.is_sign_negative())
        {
            return Err(ArgvError::BudgetNotFinite);
        }

        let mut args = vec![String::from("-p")];
        args.extend(self.prompt.iter().cloned());
        push_option(&mut args, "--system-prompt", &self.system_prompt);
        push_option(&mut args, "--model", &self.model);
        push_option(&mut args, "--effort", &self.effort);
        push_switch(&mut args, "--safe-mode", self.safe_mode);
        push_switch(&mut args, "--restricted", self.restricted);
        push_switch(
            &mut args,
            "--no-session-persistence",
            self.no_session_persistence,
        );
        push_option(&mut args, "--tools", &self.tools);
        match self.output {
            Output::Text => {}
            Output::Json => args.extend(["--output-format", "json"].map(str::to_owned)),
            Output::StreamJson => {
                args.extend(["--output-format", "stream-json", "--verbose"].map(str::to_owned));
            }
        }
        push_option(&mut args, "--json-schema", &self.json_schema);
        push_option(&mut args, "--setting-sources", &self.setting_sources);
        push_option(&mut args, "--settings", &self.settings);
        push_switch(&mut args, "--strict-mcp-config", self.strict_mcp_config);
        push_option(&mut args, "--mcp-config", &self.mcp_config);
        push_option(&mut args, "--permission-mode", &self.permission_mode);
        push_option(&mut args, "--permission-prompts", &self.permission_prompts);
        if let Some(turns) = self.max_turns {
            args.extend(["--max-turns".into(), turns.to_string()]);
        }
        if let Some(budget) = self.max_budget_usd {
            args.extend(["--max-budget-usd".into(), format!("{budget:.2}")]);
        }
        if let Some((flag, id)) = session {
            args.extend([flag.into(), id.clone()]);
        }
        push_tools(&mut args, "--disallowedTools", &self.disallowed_tools);
        push_tools(&mut args, "--allowedTools", &self.allowed_tools);
        Ok(args)
    }
}

fn push_option(args: &mut Vec<String>, flag: &str, value: &Option<String>) {
    if let Some(value) = value {
        if value.starts_with('-') || (flag == "--setting-sources" && value.is_empty()) {
            args.push(format!("{flag}={value}"));
        } else {
            args.extend([flag.into(), value.clone()]);
        }
    }
}

fn push_switch(args: &mut Vec<String>, flag: &str, enabled: bool) {
    if enabled {
        args.push(flag.into());
    }
}

fn push_tools(args: &mut Vec<String>, flag: &str, tools: &[String]) {
    if !tools.is_empty() {
        args.push(flag.into());
        args.extend_from_slice(tools);
    }
}
