//! FTS5 `MATCH` escaping. Binding a parameter does not neutralise FTS5's own query language.

/// Each whitespace-separated word becomes a quoted FTS5 string (inner `"` doubled); the words
/// are joined by spaces (implicit AND). `NOT`, `NEAR`, `*`, `^` and `col:` lose their meaning.
pub(crate) fn escape(input: &str) -> String {
    input
        .split_whitespace()
        .map(|word| format!("\"{}\"", word.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::escape;

    #[test]
    fn escape_quotes_each_word() {
        assert_eq!(escape("zealous dagger"), "\"zealous\" \"dagger\"");
    }

    #[test]
    fn escape_neutralises_operators_and_quotes() {
        assert_eq!(escape("NOT dagger*"), "\"NOT\" \"dagger*\"");
        assert_eq!(escape("say \"hi"), "\"say\" \"\"\"hi\"");
        assert_eq!(escape("NEAR( col:x ^y"), "\"NEAR(\" \"col:x\" \"^y\"");
    }

    #[test]
    fn escape_of_blank_is_empty() {
        assert_eq!(escape("   "), "");
    }
}
