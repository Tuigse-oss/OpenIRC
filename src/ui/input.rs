//! Editing help for the input line: command history and nickname
//! completion. Kept free of egui so it can be tested directly.

/// Lines sent from the input line, newest last. Up and Down walk through
/// them; the text being typed is kept and comes back after the newest line.
#[derive(Default)]
pub struct History {
    lines: Vec<String>,
    /// Each line's own number, which never changes or comes back, so work
    /// a line started (a timer, say) can still find its line later.
    ids: Vec<u64>,
    /// For each line, whether it was found to hold a password when what it
    /// started ran. Decided then, from what the line really did, so a later
    /// change of alias or prefix cannot make an old secret look harmless.
    secret: Vec<bool>,
    next_id: u64,
    position: Option<usize>,
    draft: String,
}

impl History {
    const KEEP: usize = 200;

    /// The lines kept, oldest first.
    #[cfg(test)]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The lines that may be saved: those not found to hold a password, and
    /// not still `waiting` for work they started.
    pub fn shareable<'a>(
        &'a self,
        waiting: &'a std::collections::HashSet<u64>,
    ) -> impl Iterator<Item = &'a String> {
        self.lines
            .iter()
            .zip(self.ids.iter().zip(&self.secret))
            .filter(|(_, (id, secret))| !**secret && !waiting.contains(id))
            .map(|(line, _)| line)
    }

    /// The line numbered `id` held a password. True when that is news.
    pub fn mark_secret(&mut self, id: u64) -> bool {
        match self.ids.iter().position(|known| *known == id) {
            Some(at) if !self.secret[at] => {
                self.secret[at] = true;
                true
            }
            _ => false,
        }
    }

    /// Start from lines saved by an earlier session.
    pub fn restore(&mut self, lines: &[String]) {
        self.lines.clear();
        self.ids.clear();
        self.secret.clear();
        for line in lines.iter().rev().take(Self::KEEP).rev() {
            self.lines.push(line.clone());
            self.ids.push(self.next_id);
            self.secret.push(false);
            self.next_id += 1;
        }
        self.position = None;
    }

    pub fn clear(&mut self) {
        let next_id = self.next_id;
        *self = Self::default();
        self.next_id = next_id;
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Add a typed line and give its number; the same line twice in a row
    /// is kept once, under the first one's number.
    pub fn push(&mut self, line: &str) -> Option<u64> {
        let line = line.trim_end();
        self.position = None;
        if line.is_empty() {
            return None;
        }
        if self.lines.last().is_some_and(|last| last == line) {
            return self.ids.last().copied();
        }
        let id = self.next_id;
        self.next_id += 1;
        self.lines.push(line.to_string());
        self.ids.push(id);
        self.secret.push(false);
        if self.lines.len() > Self::KEEP {
            self.lines.remove(0);
            self.ids.remove(0);
            self.secret.remove(0);
        }
        Some(id)
    }

    /// The older line to show for Up, given what is in the input now.
    pub fn up(&mut self, current: &str) -> Option<String> {
        if self.lines.is_empty() {
            return None;
        }
        let next = match self.position {
            None => {
                self.draft = current.to_string();
                self.lines.len() - 1
            }
            Some(0) => 0,
            Some(at) => at - 1,
        };
        self.position = Some(next);
        Some(self.lines[next].clone())
    }

    /// The newer line for Down; past the newest line, the saved draft.
    pub fn down(&mut self) -> Option<String> {
        let at = self.position?;
        if at + 1 < self.lines.len() {
            self.position = Some(at + 1);
            Some(self.lines[at + 1].clone())
        } else {
            self.position = None;
            Some(std::mem::take(&mut self.draft))
        }
    }
}

/// Whether a typed entry may hold a password, so it is never saved with the
/// input history. Every line of a pasted block counts, a command may start
/// with the configured `prefix` or any other prefix character (`/`, `.`,
/// `!` …), and `/raw` or `/quote` are read as the IRC lines they send.
pub fn may_hold_secret(entry: &str, prefix: &str) -> bool {
    entry.lines().any(|line| line_holds_secret(line, prefix, 0))
}

/// Whether a line sent to the server is words for its target: plain text,
/// `/say` or `/me`.
pub fn is_speech(line: &str) -> bool {
    let Some(command) = line.trim().strip_prefix('/') else {
        return true;
    };
    let name = command.split(char::is_whitespace).next().unwrap_or("");
    matches!(name.to_ascii_lowercase().as_str(), "say" | "me")
}

/// `depth` counts the `/raw` or `/quote` this line is inside; past eight,
/// the line is taken to hold a password rather than read any further.
fn line_holds_secret(line: &str, prefix: &str, depth: usize) -> bool {
    if depth > 8 {
        return true;
    }
    // `/via <target> <line>` runs the line inside: read that one, the way
    // the server side reads it, whatever prefix or spelling it has.
    let mut line = line.trim();
    for _ in 0..8 {
        let command = line
            .strip_prefix(prefix)
            .filter(|_| !prefix.is_empty())
            .or_else(|| line.strip_prefix(|ch: char| ch.is_ascii_punctuation()));
        let Some(rest) = command.and_then(crate::pump::via_args) else {
            break;
        };
        match crate::pump::unwrap_via(rest) {
            // Anything sent to a service may be a password, as with `/msg`.
            Ok(via) if crate::connection::is_service(via.target) => return true,
            Ok(via) => line = via.line.trim(),
            // Not run as it is, but whatever it holds stays unsaved.
            Err(_) => return true,
        }
    }
    let lower = line.to_ascii_lowercase();
    if ["identify", "register", "ghost", "password", "passwd"]
        .iter()
        .any(|word| lower.contains(word))
    {
        return true;
    }
    let prefix = prefix.trim().to_ascii_lowercase();
    let body = match lower.strip_prefix(prefix.as_str()) {
        Some(rest) if !prefix.is_empty() => rest,
        _ => lower.trim_start_matches(|ch: char| ch.is_ascii_punctuation() && ch != '#'),
    };
    let (command, rest) = body
        .split_once(char::is_whitespace)
        .map(|(command, rest)| (command, rest.trim()))
        .unwrap_or((body, ""));
    let args: Vec<&str> = rest.split_whitespace().collect();
    let service = crate::connection::is_service;
    // A login or a password change, whoever it is sent to: bots such as
    // `Q` are not named like services. Read in plain text, and in the words
    // a command sends.
    let words = match command {
        "say" | "me" => Some(rest),
        "msg" | "privmsg" | "notice" | "query" | "describe" | "ctcp" | "ctcpreply" => {
            rest.split_once(char::is_whitespace).map(|(_, words)| words)
        }
        _ => None,
    };
    if [Some(body), words].into_iter().flatten().any(|words| {
        crate::connection::opens_with_login(words.trim_start().trim_start_matches(':'))
    }) {
        return true;
    }
    match command {
        "pass" | "oper" | "auth" | "login" | "authenticate" | "key" => true,
        name if service(name) => true,
        "raw" | "quote" => line_holds_secret(rest, "", depth + 1),
        "msg" | "privmsg" | "notice" | "query" | "describe" | "ctcp" | "ctcpreply" => {
            args.first().is_some_and(|target| service(target))
        }
        // A run may go on to say anything to the service it picks.
        "target" | "win" | "window" => args.iter().any(|target| service(target)),
        // The same reading of the line as /server itself, so a password
        // after `host:port` counts too. A line it cannot read may still
        // hold one.
        "server" | "sserver" => {
            super::commands::parse_server(rest).map_or(true, |args| args.password.is_some())
        }
        // `/join #channel key`
        "join" | "j" => args.len() >= 2,
        // `/mode #channel +k key`, `-k key`, `-l+k key` …
        "mode" => args.get(1).is_some_and(|modes| modes.contains('k')),
        _ => false,
    }
}

/// Commands Tab completes after a `/` at the start of the line.
pub const COMMANDS: &[&str] = &[
    "admin",
    "alias",
    "ame",
    "amsg",
    "anick",
    "auser",
    "away",
    "back",
    "ban",
    "banlist",
    "beep",
    "bs",
    "channel",
    "clear",
    "clearall",
    "close",
    "closemsg",
    "cs",
    "ctcp",
    "ctcpreply",
    "dcc",
    "debug",
    "dehalfop",
    "deop",
    "describe",
    "devoice",
    "dialog",
    "disconnect",
    "dns",
    "echo",
    "editbox",
    "except",
    "exit",
    "flash",
    "flood",
    "fullname",
    "halfop",
    "help",
    "hop",
    "hs",
    "identify",
    "ignore",
    "info",
    "invite",
    "ison",
    "join",
    "key",
    "kick",
    "knock",
    "limit",
    "links",
    "list",
    "load",
    "log",
    "lusers",
    "map",
    "me",
    "mode",
    "motd",
    "ms",
    "msg",
    "names",
    "nick",
    "nickserv",
    "notice",
    "notify",
    "omsg",
    "onotice",
    "op",
    "oper",
    "os",
    "owner",
    "part",
    "partall",
    "ping",
    "play",
    "query",
    "quit",
    "quote",
    "raw",
    "run",
    "ruser",
    "savebuf",
    "say",
    "scon",
    "server",
    "set",
    "silence",
    "stats",
    "target",
    "time",
    "timer",
    "timers",
    "tip",
    "topic",
    "ulist",
    "umode",
    "unban",
    "unignore",
    "unload",
    "unset",
    "url",
    "userhost",
    "version",
    "via",
    "voice",
    "who",
    "whois",
    "whowas",
    "win",
];

/// An in-progress Tab completion, so repeated Tab presses cycle through the
/// matches for the same typed prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    /// Text before the word being completed.
    head: String,
    /// Text after the cursor when completion started.
    tail: String,
    /// Every candidate for the typed prefix, in nick list order.
    matches: Vec<String>,
    /// Which candidate is in the input now.
    index: usize,
    /// The input text this completion produced; a different text means
    /// the user typed, and a new completion starts.
    produced: String,
}

impl Completion {
    /// The input text and cursor for the current match.
    fn render(&mut self, first_suffix: &str) -> (String, usize) {
        // A nick at the start of the line gets the suffix (such as ": ");
        // channels, commands, and later words get a space.
        let word = &self.matches[self.index];
        let mut suffix = if self.head.is_empty() && !word.starts_with(['#', '/']) {
            first_suffix
        } else {
            " "
        };
        if self.tail.starts_with(char::is_whitespace) {
            suffix = suffix.trim_end();
        }
        let mut text = format!("{}{}{suffix}", self.head, self.matches[self.index]);
        let cursor = text.chars().count();
        text.push_str(&self.tail);
        self.produced = text.clone();
        (text, cursor)
    }
}

/// Complete the word before `cursor` (a character index) from `names`.
/// The first word gets `first_suffix` (such as ": "), later words a space.
/// Pressing Tab again on the produced text moves to the next match.
/// Returns the new text and the new cursor position.
pub fn complete(
    text: &str,
    cursor: usize,
    names: &[String],
    first_suffix: &str,
    state: &mut Option<Completion>,
) -> Option<(String, usize)> {
    if let Some(running) = state.as_mut()
        && running.produced == text
    {
        running.index = (running.index + 1) % running.matches.len();
        return Some(running.render(first_suffix));
    }
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    let start = chars[..cursor]
        .iter()
        .rposition(|ch| ch.is_whitespace())
        .map_or(0, |at| at + 1);
    let prefix = chars[start..cursor]
        .iter()
        .collect::<String>()
        .to_lowercase();
    if prefix.is_empty() {
        *state = None;
        return None;
    }
    let mut matches: Vec<String> = names
        .iter()
        .map(|name| {
            name.trim_start_matches(['@', '+', '%', '&', '~'])
                .to_string()
        })
        .filter(|name| name.to_lowercase().starts_with(&prefix))
        .collect();
    matches.dedup();
    if matches.is_empty() {
        *state = None;
        return None;
    }
    let mut completion = Completion {
        head: chars[..start].iter().collect(),
        tail: chars[cursor..].iter().collect(),
        matches,
        index: 0,
        produced: String::new(),
    };
    let result = completion.render(first_suffix);
    *state = Some(completion);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_not_kept_and_history_restores() {
        let may_hold_secret = |line: &str| may_hold_secret(line, "/");
        assert!(may_hold_secret("/msg NickServ IDENTIFY hunter2"));
        assert!(may_hold_secret("/ns identify hunter2"));
        assert!(may_hold_secret("/oper admin secret"));
        assert!(may_hold_secret("/raw PASS x"));
        assert!(!may_hold_secret("/join #rust"));
        for line in [
            "/raw OPER admin swordfish",
            "/quote OPER admin swordfish",
            "/server -p swordfish irc.example",
            "/server irc.example 6667 swordfish",
            "/join #room swordfish",
            "/mode #room +k swordfish",
            ".oper admin swordfish",
            "/echo ok\n/oper admin swordfish",
            "/msg ChanServ op #c",
            "/key swordfish",
            "/mode #room -l+k swordfish",
            "/mode #room -k swordfish",
            "/server irc.example:6697 swordfish",
            "/server -m irc.example:+6697 swordfish",
            "/cs set #room key swordfish",
        ] {
            assert!(may_hold_secret(line), "{line}");
        }
        for line in [
            "/server irc.example 6697",
            "/mode #room +o bob",
            "/msg bob hello",
            "#channel is great",
            "/server irc.example:6697",
            "/server -m -j #a irc.example",
        ] {
            assert!(!may_hold_secret(line), "{line}");
        }
        // A line run through /via is read for what it runs.
        assert!(may_hold_secret("/via #ops /join #locked swordfish"));
        assert!(may_hold_secret("/via #a /via #b /oper admin swordfish"));
        assert!(!may_hold_secret("/via #ops hello there"));
        // However deep `/raw` goes, the check ends, and a line too deep to
        // read is not saved.
        assert!(may_hold_secret("/raw /raw PASS x"));
        assert!(!may_hold_secret("/raw /raw PRIVMSG #ops :hi"));
        assert!(may_hold_secret(&format!(
            "{}PRIVMSG #ops :hi",
            "/raw ".repeat(4_096)
        )));
        // A configured `#` prefix makes `#oper` a command.
        assert!(super::may_hold_secret("#oper admin swordfish", "#"));
        assert!(!super::may_hold_secret("#oper admin swordfish", "/"));
        assert!(!may_hold_secret("hello everyone"));
        let mut history = History::default();
        history.restore(&["one".to_string(), "two".to_string()]);
        assert_eq!(history.up(""), Some("two".to_string()));
        assert_eq!(history.lines().len(), 2);
    }

    #[test]
    fn a_login_counts_whatever_it_is_sent_to() {
        let may_hold_secret = |line: &str| may_hold_secret(line, "/");
        for line in [
            "/msg Q AUTH me swordfish",
            "/msg X@channels.undernet.org LOGIN me swordfish",
            "/notice Q auth me swordfish",
            "/query Q AUTH me swordfish",
            "/describe Q AUTH me swordfish",
            "/ctcp Q AUTH me swordfish",
            "/ctcpreply Q AUTH me swordfish",
            "/raw PRIVMSG Q :AUTH me swordfish",
            "/quote NOTICE X :LOGIN me swordfish",
            "/raw PRIVMSG Q :\u{1}AUTH me swordfish\u{1}",
            "/say ID swordfish",
            "/me ID swordfish",
            "id swordfish",
            "RECOVER carol swordfish",
            "/msg Q REGAIN carol swordfish",
            "/msg X NEWPASS swordfish",
            "/msg Q set pass swordfish",
            "/via Q AUTH me swordfish",
            "/via #ops /msg Q AUTH me swordfish",
            // A service named with its server is still a service.
            "/msg NickServ@services.example LIST",
            "/via NickServ@services.example hello",
        ] {
            assert!(may_hold_secret(line), "{line}");
        }
        for line in [
            "/msg bob set the table",
            "/msg Q hello",
            "/msg Q HELP AUTH",
            "set the table",
            "/msg bob@example hello",
        ] {
            assert!(!may_hold_secret(line), "{line}");
        }
    }

    #[test]
    fn channels_and_commands_complete_with_a_space() {
        let names: Vec<String> = vec!["alice".into(), "#openirc".into(), "/join".into()];
        let mut state = None;
        assert_eq!(
            complete("#op", 3, &names, ": ", &mut state),
            Some(("#openirc ".to_string(), 9))
        );
        state = None;
        assert_eq!(
            complete("/jo", 3, &names, ": ", &mut state),
            Some(("/join ".to_string(), 6))
        );
        state = None;
        assert_eq!(
            complete("al", 2, &names, ": ", &mut state),
            Some(("alice: ".to_string(), 7))
        );
        let mut sorted = COMMANDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, COMMANDS, "keep the list sorted");
    }

    #[test]
    fn history_walks_back_and_restores_the_draft() {
        let mut history = History::default();
        history.push("first");
        history.push("second");
        history.push("second");
        assert_eq!(history.up("typing").as_deref(), Some("second"));
        assert_eq!(history.up("second").as_deref(), Some("first"));
        assert_eq!(history.up("first").as_deref(), Some("first"));
        assert_eq!(history.down().as_deref(), Some("second"));
        assert_eq!(history.down().as_deref(), Some("typing"));
        assert_eq!(history.down(), None);
    }

    #[test]
    fn completion_adds_a_suffix_and_cycles() {
        let names = vec!["@alice".to_string(), "+alex".to_string(), "bob".to_string()];
        let mut state = None;
        let (text, cursor) = complete("al", 2, &names, ": ", &mut state).unwrap();
        assert_eq!(text, "alice: ");
        assert_eq!(cursor, 7);
        let (text, _) = complete(&text, cursor, &names, ": ", &mut state).unwrap();
        assert_eq!(text, "alex: ");
        let (text, _) = complete(&text, 6, &names, ": ", &mut state).unwrap();
        assert_eq!(text, "alice: ");
    }

    #[test]
    fn completion_in_the_middle_of_a_line_uses_a_space() {
        let names = vec!["bob".to_string()];
        let mut state = None;
        let (text, cursor) = complete("hi b there", 4, &names, ": ", &mut state).unwrap();
        assert_eq!(text, "hi bob there");
        assert_eq!(cursor, 6);
        assert!(complete("hi zz", 5, &names, ": ", &mut None).is_none());
    }
}
