use crate::{
    domain::apply_malformed_confidence_penalty,
    provider::{DreamOutput, MAX_RESPONSE_BYTES},
};

pub const MALFORMED_OUTPUT_PENALTY: f64 = 0.2;
const BLOCKING_PARSE_THRESHOLD: usize = 64 * 1024;
const MAX_OUTPUT_RECORDS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MalformedPayloadError {
    #[error("dream payload is empty")]
    Empty,
    #[error("dream payload exceeds the response limit")]
    TooLarge,
    #[error("dream payload is invalid")]
    Invalid,
    #[error("dream payload schema is invalid")]
    Schema,
    #[error("dream payload recovery is ambiguous")]
    Ambiguous,
    #[error("dream payload parsing task failed")]
    ParseTask,
}

#[must_use]
pub fn strip_code_fences(raw: &str) -> &str {
    let trimmed = raw.trim();
    let Some(after_open) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let Some(first_newline) = after_open.find('\n') else {
        return trimmed;
    };
    let body = &after_open[first_newline + 1..];
    let Some(body) = body.strip_suffix("```") else {
        return trimmed;
    };
    body.trim()
}

pub fn parse_dream_output(raw: &str) -> Result<DreamOutput, MalformedPayloadError> {
    if raw.trim().is_empty() {
        return Err(MalformedPayloadError::Empty);
    }
    if raw.len() > MAX_RESPONSE_BYTES {
        return Err(MalformedPayloadError::TooLarge);
    }
    match decode(raw.trim()) {
        Ok(output) => return Ok(output),
        Err(MalformedPayloadError::Schema) => return Err(MalformedPayloadError::Schema),
        Err(_) => {}
    }

    let fenced = fenced_candidates(raw);
    if fenced.len() > 1 {
        return Err(MalformedPayloadError::Ambiguous);
    }
    if let Some(candidate) = fenced.first() {
        return decode_recovered(candidate);
    }

    let balanced = balanced_candidates(raw);
    if balanced.len() > 1 {
        return Err(MalformedPayloadError::Ambiguous);
    }
    let Some(candidate) = balanced.first() else {
        return Err(MalformedPayloadError::Invalid);
    };
    decode_recovered(candidate)
}

pub async fn parse_dream_output_async(raw: String) -> Result<DreamOutput, MalformedPayloadError> {
    if raw.len() < BLOCKING_PARSE_THRESHOLD {
        return parse_dream_output(&raw);
    }
    tokio::task::spawn_blocking(move || parse_dream_output(&raw))
        .await
        .map_err(|_| MalformedPayloadError::ParseTask)?
}

pub(crate) async fn parse_provider_value_async(
    raw: String,
) -> Result<serde_json::Value, MalformedPayloadError> {
    if raw.trim().is_empty() {
        return Err(MalformedPayloadError::Empty);
    }
    if raw.len() > MAX_RESPONSE_BYTES {
        return Err(MalformedPayloadError::TooLarge);
    }
    if raw.len() < BLOCKING_PARSE_THRESHOLD {
        return serde_json::from_str(&raw).map_err(|_| MalformedPayloadError::Invalid);
    }
    tokio::task::spawn_blocking(move || serde_json::from_str(&raw))
        .await
        .map_err(|_| MalformedPayloadError::ParseTask)?
        .map_err(|_| MalformedPayloadError::Invalid)
}

fn decode(raw: &str) -> Result<DreamOutput, MalformedPayloadError> {
    let output: DreamOutput = serde_json::from_str(raw).map_err(|error| {
        if error.is_data() {
            MalformedPayloadError::Schema
        } else {
            MalformedPayloadError::Invalid
        }
    })?;
    validate_output(&output)?;
    Ok(output)
}

fn validate_output(output: &DreamOutput) -> Result<(), MalformedPayloadError> {
    if output.crystals.len() + output.concepts.len() > MAX_OUTPUT_RECORDS {
        return Err(MalformedPayloadError::Schema);
    }
    let crystals_valid = output.crystals.iter().all(|candidate| {
        !candidate.crystal_type.trim().is_empty()
            && !candidate.title.trim().is_empty()
            && !candidate.text.trim().is_empty()
            && !candidate.source_credibility.trim().is_empty()
            && candidate.confidence.is_finite()
            && (0.0..=1.0).contains(&candidate.confidence)
            && candidate.malformed_penalty == 0.0
    });
    let concepts_valid = output.concepts.iter().all(|candidate| {
        !candidate.canonical_name.trim().is_empty()
            && candidate.facets.iter().all(|(kind, value, language)| {
                !kind.trim().is_empty() && !value.trim().is_empty() && !language.trim().is_empty()
            })
    });
    if crystals_valid && concepts_valid {
        Ok(())
    } else {
        Err(MalformedPayloadError::Schema)
    }
}

fn decode_recovered(raw: &str) -> Result<DreamOutput, MalformedPayloadError> {
    let mut output = decode(raw)?;
    output.recovered = true;
    for candidate in &mut output.crystals {
        candidate.malformed_penalty += MALFORMED_OUTPUT_PENALTY;
        candidate.confidence =
            apply_malformed_confidence_penalty(candidate.confidence, MALFORMED_OUTPUT_PENALTY);
    }
    Ok(output)
}

fn fenced_candidates(raw: &str) -> Vec<&str> {
    let mut candidates = Vec::new();
    let mut remaining = raw;
    while let Some(open) = remaining.find("```") {
        let after_marker = &remaining[open + 3..];
        let Some(line_end) = after_marker.find('\n') else {
            break;
        };
        let language = after_marker[..line_end].trim();
        let body = &after_marker[line_end + 1..];
        let Some(close) = body.find("```") else {
            break;
        };
        if language.is_empty() || language.eq_ignore_ascii_case("json") {
            candidates.push(body[..close].trim());
        }
        remaining = &body[close + 3..];
    }
    candidates
}

fn balanced_candidates(raw: &str) -> Vec<&str> {
    let bytes = raw.as_bytes();
    let mut candidates = Vec::new();
    let mut start = None;
    let mut stack = Vec::new();
    let mut in_string = false;
    let mut escaped = false;

    for (index, &byte) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        if byte == b'"' && start.is_some() {
            in_string = true;
            continue;
        }
        match byte {
            b'{' | b'[' => {
                if stack.is_empty() {
                    start = Some(index);
                }
                stack.push(byte);
            }
            b'}' | b']' if !stack.is_empty() => {
                let expected = if byte == b'}' { b'{' } else { b'[' };
                if stack.last() != Some(&expected) {
                    stack.clear();
                    start = None;
                    continue;
                }
                stack.pop();
                if stack.is_empty()
                    && let Some(begin) = start.take()
                {
                    candidates.push(&raw[begin..=index]);
                }
            }
            _ => {}
        }
    }
    candidates
        .into_iter()
        .filter(|candidate| decode(candidate).is_ok())
        .collect()
}
