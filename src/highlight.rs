/// First highlight entry that appears in `text`.
///
/// Matching is case-insensitive. An entry matches when it occurs as its own
/// span: bounded by the start or end of the line, or by a character that is
/// not ASCII alphanumeric. Multi-word entries such as "good night" match as a
/// phrase with the same boundary rule on the whole phrase. Empty and
/// whitespace-only entries never match. The returned string is the entry from
/// `words`, not the slice of `text`.
pub fn first_match<'a>(text: &str, words: &'a [String]) -> Option<&'a str> {
    let text_chars: Vec<char> = text.chars().collect();
    for word in words {
        if word.trim().is_empty() {
            continue;
        }
        if entry_matches(&text_chars, word) {
            return Some(word.as_str());
        }
    }
    None
}

fn entry_matches(text_chars: &[char], entry: &str) -> bool {
    let entry_chars: Vec<char> = entry.chars().collect();
    if entry_chars.is_empty() || entry_chars.len() > text_chars.len() {
        return false;
    }
    let limit = text_chars.len() - entry_chars.len();
    for start in 0..=limit {
        if !matches_at(text_chars, start, &entry_chars) {
            continue;
        }
        let before_ok = start == 0 || !text_chars[start - 1].is_ascii_alphanumeric();
        if !before_ok {
            continue;
        }
        let end = start + entry_chars.len();
        let after_ok = end == text_chars.len() || !text_chars[end].is_ascii_alphanumeric();
        if after_ok {
            return true;
        }
    }
    false
}

fn matches_at(text_chars: &[char], start: usize, entry_chars: &[char]) -> bool {
    for (offset, &expected) in entry_chars.iter().enumerate() {
        if !chars_equal_folded(text_chars[start + offset], expected) {
            return false;
        }
    }
    true
}

fn chars_equal_folded(a: char, b: char) -> bool {
    if a == b {
        return true;
    }
    a.to_lowercase().eq(b.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_word() {
        let words = vec!["rust".to_string()];
        assert_eq!(first_match("hello rust", &words), Some("rust"));
    }

    #[test]
    fn does_not_match_inside_word() {
        let words = vec!["rust".to_string()];
        assert_eq!(first_match("rusty nails", &words), None);
    }

    #[test]
    fn matches_case_insensitively() {
        let words = vec!["hello".to_string()];
        assert_eq!(first_match("Hello there", &words), Some("hello"));
    }

    #[test]
    fn matches_phrase() {
        let words = vec!["good night".to_string()];
        assert_eq!(
            first_match("say good night now", &words),
            Some("good night")
        );
    }

    #[test]
    fn empty_and_blank_entries_never_match() {
        let words = vec!["".to_string(), "   ".to_string()];
        assert_eq!(first_match("hello", &words), None);
    }

    #[test]
    fn first_matching_entry_wins() {
        let words = vec!["rust".to_string(), "hello".to_string()];
        assert_eq!(first_match("hello rust", &words), Some("rust"));
        let words = vec!["hello".to_string(), "rust".to_string()];
        assert_eq!(first_match("hello rust", &words), Some("hello"));
    }
}
