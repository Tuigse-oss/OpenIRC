//! Channel membership, topics, and the server's ISUPPORT case mapping.
//!
//! Nick and channel identity uses the casemap advertised in numeric 005.
//! The default is RFC 1459, where `[]\\` and `{}|` are the same letters.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseMap {
    Ascii,
    Rfc1459,
    StrictRfc1459,
}

impl CaseMap {
    pub fn fold(self, name: &str) -> String {
        name.chars().map(|ch| self.fold_char(ch)).collect()
    }

    /// Whether the server treats the two names as one.
    pub fn same(self, left: &str, right: &str) -> bool {
        left.chars().count() == right.chars().count()
            && left
                .chars()
                .zip(right.chars())
                .all(|(a, b)| self.fold_char(a) == self.fold_char(b))
    }

    fn fold_char(self, ch: char) -> char {
        let lower = if ch.is_ascii_uppercase() {
            ch.to_ascii_lowercase()
        } else {
            ch
        };
        match self {
            CaseMap::Ascii => lower,
            // `[]\~` are the capitals of `{}|^`.
            CaseMap::Rfc1459 => match lower {
                '[' => '{',
                ']' => '}',
                '\\' => '|',
                '~' => '^',
                other => other,
            },
            // The same without `~` and `^`.
            CaseMap::StrictRfc1459 => match lower {
                '[' => '{',
                ']' => '}',
                '\\' => '|',
                other => other,
            },
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "ascii" => CaseMap::Ascii,
            "strict-rfc1459" => CaseMap::StrictRfc1459,
            _ => CaseMap::Rfc1459,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub nick: String,
    pub prefix: String,
}

#[derive(Debug, Clone)]
struct Channel {
    name: String,
    topic: Option<String>,
    topic_known: bool,
    members: BTreeMap<String, Member>,
    names_open: bool,
    /// Channel flags such as `n`, `t`, and `k`, with their argument if any.
    modes: BTreeMap<char, Option<String>>,
}

#[derive(Debug, Clone)]
pub struct Directory {
    case: CaseMap,
    /// Mode letter paired with its nick prefix, highest privilege first.
    prefixes: Vec<(char, char)>,
    chantypes: String,
    list_modes: String,
    always_param: String,
    set_param: String,
    never_param: String,
    channels: BTreeMap<String, Channel>,
    order: Vec<String>,
}

impl Directory {
    pub fn new() -> Self {
        Self {
            case: CaseMap::Rfc1459,
            prefixes: vec![('o', '@'), ('v', '+')],
            chantypes: "#&+!".into(),
            list_modes: "b".into(),
            always_param: "k".into(),
            set_param: "l".into(),
            never_param: "imnpst".into(),
            channels: BTreeMap::new(),
            order: Vec::new(),
        }
    }

    /// A nick or channel name folded with the server's case rules.
    pub fn fold(&self, name: &str) -> String {
        self.case.fold(name)
    }

    pub fn same(&self, left: &str, right: &str) -> bool {
        self.case.fold(left) == self.case.fold(right)
    }

    pub fn is_channel(&self, name: &str) -> bool {
        name.chars()
            .next()
            .is_some_and(|ch| self.chantypes.contains(ch))
    }

    pub fn apply_support(&mut self, tokens: &[String]) {
        for token in tokens {
            let Some((key, value)) = token.split_once('=') else {
                continue;
            };
            match key {
                "CASEMAPPING" => self.case = CaseMap::parse(value),
                "CHANTYPES" => {
                    if !value.is_empty() {
                        self.chantypes = value.to_string();
                    }
                }
                "PREFIX" => {
                    if let Some(parsed) = parse_prefix_map(value) {
                        self.prefixes = parsed;
                    }
                }
                "CHANMODES" => {
                    let mut parts = value.split(',');
                    if let Some(list) = parts.next() {
                        self.list_modes = list.to_string();
                    }
                    if let Some(always) = parts.next() {
                        self.always_param = always.to_string();
                    }
                    if let Some(set_param) = parts.next() {
                        self.set_param = set_param.to_string();
                    }
                    if let Some(never) = parts.next() {
                        self.never_param = never.to_string();
                    }
                }
                _ => {}
            }
        }
    }

    pub fn joined(&mut self, channel: &str, nick: &str) {
        let key = self.case.fold(channel);
        if !self.channels.contains_key(&key) {
            self.channels.insert(
                key.clone(),
                Channel {
                    name: channel.to_string(),
                    topic: None,
                    topic_known: false,
                    members: BTreeMap::new(),
                    names_open: false,
                    modes: BTreeMap::new(),
                },
            );
            self.order.push(key.clone());
        }
        self.insert_member(&key, nick, "");
    }

    pub fn remove_member(&mut self, channel: &str, nick: &str) {
        let key = self.case.fold(channel);
        if let Some(room) = self.channels.get_mut(&key) {
            room.members.remove(&self.case.fold(nick));
        }
    }

    /// Removes the channel when this nick is the one that left.
    pub fn left(&mut self, channel: &str) -> Option<String> {
        let key = self.case.fold(channel);
        self.order.retain(|item| item != &key);
        self.channels.remove(&key).map(|room| room.name)
    }

    pub fn quit(&mut self, nick: &str) {
        let folded = self.case.fold(nick);
        for room in self.channels.values_mut() {
            room.members.remove(&folded);
        }
    }

    pub fn rename(&mut self, old: &str, new: &str) {
        let from = self.case.fold(old);
        let to = self.case.fold(new);
        for room in self.channels.values_mut() {
            if let Some(mut member) = room.members.remove(&from) {
                member.nick = new.to_string();
                room.members.insert(to.clone(), member);
            }
        }
    }

    pub fn set_topic(&mut self, channel: &str, topic: Option<String>) {
        let key = self.case.fold(channel);
        let Some(room) = self.channels.get_mut(&key) else {
            return;
        };
        room.topic = topic;
        room.topic_known = true;
        room.name = channel.to_string();
    }

    pub fn topic(&self, channel: &str) -> Option<Option<&str>> {
        let room = self.channels.get(&self.case.fold(channel))?;
        if room.topic_known {
            Some(room.topic.as_deref())
        } else {
            None
        }
    }

    pub fn begin_names(&mut self, channel: &str) {
        let key = self.case.fold(channel);
        let room = self.channels.entry(key.clone()).or_insert_with(|| Channel {
            name: channel.to_string(),
            topic: None,
            topic_known: false,
            members: BTreeMap::new(),
            names_open: false,
            modes: BTreeMap::new(),
        });
        if !room.names_open {
            room.members.clear();
            room.names_open = true;
        }
        if !self.order.iter().any(|item| item == &key) {
            self.order.push(key);
        }
    }

    pub fn add_names(&mut self, channel: &str, raw_names: &str) {
        self.begin_names(channel);
        let key = self.case.fold(channel);
        for raw in raw_names.split_whitespace() {
            let (prefix, nick) = split_prefixed_nick(&self.prefixes, raw);
            // userhost-in-names sends `nick!user@host`.
            let nick = match nick.split_once('!') {
                Some((bare, _)) => bare.to_string(),
                None => nick,
            };
            if nick.is_empty() {
                continue;
            }
            self.insert_member(&key, &nick, &prefix);
        }
    }

    pub fn end_names(&mut self, channel: &str) -> Vec<String> {
        let key = self.case.fold(channel);
        let Some(room) = self.channels.get_mut(&key) else {
            return Vec::new();
        };
        room.names_open = false;
        room.name = channel.to_string();
        self.display_names(&key)
    }

    pub fn names(&self, channel: &str) -> Option<Vec<String>> {
        let key = self.case.fold(channel);
        self.channels.get(&key).map(|_| self.display_names(&key))
    }

    pub fn channels(&self) -> Vec<&str> {
        self.order
            .iter()
            .filter_map(|key| self.channels.get(key).map(|room| room.name.as_str()))
            .collect()
    }

    pub fn fallback(&self) -> Option<&str> {
        self.order
            .last()
            .and_then(|key| self.channels.get(key).map(|room| room.name.as_str()))
    }

    /// Updates nick prefixes for channel mode changes and returns the nicks touched.
    pub fn apply_mode(&mut self, params: &[String]) -> Vec<(String, String)> {
        let Some(channel) = params.first() else {
            return Vec::new();
        };
        if !self.is_channel(channel) {
            return Vec::new();
        }
        let Some(modes) = params.get(1) else {
            return Vec::new();
        };
        let key = self.case.fold(channel);
        if !self.channels.contains_key(&key) {
            return Vec::new();
        }
        let mut adding = true;
        let mut args = params.iter().skip(2);
        let mut changed = Vec::new();
        for mode in modes.chars() {
            match mode {
                '+' => adding = true,
                '-' => adding = false,
                letter => {
                    let takes_param = self.mode_takes_param(letter, adding);
                    let prefix = self.prefixes.iter().find(|(flag, _)| *flag == letter);
                    if let Some((_, symbol)) = prefix {
                        let Some(nick) = args.next() else {
                            break;
                        };
                        if let Some(member) = self
                            .channels
                            .get_mut(&key)
                            .and_then(|room| room.members.get_mut(&self.case.fold(nick)))
                        {
                            if adding {
                                if !member.prefix.contains(*symbol) {
                                    member.prefix.push(*symbol);
                                    member.prefix = order_prefix(&self.prefixes, &member.prefix);
                                }
                            } else {
                                member.prefix.retain(|ch| ch != *symbol);
                            }
                            changed.push((member.nick.clone(), member.prefix.clone()));
                        }
                    } else if takes_param {
                        let Some(arg) = args.next() else {
                            break;
                        };
                        if !self.list_modes.contains(letter)
                            && let Some(room) = self.channels.get_mut(&key)
                        {
                            if adding {
                                room.modes.insert(letter, Some(arg.clone()));
                            } else {
                                room.modes.remove(&letter);
                            }
                        }
                    } else if self.never_param.contains(letter) || self.set_param.contains(letter) {
                        if let Some(room) = self.channels.get_mut(&key) {
                            if adding {
                                room.modes.insert(letter, None);
                            } else {
                                room.modes.remove(&letter);
                            }
                        }
                    } else if !self.never_param.contains(letter)
                        && !self.set_param.contains(letter)
                        && !self.always_param.contains(letter)
                        && !self.list_modes.contains(letter)
                    {
                        break;
                    }
                }
            }
        }
        changed
    }

    /// The individual changes in a MODE line: (adding, letter, argument).
    pub fn mode_changes(&self, params: &[String]) -> Vec<(bool, char, Option<String>)> {
        let Some(modes) = params.get(1) else {
            return Vec::new();
        };
        let mut adding = true;
        let mut args = params.iter().skip(2);
        let mut out = Vec::new();
        for letter in modes.chars() {
            match letter {
                '+' => adding = true,
                '-' => adding = false,
                letter => {
                    let arg = if self.mode_takes_param(letter, adding) {
                        args.next().cloned()
                    } else {
                        None
                    };
                    out.push((adding, letter, arg));
                }
            }
        }
        out
    }

    /// Replace a channel's flags with a full mode reply (`324`): the mode
    /// string followed by its arguments.
    pub fn set_modes(&mut self, channel: &str, params: &[String]) {
        let key = self.case.fold(channel);
        if let Some(room) = self.channels.get_mut(&key) {
            room.modes.clear();
        }
        let mut full = vec![channel.to_string()];
        full.extend(params.iter().cloned());
        self.apply_mode(&full);
    }

    /// Channel flags as a mode string, such as `+ntk secret`.
    pub fn mode_string(&self, channel: &str) -> Option<String> {
        let room = self.channels.get(&self.case.fold(channel))?;
        if room.modes.is_empty() {
            return Some(String::new());
        }
        let letters: String = room.modes.keys().collect();
        let args: Vec<&str> = room.modes.values().flatten().map(String::as_str).collect();
        let mut out = format!("+{letters}");
        for arg in args {
            out.push(' ');
            out.push_str(arg);
        }
        Some(out)
    }

    /// Whether a mode letter takes an argument as the server described its
    /// modes, or `None` for a letter it never named.
    pub fn mode_param(&self, letter: char, adding: bool) -> Option<bool> {
        if self.mode_takes_param(letter, adding) {
            Some(true)
        } else if self.never_param.contains(letter) || self.set_param.contains(letter) {
            Some(false)
        } else {
            None
        }
    }

    fn mode_takes_param(&self, letter: char, adding: bool) -> bool {
        self.list_modes.contains(letter)
            || self.always_param.contains(letter)
            || (adding && self.set_param.contains(letter))
            || self.prefixes.iter().any(|(flag, _)| *flag == letter)
    }

    fn insert_member(&mut self, channel_key: &str, nick: &str, prefix: &str) {
        let Some(room) = self.channels.get_mut(channel_key) else {
            return;
        };
        let folded = self.case.fold(nick);
        room.members.insert(
            folded,
            Member {
                nick: nick.to_string(),
                prefix: order_prefix(&self.prefixes, prefix),
            },
        );
    }

    fn display_names(&self, channel_key: &str) -> Vec<String> {
        let Some(room) = self.channels.get(channel_key) else {
            return Vec::new();
        };
        let mut members: Vec<&Member> = room.members.values().collect();
        members.sort_by(|left, right| {
            prefix_rank(&self.prefixes, &left.prefix)
                .cmp(&prefix_rank(&self.prefixes, &right.prefix))
                .then_with(|| self.case.fold(&left.nick).cmp(&self.case.fold(&right.nick)))
        });
        members
            .into_iter()
            .map(|member| format!("{}{}", member.prefix, member.nick))
            .collect()
    }
}

impl Default for Directory {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_prefix_map(value: &str) -> Option<Vec<(char, char)>> {
    let rest = value.strip_prefix('(')?;
    let (modes, prefixes) = rest.split_once(')')?;
    let pairs: Vec<(char, char)> = modes.chars().zip(prefixes.chars()).collect();
    if pairs.is_empty() { None } else { Some(pairs) }
}

fn split_prefixed_nick(prefixes: &[(char, char)], raw: &str) -> (String, String) {
    let mut prefix = String::new();
    let mut rest = raw;
    while let Some(ch) = rest.chars().next() {
        if prefixes.iter().any(|(_, symbol)| *symbol == ch) {
            prefix.push(ch);
            rest = &rest[ch.len_utf8()..];
        } else {
            break;
        }
    }
    (prefix, rest.to_string())
}

fn order_prefix(prefixes: &[(char, char)], raw: &str) -> String {
    prefixes
        .iter()
        .filter(|(_, symbol)| raw.contains(*symbol))
        .map(|(_, symbol)| *symbol)
        .collect()
}

fn prefix_rank(prefixes: &[(char, char)], prefix: &str) -> usize {
    prefixes
        .iter()
        .position(|(_, symbol)| prefix.contains(*symbol))
        .unwrap_or(prefixes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc1459_folds_brackets_and_channel_case() {
        let dir = Directory::new();
        assert!(dir.same("Bob[a]", "bob{a}"));
        assert!(dir.same("a~b\\c", "A^B|C"));
        let mut strict = Directory::new();
        strict.apply_support(&["CASEMAPPING=strict-rfc1459".into()]);
        assert!(strict.same("a[b]\\", "A{B}|"));
        assert!(!strict.same("a~", "a^"));
        let mut ascii = Directory::new();
        ascii.apply_support(&["CASEMAPPING=ascii".into()]);
        assert!(ascii.same("ABC", "abc"));
        assert!(!ascii.same("a[", "a{"));
        assert!(dir.same("#OpenIRC", "#openirc"));
        assert!(!dir.same("bob", "bobby"));
    }

    #[test]
    fn names_topic_and_operator_mode_share_one_channel() {
        let mut dir = Directory::new();
        dir.apply_support(&["CASEMAPPING=rfc1459".into(), "PREFIX=(ov)@+".into()]);
        dir.joined("#OpenIRC", "bob");
        dir.set_topic("#openirc", Some("hello".into()));
        assert_eq!(dir.topic("#OPENIRC"), Some(Some("hello")));

        dir.add_names("#openirc", "@Alice +bob carol");
        dir.add_names("#openirc", "dave");
        let names = dir.end_names("#openirc");
        assert_eq!(names, vec!["@Alice", "+bob", "carol", "dave"]);

        dir.apply_mode(&["#openirc".into(), "+o".into(), "carol".into()]);
        let names = dir.names("#openirc").unwrap();
        assert!(names.iter().any(|name| name == "@carol"));
        assert!(names.iter().any(|name| name == "@Alice"));
    }

    #[test]
    fn quit_and_leave_update_membership() {
        let mut dir = Directory::new();
        dir.joined("#a", "bob");
        dir.joined("#b", "bob");
        dir.joined("#a", "alice");
        dir.quit("alice");
        assert_eq!(dir.names("#a").unwrap(), vec!["bob"]);
        assert_eq!(dir.left("#b").as_deref(), Some("#b"));
        assert_eq!(dir.fallback(), Some("#a"));
    }
}
