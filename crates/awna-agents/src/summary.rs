use serde_json::{Map, Value};

/// The largest accepted token count in a single field.
pub const TOKENS_MAX: u64 = 1_000_000_000_000;
/// The largest accepted cost in US dollars.
pub const COST_MAX: f64 = 100_000.0;
/// The largest accepted turn count in a result.
pub const TURNS_MAX: u64 = 1_000_000;
/// The largest accepted duration in milliseconds.
pub const DURATION_MAX_MS: u64 = 30 * 24 * 3600 * 1000;
/// The exclusive upper bound for a reset time in epoch seconds.
pub const RESETS_AT_MAX: f64 = 32_503_680_000.0;

/// Facts collected from a session's output.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    /// The first valid permission mode reported by an initialization event.
    pub permission_mode: Option<String>,
    /// The most recent result.
    pub result: Option<ResultLine>,
    /// The most recent result's reported spend, or earlier spend if it reported none.
    pub spend: Option<Spend>,
    /// The last error category that a later success did not clear.
    pub api_error: Option<String>,
    /// Rate limit events and usage windows.
    pub rate_limit: RateLimit,
    /// Whether the session reported a background task cut-off.
    pub background_cut_off: bool,
    /// The number of nonblank lines that did not parse as JSON objects.
    pub non_json_lines: usize,
}

/// The fields reported by a session result.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResultLine {
    /// The reported result subtype if it passes [`is_category`].
    pub subtype: Option<String>,
    /// The error flag when reported as a JSON boolean.
    pub is_error: Option<bool>,
    /// The full result text.
    pub text: Option<String>,
    /// The reported session ID if it passes [`is_uuid`].
    pub session_id: Option<String>,
    /// The structured output in its original JSON type.
    pub structured_output: Option<Value>,
    /// The spend reported by this result.
    pub spend: Option<Spend>,
}

/// Reported token usage, cost, turns, and duration.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spend {
    /// Total cost in US dollars.
    pub cost_usd: Option<f64>,
    /// Token counts from the aggregate usage object.
    pub usage: Option<Tokens>,
    /// Valid model usage entries sorted by model identifier.
    pub model_usage: Vec<ModelUsage>,
    /// Whether a model usage object was present.
    pub has_model_usage: bool,
    /// The reported or accumulated turn count.
    pub turns: Option<u64>,
    /// The reported duration in milliseconds.
    pub duration_ms: Option<u64>,
}

/// Token counts by usage type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    /// Uncached input tokens.
    pub input: u64,
    /// Tokens written to the cache.
    pub cache_write: u64,
    /// Tokens read from the cache.
    pub cache_read: u64,
    /// Output tokens.
    pub output: u64,
}

impl Tokens {
    /// Adds the counts independently, saturating at the maximum for each field.
    pub fn saturating_add(self, other: Self) -> Self {
        Self {
            input: self.input.saturating_add(other.input),
            cache_write: self.cache_write.saturating_add(other.cache_write),
            cache_read: self.cache_read.saturating_add(other.cache_read),
            output: self.output.saturating_add(other.output),
        }
    }
}

/// Reported usage for one model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelUsage {
    /// The model identifier.
    pub model: String,
    /// Token counts for this model.
    pub tokens: Tokens,
    /// This model's cost in US dollars.
    pub cost_usd: Option<f64>,
}

/// Facts reported by rate limit events.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RateLimit {
    /// Whether any event reported a rejection.
    pub rejected: bool,
    /// Whether any event reported an allowed warning.
    pub warned: bool,
    /// The latest reset time reported by a rejection.
    pub rejected_until: Option<f64>,
    /// The first usable set of windows.
    pub first_windows: Option<Windows>,
    /// The last usable set of windows.
    pub windows: Option<Windows>,
}

/// The windows reported by a rate limit event.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Windows {
    /// The five-hour window.
    pub five_hour: Option<Window>,
    /// The seven-day window.
    pub seven_day: Option<Window>,
}

/// Utilization and reset time for one rate limit window.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Window {
    /// The share of the limit used, capped at one.
    pub utilization: f64,
    /// The reset time in epoch seconds.
    pub resets_at: f64,
}

/// Checks whether a string has the expected ASCII model identifier format.
pub fn is_model(raw: &str) -> bool {
    let mut bytes = raw.bytes();
    raw.len() <= 100
        && bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || b"-._:[]@".contains(&byte))
}

/// Checks whether a string has the expected ASCII category identifier format.
pub fn is_category(raw: &str) -> bool {
    let mut bytes = raw.bytes();
    raw.len() <= 40
        && bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

/// Checks for the 8-4-4-4-12 UUID syntax using ASCII hexadecimal digits.
pub fn is_uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

pub(crate) fn string<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

pub(crate) fn count(usage: &Map<String, Value>, key: &str) -> u64 {
    bounded_count(usage, key, TOKENS_MAX).unwrap_or(0)
}

fn bounded_count(object: &Map<String, Value>, key: &str, max: u64) -> Option<u64> {
    object.get(key)?.as_u64().filter(|&count| count <= max)
}

fn cost(value: &Value) -> Option<f64> {
    value.as_f64().filter(|usd| (0.0..=COST_MAX).contains(usd))
}

pub(crate) fn print_spend(object: &Map<String, Value>) -> Option<Spend> {
    let by_model = object.get("modelUsage").and_then(Value::as_object);
    let usage = object.get("usage").and_then(Value::as_object);
    let cost_usd = object.get("total_cost_usd").and_then(cost);
    let turns = bounded_count(object, "num_turns", TURNS_MAX);
    if by_model.is_none() && usage.is_none() && cost_usd.is_none() && turns.is_none() {
        return None;
    }

    Some(Spend {
        cost_usd,
        usage: usage.map(|usage| Tokens {
            input: count(usage, "input_tokens"),
            cache_write: count(usage, "cache_creation_input_tokens"),
            cache_read: count(usage, "cache_read_input_tokens"),
            output: count(usage, "output_tokens"),
        }),
        model_usage: by_model.map(model_usage).unwrap_or_default(),
        has_model_usage: by_model.is_some(),
        turns,
        duration_ms: bounded_count(object, "duration_ms", DURATION_MAX_MS),
    })
}

fn model_usage(by_model: &Map<String, Value>) -> Vec<ModelUsage> {
    let mut models: Vec<_> = by_model
        .iter()
        .filter(|(model, _)| is_model(model))
        .filter_map(|(model, usage)| {
            let usage = usage.as_object()?;
            Some(ModelUsage {
                model: model.clone(),
                tokens: Tokens {
                    input: count(usage, "inputTokens"),
                    cache_write: count(usage, "cacheCreationInputTokens"),
                    cache_read: count(usage, "cacheReadInputTokens"),
                    output: count(usage, "outputTokens"),
                },
                cost_usd: usage.get("costUSD").and_then(cost),
            })
        })
        .collect();
    models.sort_by(|left, right| left.model.cmp(&right.model));
    models
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn model_identifiers_follow_ascii_and_length_rules() {
        for model in ["opus-5-5", "a._:[]@-0", &"a".repeat(100)] {
            assert!(is_model(model), "{model}");
        }
        for model in ["", "-a", "a/b", "a b", "aé", &"a".repeat(101)] {
            assert!(!is_model(model), "{model}");
        }
    }

    #[test]
    fn categories_follow_ascii_and_length_rules() {
        for category in ["rate_limit", "auto", "A-b9", &"a".repeat(40)] {
            assert!(is_category(category), "{category}");
        }
        for category in ["", "0a", "a.b", "a b", "aé", &"a".repeat(41)] {
            assert!(!is_category(category), "{category}");
        }
    }

    #[test]
    fn duration_alone_does_not_count_as_spend() {
        let value = json!({"duration_ms": 9});
        assert_eq!(print_spend(value.as_object().unwrap()), None);
        let value = json!({"duration_ms": 9, "modelUsage": {}});
        let spend = print_spend(value.as_object().unwrap());
        assert_eq!(spend.and_then(|spend| spend.duration_ms), Some(9));
    }

    #[test]
    fn spend_accepts_inclusive_bounds() {
        let value = json!({
            "total_cost_usd": COST_MAX,
            "usage": {"input_tokens": TOKENS_MAX},
            "num_turns": TURNS_MAX,
            "duration_ms": DURATION_MAX_MS
        });
        let spend = print_spend(value.as_object().unwrap()).unwrap_or_default();
        assert_eq!(spend.cost_usd, Some(COST_MAX));
        assert_eq!(spend.usage.unwrap().input, TOKENS_MAX);
        assert_eq!(spend.turns, Some(TURNS_MAX));
        assert_eq!(spend.duration_ms, Some(DURATION_MAX_MS));
    }

    #[test]
    fn token_sums_saturate_independently() {
        let left = Tokens {
            input: u64::MAX,
            cache_write: 2,
            cache_read: 3,
            output: 4,
        };
        let right = Tokens {
            input: 1,
            cache_write: 5,
            cache_read: 6,
            output: 7,
        };
        assert_eq!(
            left.saturating_add(right),
            Tokens {
                input: u64::MAX,
                cache_write: 7,
                cache_read: 9,
                output: 11
            }
        );
    }
}
