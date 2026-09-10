use std::fmt;

use serde::{Serialize, Serializer};

/// A secret value that cannot leak through `Debug`, `Display`, or
/// serialization. Only credential loaders and outbound header builders should
/// call [`Secret::expose_secret`]; public DTO types must never contain a
/// `Secret` and are built through redacting projections instead.
#[derive(Clone)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Restricted accessor. Call sites are limited to credential loaders and
    /// outbound request-header builders.
    pub fn expose_secret(&self) -> &T {
        &self.0
    }
}

/// Compared on the exposed value: two `Secret`s holding the same secret are
/// equal (the Python dataclass profile equality ports through).
impl<T: PartialEq> PartialEq for Secret<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret([redacted])")
    }
}

impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

impl<T> Serialize for Secret<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str("[redacted]")
    }
}

impl Secret<String> {
    /// Presence check for configuration gates: whether the secret is missing
    /// or whitespace-only, without exposing the value.
    pub fn is_blank(&self) -> bool {
        self.0.trim().is_empty()
    }
}

/// Replace every occurrence of the given secret values in `text` with
/// `[redacted]`, longest values first so overlapping values cannot leave
/// fragments behind.
pub fn redact_values(text: &str, values: &[&str]) -> String {
    let mut redacted = text.to_string();
    let mut sorted: Vec<&str> = values
        .iter()
        .copied()
        .filter(|value| !value.is_empty())
        .collect();
    sorted.sort_by_key(|value| std::cmp::Reverse(value.len()));
    for value in sorted {
        redacted = redacted.replace(value, "[redacted]");
    }
    redacted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_skips_empty_values() {
        assert_eq!(redact_values("unchanged", &[""]), "unchanged");
    }

    #[test]
    fn redact_longest_value_first() {
        assert_eq!(
            redact_values("abcdef", &["abc", "abcdef"]),
            "[redacted]",
            "the longer value must win; prefix-only replacement would leave cdef"
        );
    }
}
