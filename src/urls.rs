//! URLs found in chat lines.
//!
//! A URL starts with `http://` or `https://` and runs until whitespace.

pub fn find_urls(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = find_scheme(rest) {
        let (_, after) = rest.split_at(start);
        let end = after.find(char::is_whitespace).unwrap_or(after.len());
        let url = after[..end].trim_end_matches(|ch: char| {
            matches!(
                ch,
                '.' | ',' | ';' | ':' | ')' | ']' | '>' | '"' | '\'' | '!' | '?'
            )
        });
        let scheme_len = if url
            .get(..8)
            .is_some_and(|head| head.eq_ignore_ascii_case("https://"))
        {
            8
        } else {
            7
        };
        if url.len() > scheme_len {
            found.push(url.to_string());
        }
        rest = &after[end..];
    }
    found
}

fn find_scheme(text: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    let https = lower.find("https://");
    let http = lower.find("http://");
    match (https, http) {
        (Some(https), Some(http)) => Some(https.min(http)),
        (Some(index), None) | (None, Some(index)) => Some(index),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_letter_after_the_scheme_that_is_not_ascii_is_kept() {
        assert_eq!(find_urls("http://é"), vec!["http://é".to_string()]);
        assert_eq!(
            find_urls("x https://ü.de y"),
            vec!["https://ü.de".to_string()]
        );
        assert!(find_urls("http://").is_empty());
    }

    #[test]
    fn finds_http_and_https_and_trims_punctuation() {
        let urls = find_urls("see https://example.com/a, and http://irc.test/b).");
        assert_eq!(
            urls,
            vec![
                "https://example.com/a".to_string(),
                "http://irc.test/b".to_string()
            ]
        );
    }

    #[test]
    fn keeps_a_short_http_url_and_trims_sentence_marks() {
        assert_eq!(find_urls("http://a"), vec!["http://a".to_string()]);
        assert_eq!(
            find_urls("see https://example.com!"),
            vec!["https://example.com".to_string()]
        );
        assert_eq!(
            find_urls("'https://example.com/a'"),
            vec!["https://example.com/a".to_string()]
        );
    }

    #[test]
    fn ignores_plain_words() {
        assert!(find_urls("no link here").is_empty());
    }
}
