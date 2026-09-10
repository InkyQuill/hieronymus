//! Exact deterministic English correction grammar. Payload bytes are never case-folded.
use hieronymus::authority_models::{CorrectionIntentV1, FactEffect, RenderingV1, TentativeReason};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ByteSpan {
    pub start: usize,
    pub end: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParsedIntent {
    Rendering,
    Invalidate,
    Qualify,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParsedCorrectionV1 {
    pub intent: ParsedIntent,
    pub event_span: ByteSpan,
    pub source_span: Option<ByteSpan>,
    pub target_span: Option<ByteSpan>,
    pub qualification_span: Option<ByteSpan>,
    pub decoded_value: Option<String>,
}
/// Immutable event loaded/minted by trusted ingress, never provider extraction.
pub struct OriginReceipt {
    pub text: String,
}
/// Values here are resolved from selected immutable evidence and typed claims.
pub struct SelectionContext {
    pub source_text: Option<String>,
    pub source_count: usize,
    pub claim: Option<(i64, u64)>,
    pub claim_count: usize,
    pub languages_resolved: bool,
    pub scope_resolved: bool,
    pub replaces: Option<(i64, u64)>,
    pub rendering: Option<RenderingV1>,
}
fn ambiguous<T>() -> Result<T, TentativeReason> {
    Err(TentativeReason::AmbiguousIntent)
}
struct Cursor<'a> {
    text: &'a str,
    at: usize,
    end: usize,
}
impl Cursor<'_> {
    fn keyword(&mut self, keyword: &str) -> bool {
        let end = self.at + keyword.len();
        if self
            .text
            .get(self.at..end)
            .is_some_and(|s| s.eq_ignore_ascii_case(keyword))
        {
            self.at = end;
            true
        } else {
            false
        }
    }
    fn ws(&mut self) -> bool {
        let start = self.at;
        while self.at < self.end && matches!(self.text.as_bytes()[self.at], b' ' | b'\t') {
            self.at += 1;
        }
        self.at > start
    }
    fn quoted(&mut self) -> Result<(ByteSpan, String), TentativeReason> {
        let start = self.at;
        if self.text.as_bytes().get(start) != Some(&b'"') {
            return ambiguous();
        }
        self.at += 1;
        let mut escaped = false;
        while self.at < self.end {
            let b = self.text.as_bytes()[self.at];
            self.at += 1;
            if b == b'"' && !escaped {
                let value: String = serde_json::from_str(&self.text[start..self.at])
                    .map_err(|_| TentativeReason::AmbiguousIntent)?;
                if value.trim().is_empty() || value.chars().any(char::is_control) {
                    return ambiguous();
                }
                return Ok((
                    ByteSpan {
                        start,
                        end: self.at,
                    },
                    value,
                ));
            }
            escaped = b == b'\\' && !escaped;
        }
        ambiguous()
    }
}
pub fn parse_correction_v1(receipt: &OriginReceipt) -> Result<ParsedCorrectionV1, TentativeReason> {
    let text = &receipt.text;
    if text.contains(['\r', '\n']) {
        return ambiguous();
    }
    let start = text.len() - text.trim_start_matches([' ', '\t']).len();
    let end = text.trim_end_matches([' ', '\t']).len();
    if start >= end {
        return ambiguous();
    }
    let mut cursor = Cursor {
        text,
        at: start,
        end,
    };
    let mut parsed = ParsedCorrectionV1 {
        intent: ParsedIntent::Invalidate,
        event_span: ByteSpan { start, end },
        source_span: None,
        target_span: None,
        qualification_span: None,
        decoded_value: None,
    };
    if cursor.keyword("translate") && cursor.ws() {
        parsed.intent = ParsedIntent::Rendering;
        let quoted_source = text.as_bytes().get(cursor.at) == Some(&b'"');
        parsed.source_span = Some(if quoted_source {
            cursor.quoted()?.0
        } else {
            let start = cursor.at;
            if !cursor.keyword("this") {
                return ambiguous();
            }
            ByteSpan {
                start,
                end: cursor.at,
            }
        });
        if !cursor.ws() || !cursor.keyword("as") || !cursor.ws() {
            return ambiguous();
        }
        let (span, value) = if text.as_bytes().get(cursor.at) == Some(&b'"') {
            cursor.quoted()?
        } else {
            if quoted_source {
                return ambiguous();
            }
            let value = &text[cursor.at..end];
            if value.trim().is_empty()
                || value.contains(['"', '\\', ',', ';'])
                || value.chars().any(char::is_control)
            {
                return ambiguous();
            }
            let words: Vec<_> = value
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .filter(|w| !w.is_empty())
                .collect();
            if words.iter().any(|w| {
                ["and", "or", "translate", "qualify"]
                    .iter()
                    .any(|bad| w.eq_ignore_ascii_case(bad))
            }) || words.windows(2).any(|w| {
                (w[0].eq_ignore_ascii_case("that") || w[0].eq_ignore_ascii_case("this"))
                    && w[1].eq_ignore_ascii_case("memory")
                    || w[0].eq_ignore_ascii_case("this")
                        && w[1].eq_ignore_ascii_case("recollection")
            }) {
                return ambiguous();
            }
            let span = ByteSpan {
                start: cursor.at,
                end,
            };
            cursor.at = end;
            (span, value.to_owned())
        };
        parsed.target_span = Some(span);
        parsed.decoded_value = Some(value);
    } else {
        cursor.at = start;
        let qualification = cursor.keyword("qualify");
        if qualification && !cursor.ws() {
            return ambiguous();
        }
        let is_this = cursor.keyword("this");
        if !is_this && !cursor.keyword("that") {
            return ambiguous();
        }
        if !cursor.ws() {
            return ambiguous();
        }
        if !(cursor.keyword("memory")
            || is_this && !qualification && cursor.keyword("recollection"))
        {
            return ambiguous();
        }
        if !cursor.ws() {
            return ambiguous();
        }
        if qualification {
            if !cursor.keyword("as") || !cursor.ws() {
                return ambiguous();
            }
            let (span, value) = cursor.quoted()?;
            parsed.intent = ParsedIntent::Qualify;
            parsed.qualification_span = Some(span);
            parsed.decoded_value = Some(value);
        } else {
            if !cursor.keyword("is") || !cursor.ws() || !cursor.keyword("wrong") {
                return ambiguous();
            }
            cursor.keyword(".");
        }
    }
    if cursor.at != end {
        return ambiguous();
    }
    Ok(parsed)
}
pub fn bind_selection(
    parsed: &ParsedCorrectionV1,
    receipt: &OriginReceipt,
    context: &SelectionContext,
) -> Result<CorrectionIntentV1, TentativeReason> {
    // Reparse so neither spans nor decoded values can be substituted.
    if parse_correction_v1(receipt)? != *parsed {
        return ambiguous();
    }
    if !context.scope_resolved {
        return Err(TentativeReason::UnknownOrder);
    }
    if !context.languages_resolved {
        return Err(TentativeReason::AmbiguousIdentity);
    }
    match parsed.intent {
        ParsedIntent::Rendering => {
            if context.source_count != 1 {
                return Err(TentativeReason::AmbiguousIdentity);
            }
            let source = context
                .source_text
                .as_ref()
                .ok_or(TentativeReason::AmbiguousIdentity)?;
            let span = parsed
                .source_span
                .as_ref()
                .ok_or(TentativeReason::AmbiguousIdentity)?;
            let raw = &receipt.text[span.start..span.end];
            if raw.starts_with('"')
                && serde_json::from_str::<String>(raw).ok().as_ref() != Some(source)
            {
                return Err(TentativeReason::AmbiguousIdentity);
            }
            let mut value = context.rendering.clone().unwrap_or(RenderingV1 {
                source_forms: vec![source.clone()],
                canonical: String::new(),
                approved_variants: vec![],
                forbidden_variants: vec![],
                case_sensitive: false,
            });
            value.canonical = parsed
                .decoded_value
                .clone()
                .ok_or(TentativeReason::AmbiguousIntent)?;
            Ok(CorrectionIntentV1::Rendering {
                replaces: context.replaces,
                value,
            })
        }
        ParsedIntent::Invalidate | ParsedIntent::Qualify => {
            if context.claim_count != 1 {
                return Err(TentativeReason::AmbiguousIdentity);
            }
            let (claim_id, claim_revision) =
                context.claim.ok_or(TentativeReason::AmbiguousIdentity)?;
            let effect = match parsed.intent {
                ParsedIntent::Invalidate => FactEffect::Invalidate,
                _ => FactEffect::Qualify {
                    qualification: parsed
                        .decoded_value
                        .clone()
                        .ok_or(TentativeReason::AmbiguousIntent)?,
                },
            };
            Ok(CorrectionIntentV1::Fact {
                claim_id,
                claim_revision,
                effect,
            })
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_grammar_and_exact_payloads() {
        for (text, expected) in [
            ("translate this as X", "X"),
            (" \tTRANSLATE\tthis  As Mixed.Case.\t", "Mixed.Case."),
            (r#"translate "Свет" as "Light""#, "Light"),
            (r#"translate this as "The \"Star\"""#, "The \"Star\""),
            (r#"translate this as "a\\b\u2603""#, "a\\b☃"),
            (r#"translate this as "Rock and Roll""#, "Rock and Roll"),
            (
                r#"qualify that memory as "Mira suspects this; it is not confirmed""#,
                "Mira suspects this; it is not confirmed",
            ),
        ] {
            let receipt = OriginReceipt { text: text.into() };
            let parsed = parse_correction_v1(&receipt).unwrap();
            assert_eq!(parsed.decoded_value.as_deref(), Some(expected), "{text}");
            let span = parsed.target_span.or(parsed.qualification_span).unwrap();
            assert!(receipt.text.get(span.start..span.end).is_some());
        }
        for text in [
            "that memory is wrong",
            "THIS\tmemory is wrong.",
            "this recollection is wrong",
        ] {
            assert_eq!(
                parse_correction_v1(&OriginReceipt { text: text.into() })
                    .unwrap()
                    .intent,
                ParsedIntent::Invalidate
            );
        }
    }
    #[test]
    fn ambiguous_events_are_never_partially_parsed() {
        for text in [
            "translate this as Rock and Roll",
            "translate this as X or Y",
            "translate this as X; translate this as Y",
            "please translate this as X",
            "translate this as \"X\".",
            "translate this as \"X\" trailing",
            "переводи это как X",
            "translate this as \"unfinished",
            "translate this as X,Y",
            "translate this as X\tY",
            "translate this as X\\Y",
            "translate this as qualify that memory",
            "translate this as X\n",
            r#"translate this as "\uD800""#,
            r#"translate this as "\n""#,
            r#"translate this as " ""#,
            r#"translate "Alex" as Bare"#,
        ] {
            assert_eq!(
                parse_correction_v1(&OriginReceipt { text: text.into() }),
                Err(TentativeReason::AmbiguousIntent),
                "{text}"
            );
        }
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    fn context() -> SelectionContext {
        SelectionContext {
            source_text: Some("Свет".into()),
            source_count: 1,
            claim: Some((7, 3)),
            claim_count: 1,
            languages_resolved: true,
            scope_resolved: true,
            replaces: None,
            rendering: None,
        }
    }
    fn bind(text: &str, c: &SelectionContext) -> Result<CorrectionIntentV1, TentativeReason> {
        let r = OriginReceipt { text: text.into() };
        bind_selection(&parse_correction_v1(&r)?, &r, c)
    }
    #[test]
    fn exact_selection_claim_and_context_matrix() {
        assert!(matches!(
            bind(r#"translate "Свет" as "Light""#, &context()),
            Ok(CorrectionIntentV1::Rendering { .. })
        ));
        assert_eq!(
            bind(r#"translate "Alex" as "A""#, &context()),
            Err(TentativeReason::AmbiguousIdentity)
        );
        for count in [0, 2] {
            let mut c = context();
            c.source_count = count;
            assert_eq!(
                bind("translate this as X", &c),
                Err(TentativeReason::AmbiguousIdentity)
            );
        }
        let mut c = context();
        c.languages_resolved = false;
        assert_eq!(
            bind("translate this as X", &c),
            Err(TentativeReason::AmbiguousIdentity)
        );
        c = context();
        c.scope_resolved = false;
        assert_eq!(
            bind("translate this as X", &c),
            Err(TentativeReason::UnknownOrder)
        );
        assert_eq!(
            bind("that memory is wrong", &context()),
            Ok(CorrectionIntentV1::Fact {
                claim_id: 7,
                claim_revision: 3,
                effect: FactEffect::Invalidate
            })
        );
        for count in [0, 3] {
            let mut c = context();
            c.claim_count = count;
            assert_eq!(
                bind("that memory is wrong", &c),
                Err(TentativeReason::AmbiguousIdentity)
            );
        }
        let receipt = OriginReceipt {
            text: r#" \ttranslate "Свет" as "\u2603""#.replace("\\t", "\t"),
        };
        let mut parsed = parse_correction_v1(&receipt).unwrap();
        let span = parsed.source_span.as_ref().unwrap();
        assert_eq!(&receipt.text[span.start..span.end], "\"Свет\"");
        parsed.decoded_value = Some("forged".into());
        assert_eq!(
            bind_selection(&parsed, &receipt, &context()),
            Err(TentativeReason::AmbiguousIntent)
        );
    }
}
