//! A dictionary check for the transcript and the input line.
//!
//! Words are compared case-insensitively, in any alphabet: accented Latin,
//! Greek, Cyrillic, and so on. URLs, nicknames, and words shorter than
//! three letters are left alone, as is text in scripts written without
//! spaces between words (Chinese, Japanese, Thai, ...), where a dictionary
//! cannot tell where one word ends.

use std::collections::HashSet;

/// Read a word list: one or more words per line. Hunspell `.dic` files work
/// too: the count on the first line is skipped and `/FLAGS` are dropped
/// (the affix rules are not applied, so only the listed forms are known).
pub fn load(text: &str) -> HashSet<String> {
    text.split_whitespace()
        .map(|word| word.split('/').next().unwrap_or_default())
        .map(normalize)
        .filter(|word| word.chars().count() >= 3 && word.chars().any(char::is_alphabetic))
        .collect()
}

/// Where words added from the right-click menu are kept for `dictionary`:
/// a UTF-8 file beside it, so the original list (perhaps in an old code
/// page) is never changed.
pub fn added_path(dictionary: &std::path::Path) -> std::path::PathBuf {
    let mut name = dictionary.as_os_str().to_owned();
    name.push(".added.txt");
    std::path::PathBuf::from(name)
}

/// The text of a word list. A Hunspell `.dic` is read in the encoding its
/// `.aff` names on the `SET` line; anything else is read line by line as
/// UTF-8, or Windows-1252 for a line that is not. A `SET` naming an
/// encoding this program cannot read is an error, so the list is not read
/// wrongly.
pub fn decode_list(path: &std::path::Path, bytes: &[u8]) -> Result<String, String> {
    let label = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("dic"))
        .then(|| std::fs::read(path.with_extension("aff")).ok())
        .flatten()
        .and_then(|aff| {
            String::from_utf8_lossy(&aff).lines().find_map(|line| {
                // A byte order mark may come before the first line.
                let line = line.trim_start_matches('\u{feff}').trim();
                let label = line.strip_prefix("SET")?;
                label
                    .starts_with(char::is_whitespace)
                    .then(|| label.trim().to_string())
            })
        });
    if let Some(label) = label {
        let Some(encoding) = encoding_rs::Encoding::for_label(label.as_bytes()) else {
            return Err(format!(
                "its .aff file names the encoding {label}, which OpenIRC cannot read"
            ));
        };
        return Ok(encoding.decode_without_bom_handling(bytes).0.into_owned());
    }
    Ok(bytes
        .split(|byte| *byte == b'\n')
        .map(crate::message::decode_line)
        .collect::<Vec<_>>()
        .join("\n"))
}

/// The words of a dictionary file and of the words added to it.
pub fn read(path: &std::path::Path) -> Result<HashSet<String>, String> {
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    let mut words = load(&decode_list(path, &bytes)?);
    if let Ok(bytes) = std::fs::read(added_path(path)) {
        words.extend(load(&String::from_utf8_lossy(&bytes)));
    }
    Ok(words)
}

/// Lowercase, with a typographic apostrophe read as a plain one.
pub fn normalize(word: &str) -> String {
    word.trim_matches(is_apostrophe)
        .to_lowercase()
        .replace('\u{2019}', "'")
}

fn is_apostrophe(ch: char) -> bool {
    ch == '\'' || ch == '\u{2019}'
}

/// Marks that belong to the letter before them: accents written as
/// separate characters, Hebrew and Arabic vowel points, and the vowel signs
/// and viramas of the Indian scripts (whose whole blocks are counted, less
/// the full stops).
fn is_mark(ch: char) -> bool {
    matches!(ch as u32,
        0x0300..=0x036F
        | 0x0483..=0x0489
        | 0x0591..=0x05C7
        | 0x0610..=0x061A
        | 0x064B..=0x065F
        | 0x0670
        | 0x06D6..=0x06ED
        | 0x0900..=0x0963
        | 0x0966..=0x0DFF
        | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF
        | 0x200C..=0x200D
        | 0x20D0..=0x20FF
        | 0xFE20..=0xFE2F)
}

/// Letters of scripts written without spaces between words.
fn unspaced(ch: char) -> bool {
    matches!(ch as u32,
        0x0E00..=0x0EFF      // Thai, Lao
        | 0x1000..=0x109F    // Myanmar
        | 0x1780..=0x17FF    // Khmer
        | 0x3040..=0x30FF    // Hiragana, Katakana
        | 0x3400..=0x4DBF    // CJK extension A
        | 0x4E00..=0x9FFF    // CJK
        | 0xF900..=0xFAFF    // CJK compatibility
        | 0x20000..=0x2FFFF) // CJK extensions
}

/// Byte ranges of the words in `text` that are not in the dictionary, nor
/// in `skip` (lowercase; nicknames and channel names, for example). URLs,
/// a leading `/command`, and short or numeric words are left alone. With
/// `finished_only`, a word still being typed at the end is not marked yet.
pub fn miss_ranges(
    text: &str,
    dictionary: &HashSet<String>,
    skip: &HashSet<String>,
    finished_only: bool,
) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if dictionary.is_empty() {
        return out;
    }
    let mut token_start = 0;
    for (token_index, token) in text.split(char::is_whitespace).enumerate() {
        let start = token_start;
        // Every whitespace character is one `char`; step over its bytes.
        token_start += token.len()
            + text[start + token.len()..]
                .chars()
                .next()
                .map_or(0, char::len_utf8);
        let lower = token.to_lowercase();
        if lower.contains("://") || lower.starts_with("www.") {
            continue;
        }
        if token_index == 0 && token.starts_with('/') {
            continue;
        }
        if token.starts_with(['#', '&']) || skip.contains(&lower) {
            continue;
        }
        // Words are runs of letters and digits, with apostrophes inside.
        let mut chars = token.char_indices().peekable();
        while let Some((word_start, first)) = chars.next() {
            if !first.is_alphanumeric() {
                continue;
            }
            let mut word_end = word_start + first.len_utf8();
            while let Some(&(at, ch)) = chars.peek() {
                if ch.is_alphanumeric() || is_apostrophe(ch) || is_mark(ch) {
                    word_end = at + ch.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            let word = token[word_start..word_end].trim_end_matches(is_apostrophe);
            let at = start + word_start;
            if finished_only && at + word.len() == text.len() {
                continue;
            }
            let lower = normalize(word);
            if word.chars().count() >= 3
                && word
                    .chars()
                    .all(|ch| ch.is_alphabetic() || is_apostrophe(ch) || is_mark(ch))
                && !word.chars().any(unspaced)
                && !dictionary.contains(&lower)
                && !skip.contains(&lower)
            {
                out.push((at, at + word.len()));
            }
        }
    }
    out
}

/// The byte range of the word around byte `at`, if there is one.
pub fn word_at(text: &str, at: usize) -> Option<(usize, usize)> {
    let is_word = |ch: char| ch.is_alphanumeric() || is_apostrophe(ch) || is_mark(ch) || ch == '-';
    let at = at.min(text.len());
    if !text.is_char_boundary(at) {
        return None;
    }
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|(_, ch)| is_word(*ch))
        .last()
        .map_or(at, |(index, _)| index);
    let end = text[at..]
        .char_indices()
        .find(|(_, ch)| !is_word(*ch))
        .map_or(text.len(), |(index, _)| at + index);
    let (start, end) = {
        let word = &text[start..end];
        let edge = |ch: char| is_apostrophe(ch) || ch == '-';
        let lead = word.len() - word.trim_start_matches(edge).len();
        let tail = word.len() - word.trim_end_matches(edge).len();
        (start + lead, end - tail)
    };
    (start < end).then_some((start, end))
}

/// Dictionary words close to `word` (at most two letters changed, added,
/// or removed), closest first.
pub fn suggestions(word: &str, dictionary: &HashSet<String>, most: usize) -> Vec<String> {
    let word = normalize(word);
    let length = word.chars().count();
    let mut found: Vec<(usize, &String)> = dictionary
        .iter()
        .filter(|item| item.chars().count().abs_diff(length) <= 2)
        .filter_map(|item| {
            let distance = distance(&word, item, 2);
            (distance <= 2).then_some((distance, item))
        })
        .collect();
    found.sort();
    found
        .into_iter()
        .take(most)
        .map(|(_, item)| item.clone())
        .collect()
}

/// Edit distance between two words, stopping early above `limit`.
fn distance(left: &str, right: &str, limit: usize) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let mut row: Vec<usize> = (0..=right.len()).collect();
    for (i, a) in left.iter().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        let mut lowest = row[0];
        for (j, b) in right.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if a == b {
                previous
            } else {
                1 + previous.min(above).min(row[j])
            };
            previous = above;
            lowest = lowest.min(row[j + 1]);
        }
        if lowest > limit {
            return limit + 1;
        }
    }
    row[right.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionaries_keep_their_encodings_and_added_words_stay_apart() {
        let dir = std::env::temp_dir().join(format!("openirc-dict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A Windows-1252 list with one UTF-8 line already added to it.
        let list = dir.join("words.txt");
        let mut bytes = b"caf\xe9\n".to_vec();
        bytes.extend("naïve\n".as_bytes());
        std::fs::write(&list, bytes).unwrap();
        let words = read(&list).unwrap();
        assert!(
            words.contains("café") && words.contains("naïve"),
            "{words:?}"
        );
        std::fs::write(added_path(&list), "größe\n").unwrap();
        assert!(read(&list).unwrap().contains("größe"));
        // A Hunspell pair in a Cyrillic code page.
        let dic = dir.join("ru.dic");
        std::fs::write(dir.join("ru.aff"), "SET CP1251\n").unwrap();
        let (cp1251, _, _) = encoding_rs::WINDOWS_1251.encode("1\nпривет/A\n");
        std::fs::write(&dic, &cp1251).unwrap();
        assert!(read(&dic).unwrap().contains("привет"));
        // A byte order mark before the SET line.
        std::fs::write(dir.join("ru.aff"), "\u{feff}SET CP1251\n").unwrap();
        assert!(read(&dic).unwrap().contains("привет"));
        // An encoding that cannot be read is said so, not guessed at.
        std::fs::write(dir.join("ru.aff"), "SET KOI-NONSENSE\n").unwrap();
        let err = read(&dic).unwrap_err();
        assert!(err.contains("KOI-NONSENSE"), "{err}");
        // A SETTINGS-like word is not a SET line.
        std::fs::write(dir.join("ru.aff"), "SETTLE x\n").unwrap();
        assert!(read(&dic).is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn flags_words_missing_from_the_dictionary() {
        let dictionary = load("hello there");
        let text = "hello thar https://example.com";
        let ranges = miss_ranges(text, &dictionary, &HashSet::new(), false);
        assert_eq!(ranges, vec![(6, 10)]);
        assert!(miss_ranges("hello there", &dictionary, &HashSet::new(), false).is_empty());
    }

    #[test]
    fn ranges_skip_nicks_commands_links_and_the_word_being_typed() {
        let dictionary = load("hello there");
        let skip: HashSet<String> = ["alicex".to_string()].into();
        let text = "/msg alicex hello thar https://exampel.com wrld";
        let ranges = miss_ranges(text, &dictionary, &skip, true);
        let words: Vec<&str> = ranges.iter().map(|(a, b)| &text[*a..*b]).collect();
        assert_eq!(words, vec!["thar"], "wrld is still being typed");
        let ranges = miss_ranges(text, &dictionary, &skip, false);
        let words: Vec<&str> = ranges.iter().map(|(a, b)| &text[*a..*b]).collect();
        assert_eq!(words, vec!["thar", "wrld"]);
        let text = "alicex: thar";
        let words: Vec<&str> = miss_ranges(text, &dictionary, &skip, false)
            .iter()
            .map(|(a, b)| &text[*a..*b])
            .collect();
        assert_eq!(words, vec!["thar"], "a nick with a colon is still a nick");
    }

    #[test]
    fn suggestions_are_close_words_closest_first() {
        let dictionary = load("there three their other hello");
        assert_eq!(suggestions("thre", &dictionary, 3), vec!["there", "three"]);
        assert_eq!(suggestions("hellp", &dictionary, 3), vec!["hello"]);
        assert!(suggestions("zzzzzz", &dictionary, 3).is_empty());
    }

    #[test]
    fn word_at_finds_the_word_under_a_position() {
        let text = "say 'hello-there' now";
        assert_eq!(
            word_at(text, 7).map(|(a, b)| &text[a..b]),
            Some("hello-there")
        );
        assert_eq!(word_at(text, 3), Some((0, 3)));
        assert_eq!(word_at("  ", 1), None);
    }

    #[test]
    fn other_alphabets_are_checked_as_whole_words() {
        let dictionary = load("café größe привет don't");
        let none = HashSet::new();
        let check = |text: &str| -> Vec<String> {
            miss_ranges(text, &dictionary, &none, false)
                .iter()
                .map(|(a, b)| text[*a..*b].to_string())
                .collect()
        };
        assert!(check("Café GRÖSSE Größe привет").contains(&"GRÖSSE".to_string()));
        assert_eq!(
            check("Café Größe Привет don\u{2019}t"),
            Vec::<String>::new()
        );
        assert_eq!(check("kafé прывет"), vec!["kafé", "прывет"]);
        assert_eq!(
            check("今日は良い天気 สวัสดีครับ"),
            Vec::<String>::new(),
            "no word spaces"
        );
        assert_eq!(suggestions("Gröse", &dictionary, 3), vec!["größe"]);
        // A virama or vowel sign does not split a Hindi word.
        let hindi = load("नमस्ते");
        let text = "नमस्ते दुनिया";
        let ranges = miss_ranges(text, &hindi, &none, false);
        let words: Vec<&str> = ranges.iter().map(|(a, b)| &text[*a..*b]).collect();
        assert_eq!(words, vec!["दुनिया"]);
        // A decomposed accent stays with its letter.
        let text = "cafe\u{301}";
        assert_eq!(
            miss_ranges(text, &dictionary, &none, false),
            vec![(0, text.len())]
        );
    }

    #[test]
    fn hunspell_dictionaries_load_their_words() {
        let dictionary = load("3\nmaison/S\nété/AB\nvoiture\n");
        assert!(dictionary.contains("maison") && dictionary.contains("été"));
        assert!(dictionary.contains("voiture"));
        assert_eq!(dictionary.len(), 3, "the count line is not a word");
    }
}
