use crate::ingest_config::ShortMemoryLimits;

const SENTENCE_TERMINATORS: [char; 6] = ['.', '!', '?', '。', '！', '？'];
const LARGE_MEMORY_WARNING: &str = "short-term memory is large; prefer 1-6 sentences";

/// Outcome of short-term memory text validation: always `ok` when returned;
/// rejections are errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortMemoryValidation {
    pub warning: String,
    pub sentence_count: usize,
    pub symbol_count: usize,
}

impl ShortMemoryValidation {
    pub fn ok(&self) -> bool {
        true
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ShortMemoryError {
    #[error("short-term memory text must not be empty")]
    EmptyText,
    #[error("short-term memory is too large")]
    TooManySentences,
    #[error("short-term memory exceeds {0} symbols")]
    TooManySymbols(usize),
}

/// Validate short-term memory text against the configured thresholds:
/// a hard sentence cap and optional hard symbol cap reject; soft caps
/// produce warnings.
pub fn validate_short_memory_text(
    text: &str,
    limits: &ShortMemoryLimits,
) -> Result<ShortMemoryValidation, ShortMemoryError> {
    let stripped = text.trim();
    if stripped.is_empty() {
        return Err(ShortMemoryError::EmptyText);
    }

    let sentence_count = count_sentences(stripped);
    let symbol_count = stripped.chars().count();
    if sentence_count > limits.rejection_sentence_count as usize {
        return Err(ShortMemoryError::TooManySentences);
    }
    if limits.rejection_symbol_count != 0 && symbol_count > limits.rejection_symbol_count as usize {
        return Err(ShortMemoryError::TooManySymbols(
            limits.rejection_symbol_count as usize,
        ));
    }

    let mut warnings: Vec<String> = Vec::new();
    if sentence_count > limits.warning_sentence_count as usize {
        warnings.push(LARGE_MEMORY_WARNING.to_string());
    }
    if limits.warning_symbol_count != 0 && symbol_count > limits.warning_symbol_count as usize {
        warnings.push(format!(
            "short-term memory is large; prefer <= {} symbols",
            limits.warning_symbol_count
        ));
    }

    Ok(ShortMemoryValidation {
        warning: warnings.join("; "),
        sentence_count,
        symbol_count,
    })
}

/// Count sentences: runs of sentence terminators count as one boundary each,
/// and trailing text after the last terminator starts an implicit sentence.
pub fn count_sentences(text: &str) -> usize {
    let mut boundaries = 0usize;
    let mut last_boundary_end: Option<usize> = None;
    let mut characters = text.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if SENTENCE_TERMINATORS.contains(&character) {
            boundaries += 1;
            // Consume the rest of the terminator run.
            while let Some(&(_, next)) = characters.peek() {
                if SENTENCE_TERMINATORS.contains(&next) {
                    characters.next();
                } else {
                    break;
                }
            }
            last_boundary_end = Some(index + character.len_utf8());
        }
    }
    match last_boundary_end {
        None => 1,
        Some(end) => {
            let tail = &text[end.min(text.len())..];
            if !tail.trim().is_empty() {
                boundaries + 1
            } else {
                boundaries
            }
        }
    }
}

/// Build the FTS5 MATCH expression for a free-text query: quoted word tokens,
/// reserved operators removed. An empty result means "nothing to search".
pub fn search_expression(query: &str) -> String {
    query
        .split(|character: char| !(character.is_alphanumeric() || character == '_'))
        .filter(|token| !token.is_empty())
        .filter(|token| !matches!(token.to_lowercase().as_str(), "and" | "or" | "not" | "near"))
        .map(|token| format!("\"{token}\""))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_japanese_sentence_punctuation() {
        assert_eq!(count_sentences("一つ。二つ！三つ？"), 3);
    }

    #[test]
    fn without_terminator_counts_as_one_sentence() {
        assert_eq!(count_sentences("No sentence terminator here"), 1);
    }

    #[test]
    fn search_expression_quotes_tokens_and_drops_operators() {
        assert_eq!(
            search_expression("Van and NOT near some-or-term"),
            "\"Van\" \"some\" \"term\"",
        );
    }

    #[test]
    fn search_expression_is_empty_without_word_tokens() {
        assert_eq!(search_expression("... !!! ---"), "");
    }
}
