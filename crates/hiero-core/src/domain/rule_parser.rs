use serde::{Deserialize, Serialize};

const DELIMITER: &str = " is translated as ";
const FORBIDDEN_DELIMITER: &str = ", not ";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedRule {
    pub source_text: String,
    pub canonical: String,
    pub forbidden: Vec<String>,
}

/// Parses the deliberately small grammar used by agent-authored deterministic rules.
///
/// Ambiguous repetitions are rejected. Structured legacy conversion has its own typed
/// field mapping and must not pass through this free-text boundary.
#[must_use]
pub fn parse_rule(text: &str) -> Option<ParsedRule> {
    let text = text.trim();
    let text = text.strip_suffix('.').unwrap_or(text);
    if text.matches(DELIMITER).count() != 1 {
        return None;
    }
    let (source, rendering) = text.split_once(DELIMITER)?;
    if source.contains(FORBIDDEN_DELIMITER) || rendering.matches(FORBIDDEN_DELIMITER).count() > 1 {
        return None;
    }
    let source = source.trim();
    let (canonical, forbidden) = match rendering.split_once(FORBIDDEN_DELIMITER) {
        Some((canonical, forbidden)) => (canonical.trim(), vec![forbidden.trim().to_owned()]),
        None => (rendering.trim(), Vec::new()),
    };
    if source.is_empty()
        || canonical.is_empty()
        || forbidden.iter().any(|value| value.is_empty())
        || has_terminal_punctuation(source)
        || has_terminal_punctuation(canonical)
        || forbidden
            .iter()
            .any(|value| has_terminal_punctuation(value))
    {
        return None;
    }
    Some(ParsedRule {
        source_text: source.to_owned(),
        canonical: canonical.to_owned(),
        forbidden,
    })
}

fn has_terminal_punctuation(value: &str) -> bool {
    value.ends_with(['.', '?', '!', '。', '！', '？', '｡', '؟', '։'])
}
