//! User rules: highlights, ignores, nickname colors, and automatic modes.
//! Each rule is plain data with a matching function, saved as one settings
//! line of tab-separated fields.

use std::collections::HashMap;
use std::sync::Mutex;

/// Where a highlight looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MatchOn {
    #[default]
    Message,
    Nick,
    Both,
}

impl MatchOn {
    pub const ALL: [MatchOn; 3] = [MatchOn::Message, MatchOn::Nick, MatchOn::Both];

    pub fn label(self) -> &'static str {
        match self {
            MatchOn::Message => "the message",
            MatchOn::Nick => "the sender's nick",
            MatchOn::Both => "message or nick",
        }
    }

    fn key(self) -> &'static str {
        match self {
            MatchOn::Message => "message",
            MatchOn::Nick => "nick",
            MatchOn::Both => "both",
        }
    }

    fn from_key(key: &str) -> Self {
        match key {
            "nick" => MatchOn::Nick,
            "both" => MatchOn::Both,
            _ => MatchOn::Message,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighlightRule {
    /// Words or phrases separated by commas, or one regular expression.
    /// Words may use `*` and `?`; `$me` stands for your nick.
    pub words: String,
    /// Channels or nicks it applies to (wildcards, separated by commas);
    /// blank for everywhere.
    pub sources: String,
    pub on: MatchOn,
    pub case: bool,
    pub regex: bool,
    /// Palette color for the line, or `None` for the attention color.
    pub color: Option<u8>,
    /// WAV file to play; blank uses the highlight sound.
    pub sound: String,
    /// Flash the taskbar.
    pub flash: bool,
    /// Show a corner notice.
    pub notice: bool,
}

impl HighlightRule {
    pub fn words(words: &str) -> Self {
        Self {
            words: words.to_string(),
            sources: String::new(),
            on: MatchOn::Message,
            case: false,
            regex: false,
            color: None,
            sound: String::new(),
            flash: true,
            notice: true,
        }
    }

    /// Whether a message `text` from `nick` in `target` matches. `me` is
    /// your current nick, for `$me`.
    pub fn matches(&self, text: &str, nick: &str, target: &str, me: &str) -> bool {
        if !sources_match(&self.sources, nick, target) {
            return false;
        }
        let check = |haystack: &str| {
            if self.regex {
                regex_match(&self.words, self.case, haystack)
            } else {
                split_list(&self.words)
                    .into_iter()
                    .map(|word| {
                        if word.eq_ignore_ascii_case("$me") {
                            me.to_string()
                        } else {
                            word
                        }
                    })
                    .any(|word| word_match(&word, haystack, self.case))
            }
        };
        match self.on {
            MatchOn::Message => check(text),
            MatchOn::Nick => check(nick),
            MatchOn::Both => check(text) || check(nick),
        }
    }

    pub fn fields(&self) -> Vec<String> {
        vec![
            self.words.clone(),
            self.sources.clone(),
            self.on.key().into(),
            self.case.to_string(),
            self.regex.to_string(),
            self.color
                .map(|color| color.to_string())
                .unwrap_or_default(),
            self.sound.clone(),
            self.flash.to_string(),
            self.notice.to_string(),
        ]
    }

    pub fn from_fields(fields: &[String]) -> Option<Self> {
        if fields.len() != 9 {
            return None;
        }
        Some(Self {
            words: fields[0].clone(),
            sources: fields[1].clone(),
            on: MatchOn::from_key(&fields[2]),
            case: crate::settings::parse_flag(&fields[3])?,
            regex: crate::settings::parse_flag(&fields[4])?,
            color: fields[5]
                .trim()
                .parse()
                .ok()
                .filter(|color: &u8| *color < 16),
            sound: fields[6].clone(),
            flash: crate::settings::parse_flag(&fields[7])?,
            notice: crate::settings::parse_flag(&fields[8])?,
        })
    }
}

/// What an ignore rule silences. Letters as in `/ignore -pcntdik`.
pub const IGNORE_TYPES: [(char, &str); 7] = [
    ('p', "Private messages"),
    ('c', "Channel messages"),
    ('n', "Notices"),
    ('t', "CTCP requests"),
    ('d', "DCC offers"),
    ('i', "Invites"),
    ('k', "Colors and styles (strip them)"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IgnoreRule {
    /// `nick!user@host` with wildcards; a bare nick means `nick!*@*`.
    pub mask: String,
    /// Letters from [`IGNORE_TYPES`].
    pub types: String,
    /// Network or server it applies to; blank for all.
    pub network: String,
    /// Unix time it stops applying, or 0 for never.
    pub expires: u64,
}

impl IgnoreRule {
    /// Ignore everything from `mask`.
    pub fn all(mask: &str) -> Self {
        Self {
            mask: mask.to_string(),
            types: "pcntdi".into(),
            network: String::new(),
            expires: 0,
        }
    }

    /// The full mask: a bare nick becomes `nick!*@*`.
    pub fn full_mask(&self) -> String {
        full_mask(&self.mask)
    }

    pub fn active(&self, now: u64) -> bool {
        self.expires == 0 || now < self.expires
    }

    /// Whether this rule silences `kind` (a letter from [`IGNORE_TYPES`])
    /// from `source` (`nick!user@host`) on `network`.
    pub fn applies(&self, source: &str, kind: char, network: &str, now: u64) -> bool {
        self.active(now)
            && self.types.contains(kind)
            && network_matches(&self.network, network)
            && crate::script::mask_match(&self.full_mask(), source)
    }

    pub fn fields(&self) -> Vec<String> {
        vec![
            self.mask.clone(),
            self.types.clone(),
            self.network.clone(),
            self.expires.to_string(),
        ]
    }

    pub fn from_fields(fields: &[String]) -> Option<Self> {
        if fields.len() != 4 || fields[0].trim().is_empty() {
            return None;
        }
        Some(Self {
            mask: fields[0].clone(),
            types: fields[1].clone(),
            network: fields[2].clone(),
            expires: fields[3].trim().parse().unwrap_or(0),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NickColorRule {
    /// Nick mask with wildcards; blank for anyone.
    pub mask: String,
    /// Status prefixes it applies to, such as `@` or `@%`; `*` for anyone
    /// with a status, `-` for no status, blank for any.
    pub modes: String,
    pub color: u8,
}

impl NickColorRule {
    pub fn matches(&self, nick: &str, prefix: &str) -> bool {
        let modes = self.modes.trim();
        let mode_ok = match modes {
            "" => true,
            "*" => !prefix.is_empty(),
            "-" => prefix.is_empty(),
            wanted => prefix.chars().any(|ch| wanted.contains(ch)),
        };
        mode_ok
            && (self.mask.trim().is_empty() || crate::script::mask_match(self.mask.trim(), nick))
    }

    pub fn fields(&self) -> Vec<String> {
        vec![
            self.mask.clone(),
            self.modes.clone(),
            self.color.to_string(),
        ]
    }

    pub fn from_fields(fields: &[String]) -> Option<Self> {
        if fields.len() != 3 {
            return None;
        }
        Some(Self {
            mask: fields[0].clone(),
            modes: fields[1].clone(),
            color: fields[2]
                .trim()
                .parse()
                .ok()
                .filter(|color: &u8| *color < 16)?,
        })
    }
}

/// Whether a rule for `wanted` networks (blank for all) covers `network`.
/// A rule for particular networks never applies while the network's name
/// is still unknown.
fn network_matches(wanted: &str, network: &str) -> bool {
    let wanted = wanted.trim();
    wanted.is_empty() || (!network.is_empty() && crate::script::mask_match(wanted, network))
}

/// Give someone a channel status automatically when they join.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoMode {
    /// `nick!user@host` with wildcards; a bare nick means `nick!*@*`.
    pub mask: String,
    /// Channels (wildcards, separated by commas); blank for all.
    pub channels: String,
    pub network: String,
    /// `o` op, `h` halfop, `v` voice.
    pub mode: char,
    /// Op them again if someone takes their status away.
    pub protect: bool,
}

impl AutoMode {
    pub fn applies(&self, source: &str, channel: &str, network: &str) -> bool {
        crate::script::mask_match(&full_mask(&self.mask), source)
            && sources_match(&self.channels, "", channel)
            && network_matches(&self.network, network)
    }

    pub fn fields(&self) -> Vec<String> {
        vec![
            self.mask.clone(),
            self.channels.clone(),
            self.network.clone(),
            self.mode.to_string(),
            self.protect.to_string(),
        ]
    }

    pub fn from_fields(fields: &[String]) -> Option<Self> {
        if fields.len() != 5 || fields[0].trim().is_empty() {
            return None;
        }
        let mode = fields[3]
            .chars()
            .next()
            .filter(|mode| matches!(mode, 'o' | 'h' | 'v'))?;
        Some(Self {
            mask: fields[0].clone(),
            channels: fields[1].clone(),
            network: fields[2].clone(),
            mode,
            protect: crate::settings::parse_flag(&fields[4])?,
        })
    }
}

/// A bare nick becomes `nick!*@*`; a mask with `!` or `@` is kept.
pub fn full_mask(mask: &str) -> String {
    let mask = mask.trim();
    if mask.contains('!') || mask.contains('@') {
        mask.to_string()
    } else {
        format!("{mask}!*@*")
    }
}

fn split_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

/// Blank `sources` match everything; otherwise the channel or the nick
/// must match one of the masks.
fn sources_match(sources: &str, nick: &str, target: &str) -> bool {
    let masks = split_list(sources);
    masks.is_empty()
        || masks.iter().any(|mask| {
            crate::script::mask_match(mask, target)
                || (!nick.is_empty() && crate::script::mask_match(mask, nick))
        })
}

/// A word or phrase with word boundaries, or a wildcard pattern.
fn word_match(word: &str, haystack: &str, case: bool) -> bool {
    if word.contains(['*', '?']) {
        // A pattern matches one word of the message, or the whole message
        // when it spans several words.
        let one = |text: &str| {
            if case {
                wildcard_case(word, text)
            } else {
                crate::script::mask_match(word, text)
            }
        };
        return one(haystack)
            || haystack
                .split_whitespace()
                .map(|token| {
                    token.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '*' && ch != '?')
                })
                .any(one);
    }
    if case {
        bounded(haystack, word)
    } else {
        crate::highlight::first_match(haystack, std::slice::from_ref(&word.to_string())).is_some()
    }
}

fn bounded(haystack: &str, word: &str) -> bool {
    haystack.match_indices(word).any(|(at, found)| {
        let before = haystack[..at].chars().next_back();
        let after = haystack[at + found.len()..].chars().next();
        before.is_none_or(|ch| !ch.is_alphanumeric())
            && after.is_none_or(|ch| !ch.is_alphanumeric())
    })
}

fn wildcard_case(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((star_p, star_t)) = star {
            p = star_p + 1;
            t = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&ch| ch == '*')
}

/// Compiled patterns by pattern text and case sensitivity; `None` for a
/// pattern that does not compile.
type RegexCache = HashMap<(String, bool), Option<regex::Regex>>;

/// Compiled regular expressions, kept so a rule is not compiled again for
/// every message. A pattern that does not compile never matches.
fn regex_match(pattern: &str, case: bool, haystack: &str) -> bool {
    static CACHE: Mutex<Option<RegexCache>> = Mutex::new(None);
    let Ok(mut cache) = CACHE.lock() else {
        return false;
    };
    let cache = cache.get_or_insert_with(HashMap::new);
    let key = (pattern.to_string(), case);
    let compiled = cache.entry(key).or_insert_with(|| {
        regex::RegexBuilder::new(pattern)
            .case_insensitive(!case)
            .size_limit(1 << 20)
            .build()
            .ok()
    });
    compiled.as_ref().is_some_and(|re| re.is_match(haystack))
}

/// Whether a regular expression compiles, for the rule editor.
pub fn regex_error(pattern: &str) -> Option<String> {
    regex::Regex::new(pattern).err().map(|err| err.to_string())
}

/// Parse `/ignore` arguments: `[-pcntdik] [-uSECONDS] [-wNETWORK] mask`.
/// Without type letters, everything but color stripping is ignored.
pub fn parse_ignore(args: &str, now: u64) -> Result<IgnoreRule, String> {
    let mut types = String::new();
    let mut expires = 0;
    let mut network = String::new();
    let mut mask = None;
    for word in args.split_whitespace() {
        if let Some(flags) = word.strip_prefix('-') {
            if let Some(seconds) = flags.strip_prefix('u') {
                let seconds: u64 = seconds
                    .parse()
                    .map_err(|_| format!("-u needs a number of seconds, not `{seconds}`"))?;
                expires = now + seconds;
            } else if let Some(name) = flags.strip_prefix('w') {
                network = name.to_string();
            } else {
                for flag in flags.chars() {
                    if !IGNORE_TYPES.iter().any(|(letter, _)| *letter == flag) {
                        return Err(format!("unknown ignore type -{flag}"));
                    }
                    types.push(flag);
                }
            }
        } else if mask.is_none() {
            mask = Some(word.to_string());
        } else {
            return Err("give one mask".into());
        }
    }
    let mask = mask.ok_or("usage: /ignore [-pcntdik] [-uSECONDS] [-wNETWORK] <nick or mask>")?;
    Ok(IgnoreRule {
        mask,
        types: if types.is_empty() {
            "pcntdi".into()
        } else {
            types
        },
        network,
        expires,
    })
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_words_wildcards_me_sources_and_regex() {
        let mut rule = HighlightRule::words("rust, good night, open*irc, $me");
        assert!(rule.matches("I love Rust!", "bob", "#c", "me"));
        assert!(!rule.matches("trusty", "bob", "#c", "me"));
        assert!(rule.matches("well good night all", "bob", "#c", "me"));
        assert!(rule.matches("try openfooirc", "bob", "#c", "me"));
        assert!(rule.matches("hi tester", "bob", "#c", "tester"));
        rule.sources = "#work, alice".into();
        assert!(!rule.matches("rust", "bob", "#c", "me"));
        assert!(rule.matches("rust", "bob", "#work", "me"));
        assert!(rule.matches("rust", "alice", "#c", "me"));
        let mut regex = HighlightRule::words(r"\bdeploy(ed|ing)?\b");
        regex.regex = true;
        assert!(regex.matches("we DEPLOYED it", "x", "#c", "me"));
        regex.case = true;
        assert!(!regex.matches("we DEPLOYED it", "x", "#c", "me"));
        let mut nick = HighlightRule::words("*bot");
        nick.on = MatchOn::Nick;
        assert!(nick.matches("hello", "chanbot", "#c", "me"));
        assert!(!nick.matches("a bot", "carol", "#c", "me"));
        assert_eq!(HighlightRule::from_fields(&regex.fields()), Some(regex));
        assert!(regex_error("(").is_some());
    }

    #[test]
    fn ignore_rules_by_type_network_and_time() {
        let mut rule = IgnoreRule::all("spammer");
        assert!(rule.applies("spammer!u@h", 'p', "Libera", 100));
        assert!(rule.applies("SPAMMER!u@h", 'c', "Libera", 100));
        assert!(!rule.applies("spammer!u@h", 'k', "Libera", 100));
        rule.network = "OFTC".into();
        assert!(!rule.applies("spammer!u@h", 'p', "Libera", 100));
        rule.network.clear();
        rule.expires = 50;
        assert!(!rule.applies("spammer!u@h", 'p', "Libera", 100));
        let host = IgnoreRule {
            mask: "*!*@*.spam.example".into(),
            types: "n".into(),
            network: String::new(),
            expires: 0,
        };
        assert!(host.applies("x!y@a.spam.example", 'n', "", 1));
        assert!(!host.applies("x!y@a.spam.example", 'p', "", 1));
        assert_eq!(IgnoreRule::from_fields(&host.fields()), Some(host));
    }

    #[test]
    fn ignore_command_arguments() {
        let rule = parse_ignore("-pn -u60 -wLibera troll", 1000).unwrap();
        assert_eq!(rule.mask, "troll");
        assert_eq!(rule.types, "pn");
        assert_eq!(rule.expires, 1060);
        assert_eq!(rule.network, "Libera");
        assert_eq!(parse_ignore("bob", 0).unwrap().types, "pcntdi");
        assert!(parse_ignore("-z bob", 0).is_err());
        assert!(parse_ignore("", 0).is_err());
    }

    #[test]
    fn nick_colors_and_auto_modes() {
        let ops = NickColorRule {
            mask: String::new(),
            modes: "@".into(),
            color: 4,
        };
        assert!(ops.matches("alice", "@"));
        assert!(!ops.matches("bob", "+"));
        let plain = NickColorRule {
            mask: "guest*".into(),
            modes: "-".into(),
            color: 14,
        };
        assert!(plain.matches("Guest42", ""));
        assert!(!plain.matches("Guest42", "+"));
        assert_eq!(NickColorRule::from_fields(&plain.fields()), Some(plain));
        let auto = AutoMode {
            mask: "friend".into(),
            channels: "#home".into(),
            network: String::new(),
            mode: 'o',
            protect: true,
        };
        assert!(auto.applies("friend!f@h", "#home", "Net"));
        assert!(!auto.applies("friend!f@h", "#other", "Net"));
        assert_eq!(AutoMode::from_fields(&auto.fields()), Some(auto));
    }
}
