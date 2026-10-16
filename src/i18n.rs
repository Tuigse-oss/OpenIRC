//! Translations of the menus, dialogs, and options.
//!
//! The English text in the source is the key: `tr("Quit")` gives the text
//! for the chosen language, or the English text when there is no entry, so
//! a missing line never hides anything. Catalogs are plain text, one entry
//! per line, the English and the translation separated by a tab (shown
//! here as `<TAB>`):
//!
//! ```text
//! # Lines starting with # and without a tab are notes.
//! @language<TAB>Deutsch
//! Quit<TAB>Beenden
//! Could not read {0}: {1}<TAB>{0} konnte nicht gelesen werden: {1}
//! ```
//!
//! German, Spanish, French, and Brazilian Portuguese are built in. Anyone
//! can add a language, or fix one, by putting `<code>.txt` in the `lang`
//! folder of the OpenIRC settings folder; a file there wins over the
//! built-in one with the same code.
//!
//! The chosen language is kept per thread: the window runs on one thread,
//! and tests on other threads stay in English.

use std::cell::RefCell;
use std::collections::HashMap;

/// Built-in catalogs: code and file text.
const BUILT_IN: [(&str, &str); 4] = [
    ("de", include_str!("../lang/de.txt")),
    ("es", include_str!("../lang/es.txt")),
    ("fr", include_str!("../lang/fr.txt")),
    ("pt-BR", include_str!("../lang/pt-BR.txt")),
];

thread_local! {
    static TABLE: RefCell<Table> = RefCell::new(Table::default());
}

#[derive(Default)]
struct Table {
    code: String,
    /// English text to translation. Entries live as long as the program:
    /// a catalog is a few tens of kilobytes and is loaded only when the
    /// language changes.
    entries: HashMap<&'static str, &'static str>,
}

/// The text for `english` in the current language.
pub fn tr(english: &str) -> &str {
    TABLE.with(|table| {
        table
            .borrow()
            .entries
            .get(english)
            .copied()
            .unwrap_or(english)
    })
}

/// Marks text for translation where `tr` cannot run (in a `const`); the
/// text is translated with `tr` where it is shown.
pub const fn tr_key(english: &'static str) -> &'static str {
    english
}

/// Fill `{0}`, `{1}`, ... in text that is already translated (on a
/// background thread, which has no language of its own, translate the text
/// before starting it).
pub fn fill(text: &str, args: &[&dyn std::fmt::Display]) -> String {
    // One pass over the text: a value that itself looks like `{1}` (a nick
    // can) is put in as it is and never filled again.
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let filled = after.find('}').and_then(|close| {
            let index: usize = after[..close].parse().ok()?;
            let arg = args.get(index)?;
            Some((arg.to_string(), close))
        });
        match filled {
            Some((value, close)) => {
                out.push_str(&value);
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `tr` for text with `{0}`, `{1}`, ... filled in after translating, so a
/// translation can put them in another order.
pub fn trf(english: &str, args: &[&dyn std::fmt::Display]) -> String {
    fill(tr(english), args)
}

/// The language now in use: a code such as `de`, or empty for English.
pub fn current() -> String {
    TABLE.with(|table| table.borrow().code.clone())
}

/// Switch to `code` (`""` or `auto` follows the system, `en` is English).
/// Returns a problem to show, if the catalog could not be used.
pub fn set_language(code: &str) -> Option<String> {
    let wanted = match code.trim() {
        "" | "auto" => system_language(),
        other => other.to_string(),
    };
    let resolved = resolve(&wanted);
    if resolved == current() {
        return None;
    }
    let (entries, problem) = match resolved.as_str() {
        "" => (HashMap::new(), None),
        code => match catalog_text(code) {
            Some(text) => {
                let (parsed, bad) = parse(&text);
                let problem = (!bad.is_empty()).then(|| {
                    format!(
                        "The {code} translation has lines without a tab, which were skipped: {}",
                        bad.iter()
                            .map(|line| line.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                });
                (kept(code, &text, parsed), problem)
            }
            None => (
                HashMap::new(),
                Some(format!("There is no {code} translation.")),
            ),
        },
    };
    TABLE.with(|table| {
        *table.borrow_mut() = Table {
            code: resolved,
            entries,
        };
    });
    problem
}

/// The best catalog code for a wanted language: an exact match, else the
/// same language in another region (`pt-PT` uses `pt-BR`), else English.
fn resolve(wanted: &str) -> String {
    let wanted = wanted.replace('_', "-");
    let wanted = wanted.split(['.', '@']).next().unwrap_or_default();
    if wanted.is_empty() || wanted.eq_ignore_ascii_case("en") || wanted.starts_with("en-") {
        return String::new();
    }
    let codes: Vec<String> = available().into_iter().map(|(code, _)| code).collect();
    if let Some(code) = codes.iter().find(|code| code.eq_ignore_ascii_case(wanted)) {
        return code.clone();
    }
    let base = wanted.split('-').next().unwrap_or_default();
    codes
        .into_iter()
        .find(|code| {
            code.split('-')
                .next()
                .is_some_and(|start| start.eq_ignore_ascii_case(base))
        })
        .unwrap_or_default()
}

/// Every language on offer: code and name in that language, English first.
pub fn available() -> Vec<(String, String)> {
    // The Options page asks every frame; read the folder at most every few
    // seconds (a file dropped in still shows up quickly).
    type Listed = Vec<(String, String)>;
    thread_local! {
        static SEEN: RefCell<Option<(std::time::Instant, Listed)>> = const { RefCell::new(None) };
    }
    SEEN.with(|seen| {
        let mut seen = seen.borrow_mut();
        if let Some((when, list)) = seen.as_ref()
            && when.elapsed() < std::time::Duration::from_secs(5)
        {
            return list.clone();
        }
        let list = read_available();
        *seen = Some((std::time::Instant::now(), list.clone()));
        list
    })
}

fn read_available() -> Vec<(String, String)> {
    let mut list = vec![(String::from("en"), String::from("English"))];
    let mut add = |code: String, text: &str| {
        let name = language_name(text).unwrap_or_else(|| code.clone());
        match list.iter_mut().find(|(known, _)| *known == code) {
            Some(entry) => entry.1 = name,
            None => list.push((code, name)),
        }
    };
    for (code, text) in BUILT_IN {
        add(code.to_string(), text);
    }
    if let Ok(entries) = std::fs::read_dir(user_folder()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "txt")
                && let Some(code) = path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().to_string())
                && let Ok(bytes) = std::fs::read(&path)
            {
                add(code, &crate::message::decode_line(&bytes));
            }
        }
    }
    list
}

fn user_folder() -> std::path::PathBuf {
    crate::settings::config_dir().join("lang")
}

fn catalog_text(code: &str) -> Option<String> {
    let path = user_folder().join(format!("{code}.txt"));
    if let Ok(bytes) = std::fs::read(&path) {
        return Some(crate::message::decode_line(&bytes));
    }
    BUILT_IN
        .iter()
        .find(|(known, _)| *known == code)
        .map(|(_, text)| text.to_string())
}

fn language_name(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix("@language\t"))
        .map(|name| name.trim().to_string())
}

/// Entries of a catalog, and the numbers of lines that could not be read.
fn parse(text: &str) -> (Vec<(String, String)>, Vec<usize>) {
    let mut entries = Vec::new();
    let mut bad = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        // A note is a `#` line without a tab; texts may start with `#` or
        // `@` themselves ("#channel", "@ for ops").
        let note = line.starts_with('#') && !line.contains('\t');
        if line.trim().is_empty() || note || line.starts_with("@language\t") {
            continue;
        }
        match line.split_once('\t') {
            // Spaces at the start of a text are part of it (" on {0}").
            Some((english, translated)) if !english.is_empty() && !translated.trim().is_empty() => {
                entries.push((unescape(english), unescape(translated.trim_end())));
            }
            _ => bad.push(index + 1),
        }
    }
    (entries, bad)
}

/// `\n` in a catalog stands for a line break.
fn unescape(text: &str) -> String {
    text.replace("\\n", "\n")
}

type Entries = HashMap<&'static str, &'static str>;

/// The table for a catalog, made once for each catalog text and reused, so
/// switching languages back and forth does not keep using more memory.
fn kept(code: &str, text: &str, parsed: Vec<(String, String)>) -> Entries {
    use std::hash::{Hash, Hasher};
    thread_local! {
        static MADE: RefCell<HashMap<(String, u64), Entries>> = RefCell::new(HashMap::new());
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    let key = (code.to_string(), hasher.finish());
    MADE.with(|made| {
        made.borrow_mut()
            .entry(key)
            .or_insert_with(|| leak(parsed))
            .clone()
    })
}

fn leak(entries: Vec<(String, String)>) -> Entries {
    entries
        .into_iter()
        .map(|(english, translated)| {
            (
                &*Box::leak(english.into_boxed_str()),
                &*Box::leak(translated.into_boxed_str()),
            )
        })
        .collect()
}

/// The system's language, such as `de-DE`, or empty if unknown.
pub fn system_language() -> String {
    #[cfg(windows)]
    {
        let mut buffer = [0u16; 85];
        // SAFETY: the buffer is LOCALE_NAME_MAX_LENGTH wide characters, and
        // the call writes at most that many.
        let length = unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), buffer.len() as i32) };
        if length > 1 {
            return String::from_utf16_lossy(&buffer[..length as usize - 1]);
        }
        String::new()
    }
    #[cfg(not(windows))]
    {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|key| std::env::var(key).ok())
            .find(|value| !value.is_empty())
            // The first one set wins; `C` and `POSIX` there mean English.
            .filter(|value| value != "C" && value != "POSIX" && !value.starts_with("C."))
            .unwrap_or_default()
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetUserDefaultLocaleName(name: *mut u16, length: i32) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_shaped_values_are_put_in_as_they_are() {
        assert_eq!(
            fill("{0} kicked {1} from {2}", &[&"{1}", &"Alice", &"#room"]),
            "{1} kicked Alice from #room"
        );
        assert_eq!(fill("{1} then {0}", &[&"a", &"b"]), "b then a");
        assert_eq!(fill("{x} {9} {", &[&"a"]), "{x} {9} {");
        assert_eq!(fill("café {0}", &[&"ü"]), "café ü");
    }

    /// The window code, where every text a person reads is translated.
    const GUI_SOURCES: [(&str, &str); 16] = [
        ("ui.rs", include_str!("ui.rs")),
        ("ui/channel.rs", include_str!("ui/channel.rs")),
        ("ui/chrome.rs", include_str!("ui/chrome.rs")),
        ("ui/icons.rs", include_str!("ui/icons.rs")),
        ("ui/input.rs", include_str!("ui/input.rs")),
        ("ui/logs.rs", include_str!("ui/logs.rs")),
        ("ui/monitor.rs", include_str!("ui/monitor.rs")),
        ("ui/prompts.rs", include_str!("ui/prompts.rs")),
        ("ui/rules.rs", include_str!("ui/rules.rs")),
        ("ui/scripts.rs", include_str!("ui/scripts.rs")),
        ("ui/search.rs", include_str!("ui/search.rs")),
        ("ui/servers.rs", include_str!("ui/servers.rs")),
        ("ui/toast.rs", include_str!("ui/toast.rs")),
        ("ui/transfers.rs", include_str!("ui/transfers.rs")),
        ("ui/view.rs", include_str!("ui/view.rs")),
        ("tray.rs", include_str!("tray.rs")),
    ];

    /// Widgets, tool tips, notices and window titles given words as plain
    /// text or through `format!` instead of `tr`, as their first argument.
    /// The catalog test only sees marked texts; this finds unmarked ones
    /// written straight into those calls. It cannot see text that reaches a
    /// widget through a variable or a later argument, so it is a guard, not
    /// proof that every window text is translated. Command output typed
    /// lines get back (`note_active`) stays English and is not looked at,
    /// nor are examples in empty fields (`hint_text`).
    #[test]
    fn widget_calls_never_take_plain_words() {
        const CALLS: [&str; 14] = [
            "on_hover_text(",
            "on_disabled_hover_text(",
            ".label(",
            ".button(",
            ".small_button(",
            ".checkbox(",
            ".heading(",
            "selectable_label(",
            "RichText::new(",
            "Button::new(",
            "Window::new(",
            "menu_button(",
            ".tip(",
            "note_status(",
        ];
        // Proper names and protocol words, the same in every language.
        const KEPT: [&str; 6] = [
            "OpenIRC",
            "TLS certificate SHA-256 {fingerprint}",
            "PING? PONG!",
            "{host} — SHA-256",
            "on {} {} {}",
            "Script: {err}",
        ];
        let mut found = Vec::new();
        for (file, source) in GUI_SOURCES {
            let body = source
                .find("#[cfg(test)]\nmod tests")
                .map_or(source, |end| &source[..end]);
            for call in CALLS {
                for (at, _) in body.match_indices(call) {
                    let rest = body[at + call.len()..].trim_start();
                    let rest = rest.strip_prefix("format!(").map_or(rest, str::trim_start);
                    let Some(text) = rest.strip_prefix('"') else {
                        continue;
                    };
                    let text = &text[..text.find('"').unwrap_or(text.len())];
                    let mut words = String::new();
                    let mut depth = 0;
                    for ch in text.chars() {
                        match ch {
                            '{' => depth += 1,
                            '}' => depth -= 1,
                            ch if depth == 0 => words.push(ch),
                            _ => {}
                        }
                    }
                    let wordy = words
                        .split(|ch: char| !ch.is_ascii_alphabetic())
                        .any(|word| word.len() >= 2);
                    if wordy && !KEPT.contains(&text) {
                        let line = body[..at].lines().count();
                        found.push(format!("{file}:{line}: {text}"));
                    }
                }
            }
        }
        assert!(found.is_empty(), "untranslated: {found:#?}");
    }

    /// Every `tr("...")` and `trf("...", ...)` text in the program.
    fn keys_in_source() -> Vec<String> {
        let sources = [
            include_str!("ui.rs"),
            include_str!("ui/channel.rs"),
            include_str!("ui/chrome.rs"),
            include_str!("ui/commands.rs"),
            include_str!("ui/icons.rs"),
            include_str!("ui/input.rs"),
            include_str!("ui/logs.rs"),
            include_str!("ui/monitor.rs"),
            include_str!("ui/prompts.rs"),
            include_str!("ui/rules.rs"),
            include_str!("ui/scripts.rs"),
            include_str!("ui/search.rs"),
            include_str!("ui/servers.rs"),
            include_str!("ui/toast.rs"),
            include_str!("ui/transfers.rs"),
            include_str!("ui/view.rs"),
            include_str!("settings.rs"),
            include_str!("fonts.rs"),
            include_str!("theme.rs"),
            include_str!("tray.rs"),
        ];
        let mut keys = Vec::new();
        for source in sources {
            for start in ["tr(", "trf(", "tr_key("] {
                let mut rest = source;
                while let Some(at) = rest.find(start) {
                    // Only whole names: not `str(` or `attr(`.
                    let before = rest[..at].chars().next_back();
                    rest = &rest[at + start.len()..];
                    if before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_') {
                        continue;
                    }
                    // The text may be on the next line.
                    let Some(text) = rest.trim_start().strip_prefix('"') else {
                        continue;
                    };
                    rest = text;
                    let mut key = String::new();
                    let mut chars = rest.chars();
                    while let Some(ch) = chars.next() {
                        match ch {
                            '"' => break,
                            '\\' => match chars.next() {
                                Some('n') => key.push('\n'),
                                // A line continuation: the break and the
                                // next line's indent are not in the text.
                                Some('\n') => {
                                    let mut peek = chars.clone();
                                    while peek.next().is_some_and(char::is_whitespace) {
                                        chars.next();
                                    }
                                }
                                Some('u') => {
                                    let code: String = chars
                                        .by_ref()
                                        .skip(1)
                                        .take_while(|ch| *ch != '}')
                                        .collect();
                                    if let Some(ch) =
                                        u32::from_str_radix(&code, 16).ok().and_then(char::from_u32)
                                    {
                                        key.push(ch);
                                    }
                                }
                                Some(other) => key.push(other),
                                None => {}
                            },
                            _ => key.push(ch),
                        }
                    }
                    if !keys.contains(&key) {
                        keys.push(key);
                    }
                }
            }
            // `plural(count, "{0} user", "{0} users")`: the two texts.
            let mut rest = source;
            while let Some(at) = rest.find("plural(") {
                rest = &rest[at + "plural(".len()..];
                // Up to the call's own closing parenthesis.
                let mut depth = 0;
                let end = rest
                    .char_indices()
                    .find(|(_, ch)| match ch {
                        '(' => {
                            depth += 1;
                            false
                        }
                        ')' if depth == 0 => true,
                        ')' => {
                            depth -= 1;
                            false
                        }
                        _ => false,
                    })
                    .map_or(rest.len(), |(at, _)| at);
                let call = &rest[..end];
                for (index, part) in call.split('"').enumerate() {
                    if index % 2 == 1 && !keys.iter().any(|key| key == part) {
                        keys.push(part.to_string());
                    }
                }
            }
        }
        keys
    }

    #[test]
    fn every_built_in_catalog_translates_every_text() {
        let keys = keys_in_source();
        assert!(keys.len() > 300, "only {} texts found", keys.len());
        for (code, text) in BUILT_IN {
            let (entries, bad) = parse(text);
            assert!(bad.is_empty(), "{code}: unreadable lines {bad:?}");
            let known: HashMap<String, String> = entries.into_iter().collect();
            let missing: Vec<&String> = keys
                .iter()
                .filter(|key| !known.contains_key(*key))
                .collect();
            assert!(
                missing.is_empty(),
                "{code} lacks {} texts, for example {:?}",
                missing.len(),
                &missing[..missing.len().min(10)]
            );
            let stale: Vec<&String> = known.keys().filter(|key| !keys.contains(key)).collect();
            assert!(
                stale.is_empty(),
                "{code} has texts no longer used: {stale:?}"
            );
            for (english, translated) in &known {
                for index in 0..4 {
                    let mark = format!("{{{index}}}");
                    assert_eq!(
                        english.contains(&mark),
                        translated.contains(&mark),
                        "{code}: {english:?} and {translated:?} differ in {mark}"
                    );
                }
            }
        }
    }

    #[test]
    fn languages_switch_and_fall_back() {
        assert_eq!(tr("Quit"), "Quit");
        assert_eq!(set_language("de-AT"), None, "German for Austria");
        assert_eq!(current(), "de");
        assert_eq!(tr("Quit"), "Beenden");
        assert_eq!(tr("text with no entry"), "text with no entry");
        assert_eq!(set_language("pt_PT.UTF-8"), None);
        assert_eq!(current(), "pt-BR");
        set_language("en");
        assert_eq!(tr("Quit"), "Quit");
        assert_eq!(resolve("ja"), "", "no Japanese catalog yet");
    }

    #[test]
    fn placeholders_can_move() {
        let (entries, bad) = parse("@language\tTest\nA {0} b {1}\t{1} x {0}\nno tab here\n");
        assert_eq!(bad, vec![3]);
        let table = leak(entries);
        TABLE.with(|current| {
            *current.borrow_mut() = Table {
                code: "test".into(),
                entries: table,
            };
        });
        assert_eq!(trf("A {0} b {1}", &[&1, &"two"]), "two x 1");
        TABLE.with(|current| *current.borrow_mut() = Table::default());
    }
}
