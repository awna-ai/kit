//! Command-line argument builders and stream parsers for coding-agent CLIs. Parsing only: no
//! process code.
//!
//! [`print::Flags`] builds the arguments for a print-mode call. [`print::stream`] reads a
//! print-mode stream and [`exec`] reads an exec-mode JSON Lines stream; both return raw facts
//! in one [`Summary`], so each caller decides what counts as success. Other backends come as
//! modules beside these.
//!
//! ```
//! use awna_agents::print::{Flags, Output, stream::summarize};
//!
//! let flags = Flags {
//!     prompt: Some("Read the total.".into()),
//!     output: Output::StreamJson,
//!     ..Flags::default()
//! };
//! let argv = flags.argv()?;
//! assert_eq!(argv, ["-p", "Read the total.", "--output-format", "stream-json", "--verbose"]);
//!
//! let summary = summarize(br#"{"type":"system","subtype":"init","permissionMode":"default"}
//! {"type":"result","subtype":"success","is_error":false,"result":"The total is 7."}"#);
//! assert_eq!(summary.permission_mode.as_deref(), Some("default"));
//! assert_eq!(summary.result.and_then(|result| result.text).as_deref(), Some("The total is 7."));
//! # Ok::<(), awna_agents::print::ArgvError>(())
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod exec;
pub mod print;
mod summary;

pub use summary::{
    COST_MAX, DURATION_MAX_MS, ModelUsage, RESETS_AT_MAX, RateLimit, ResultLine, Spend, Summary,
    TOKENS_MAX, TURNS_MAX, Tokens, Window, Windows, is_category, is_model, is_uuid,
};
