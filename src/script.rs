//! Remote scripts, identifiers, variables, and timers.
//!
//! A remote is `on <event> <mask> <body>`. The body can use `$nick`, `$chan`,
//! `$me`, `$text`, `$time`, and `%name` variables. `/timer` is parsed here
//! and run by the window.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub event: String,
    pub mask: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fields<'a> {
    pub nick: &'a str,
    pub chan: &'a str,
    pub me: &'a str,
    pub text: &'a str,
    pub time: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimerSpec {
    pub seconds: u64,
    pub repeats: u32,
    pub command: String,
}

/// Case-insensitive wildcard match: `*` is any run of characters and `?` is
/// one character. An empty mask matches everything.
pub fn mask_match(mask: &str, text: &str) -> bool {
    let mask: Vec<char> = mask.trim().to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();
    if mask.is_empty() {
        return true;
    }
    let (mut m, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if m < mask.len() && (mask[m] == '?' || mask[m] == text[t]) {
            m += 1;
            t += 1;
        } else if m < mask.len() && mask[m] == '*' {
            star = Some((m, t));
            m += 1;
        } else if let Some((star_m, star_t)) = star {
            m = star_m + 1;
            t = star_t + 1;
            star = Some((star_m, star_t + 1));
        } else {
            return false;
        }
    }
    mask[m..].iter().all(|&ch| ch == '*')
}

/// Fill `$nick`, `$chan`, `$me`, `$text`, `$time`, and `%name` variables in
/// one left-to-right pass. Inserted values are never scanned again, so a
/// message that contains `%secret` or `$me` stays literal text.
pub fn expand(body: &str, fields: &Fields, vars: &[(String, String)]) -> String {
    let identifiers = [
        ("$nick", fields.nick),
        ("$chan", fields.chan),
        ("$me", fields.me),
        ("$text", fields.text),
        ("$time", fields.time),
    ];
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    'scan: while let Some(ch) = rest.chars().next() {
        if ch == '$' {
            for (token, value) in identifiers {
                if starts_with_word(rest, token) {
                    out.push_str(value);
                    rest = &rest[token.len()..];
                    continue 'scan;
                }
            }
        }
        if ch == '%' {
            let name_len = rest[1..]
                .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                .unwrap_or(rest.len() - 1);
            let name = &rest[1..1 + name_len];
            if let Some((_, value)) = vars
                .iter()
                .find(|(item, _)| item.trim_start_matches('%') == name)
            {
                out.push_str(value);
                rest = &rest[1 + name_len..];
                continue;
            }
        }
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

/// `text` starts with `token` (any case) and the next character does not
/// continue a word.
fn starts_with_word(text: &str, token: &str) -> bool {
    text.len() >= token.len()
        && text.is_char_boundary(token.len())
        && text[..token.len()].eq_ignore_ascii_case(token)
        && text[token.len()..]
            .chars()
            .next()
            .is_none_or(|ch| !ch.is_ascii_alphanumeric())
}

pub fn commands(
    remotes: &[Remote],
    vars: &[(String, String)],
    event: &str,
    fields: &Fields,
    separator: &str,
) -> Vec<String> {
    let haystack = if event.eq_ignore_ascii_case("join") {
        fields.chan
    } else {
        fields.text
    };
    remotes
        .iter()
        .filter(|remote| {
            remote.event.eq_ignore_ascii_case(event) && mask_match(&remote.mask, haystack)
        })
        .flat_map(|remote| {
            // Split the template before filling it in. `$text` and `$nick`
            // come from other users, so a `|` inside them must stay text and
            // never start a second command.
            remote
                .body
                .split(separator)
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(|line| expand(line, fields, vars))
                .collect::<Vec<_>>()
        })
        .collect()
}

pub fn parse_timer(line: &str) -> Option<TimerSpec> {
    let rest = line.trim().strip_prefix('/')?;
    let (name, rest) = rest.split_once(char::is_whitespace)?;
    if !name.eq_ignore_ascii_case("timer") {
        return None;
    }
    let (seconds, rest) = rest.trim().split_once(char::is_whitespace)?;
    let (repeats, command) = rest.trim().split_once(char::is_whitespace)?;
    let seconds = seconds.parse::<u64>().ok()?;
    let repeats = repeats.parse::<u32>().ok()?;
    if seconds == 0 || repeats == 0 || command.is_empty() {
        return None;
    }
    Some(TimerSpec {
        seconds,
        repeats,
        command: command.to_string(),
    })
}

pub fn parse_set(line: &str) -> Option<(String, String)> {
    let rest = line.trim().strip_prefix('/')?;
    let (name, rest) = rest.split_once(char::is_whitespace)?;
    if !name.eq_ignore_ascii_case("set") {
        return None;
    }
    let (var, value) = rest.trim().split_once(char::is_whitespace)?;
    let var = var.trim_start_matches('%').to_string();
    if var.is_empty() {
        return None;
    }
    Some((var, value.to_string()))
}

pub fn parse_unset(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix('/')?;
    let (name, var) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    if !name.eq_ignore_ascii_case("unset") || var.trim().is_empty() {
        return None;
    }
    Some(var.trim().trim_start_matches('%').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_remote_fills_identifiers() {
        let remotes = vec![Remote {
            event: "text".into(),
            mask: "*hi*".into(),
            body: "/notice $nick from $me in $chan: $text".into(),
        }];
        let fields = Fields {
            nick: "Ann",
            chan: "#openirc",
            me: "bob",
            text: "oh hi",
            time: "12:00",
        };
        assert_eq!(
            commands(&remotes, &[], "text", &fields, " | "),
            vec!["/notice Ann from bob in #openirc: oh hi"]
        );
        let quiet = Fields {
            text: "bye",
            ..fields
        };
        assert!(commands(&remotes, &[], "text", &quiet, " | ").is_empty());
    }

    #[test]
    fn variables_and_timer_and_set() {
        let fields = Fields {
            nick: "a",
            chan: "#c",
            me: "me",
            text: "",
            time: "12:01",
        };
        assert_eq!(
            expand(
                "say %greet at $time",
                &fields,
                &[("greet".into(), "hello".into())]
            ),
            "say hello at 12:01"
        );
        assert_eq!(
            parse_timer("/timer 5 3 /echo hi"),
            Some(TimerSpec {
                seconds: 5,
                repeats: 3,
                command: "/echo hi".into(),
            })
        );
        assert_eq!(
            parse_set("/set %greet hello"),
            Some(("greet".into(), "hello".into()))
        );
        assert_eq!(parse_unset("/unset %greet"), Some("greet".into()));
    }

    #[test]
    fn text_from_other_users_cannot_add_commands_or_read_variables() {
        let remotes = vec![Remote {
            event: "text".into(),
            mask: "*hi*".into(),
            body: "/notice $nick you said $text | /echo done".into(),
        }];
        let fields = Fields {
            nick: "carol",
            chan: "#c",
            me: "me",
            text: "hi | /raw PRIVMSG NickServ :DROP %secret $me",
            time: "12:00",
        };
        let vars = [("secret".to_string(), "hunter2".to_string())];
        assert_eq!(
            commands(&remotes, &vars, "text", &fields, " | "),
            vec![
                "/notice carol you said hi | /raw PRIVMSG NickServ :DROP %secret $me".to_string(),
                "/echo done".to_string(),
            ]
        );
    }

    #[test]
    fn masks_support_inner_wildcards() {
        assert!(mask_match("*", "anything"));
        assert!(mask_match("", "anything"));
        assert!(mask_match("*hi*", "oh HI there"));
        assert!(mask_match("hi*", "hi there"));
        assert!(!mask_match("hi*", "oh hi"));
        assert!(mask_match("a*b", "a long b"));
        assert!(!mask_match("a*b", "a long c"));
        assert!(mask_match("#op?nirc", "#openirc"));
        assert!(mask_match("exact", "EXACT"));
        assert!(!mask_match("exact", "exactly"));
    }
}
