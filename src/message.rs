//! IRC wire messages, including the IRCv3 tag prefix.
//!
//! The grammar follows RFC 1459 / RFC 2812: an optional prefix, a command,
//! and up to 15 parameters. The last parameter may contain spaces when it is
//! introduced with `:`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub tags: Vec<(String, String)>,
    pub prefix: Option<Prefix>,
    pub command: String,
    pub params: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prefix {
    Server(String),
    User {
        nick: String,
        user: Option<String>,
        host: Option<String>,
    },
}

impl Prefix {
    pub fn name(&self) -> &str {
        match self {
            Prefix::Server(name) => name,
            Prefix::User { nick, .. } => nick,
        }
    }
}

impl std::fmt::Display for Prefix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Prefix::Server(name) => f.write_str(name),
            Prefix::User { nick, user, host } => {
                f.write_str(nick)?;
                if let Some(user) = user {
                    write!(f, "!{user}")?;
                }
                if let Some(host) = host {
                    write!(f, "@{host}")?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub detail: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}

impl std::error::Error for ParseError {}

impl Message {
    pub fn parse(raw: &str) -> Result<Self, ParseError> {
        let line = raw.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            return Err(ParseError {
                detail: "empty IRC line".into(),
            });
        }

        let mut rest = line;
        let mut tags = Vec::new();
        if let Some(stripped) = rest.strip_prefix('@') {
            let Some((tag_src, after)) = stripped.split_once(' ') else {
                return Err(ParseError {
                    detail: "IRC tags are missing the command".into(),
                });
            };
            tags = parse_tags(tag_src)?;
            rest = after;
        }

        let mut prefix = None;
        if let Some(stripped) = rest.strip_prefix(':') {
            let Some((pfx, after)) = stripped.split_once(' ') else {
                return Err(ParseError {
                    detail: "IRC prefix is missing the command".into(),
                });
            };
            prefix = Some(parse_prefix(pfx));
            rest = after;
        }

        if rest.is_empty() {
            return Err(ParseError {
                detail: "IRC line is missing a command".into(),
            });
        }

        let (command, params_src) = match rest.split_once(' ') {
            Some((command, params)) => (command, params),
            None => (rest, ""),
        };
        if command.is_empty() || command.contains(' ') {
            return Err(ParseError {
                detail: "IRC command is empty".into(),
            });
        }

        Ok(Message {
            tags,
            prefix,
            command: command.to_ascii_uppercase(),
            params: parse_params(params_src),
        })
    }

    pub fn command(command: &str, params: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Message {
            tags: Vec::new(),
            prefix: None,
            command: command.to_ascii_uppercase(),
            params: params.into_iter().map(Into::into).collect(),
        }
    }

    /// Encode one message without the trailing CRLF.
    pub fn encode(&self) -> String {
        let mut out = String::new();
        if let Some(prefix) = &self.prefix {
            out.push(':');
            out.push_str(&prefix.to_string());
            out.push(' ');
        }
        out.push_str(&self.command);
        for (index, param) in self.params.iter().enumerate() {
            out.push(' ');
            if index + 1 == self.params.len() {
                out.push(':');
            }
            out.push_str(param);
        }
        out
    }

    pub fn trailing(&self) -> Option<&str> {
        self.params.last().map(String::as_str)
    }
}

/// Turn one raw line from the wire into text. IRC does not fix an encoding:
/// valid UTF-8 is kept as is, anything else is read as Windows-1252 (a
/// superset of Latin-1) so one odd byte never ends the session.
pub fn decode_line(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => encoding_rs::WINDOWS_1252
            .decode_without_bom_handling(bytes)
            .0
            .into_owned(),
    }
}

/// Bytes read from a socket, handed back one complete line at a time. Taking
/// a line only moves a start offset; the consumed front is dropped when it
/// is more than half the buffer, so a burst of lines costs linear time.
#[derive(Default)]
pub struct LineBuffer {
    bytes: Vec<u8>,
    start: usize,
}

impl LineBuffer {
    /// Longest line kept while waiting for its newline. IRC lines are 512
    /// bytes plus up to 8 KiB of tags; anything longer is cut at this size.
    pub const MAX_LINE: usize = 16 * 1024;

    pub fn extend(&mut self, data: &[u8]) {
        if self.start > 0 && self.start * 2 >= self.bytes.len() {
            self.bytes.drain(..self.start);
            self.start = 0;
        }
        self.bytes.extend_from_slice(data);
    }

    /// The next complete line without its CR/LF ending. A run longer than
    /// `MAX_LINE` without a newline is returned as one line.
    pub fn next_line(&mut self) -> Option<&[u8]> {
        let pending = &self.bytes[self.start..];
        let (end, next) = match pending.iter().position(|&byte| byte == b'\n') {
            Some(newline) => (newline, newline + 1),
            None if pending.len() >= Self::MAX_LINE => (Self::MAX_LINE, Self::MAX_LINE),
            None => return None,
        };
        let line_start = self.start;
        self.start += next;
        let mut line = &self.bytes[line_start..line_start + end];
        while let [rest @ .., b'\r' | b'\n'] = line {
            line = rest;
        }
        Some(line)
    }

    #[cfg(test)]
    fn pending(&self) -> usize {
        self.bytes.len() - self.start
    }
}

/// PRIVMSG lines, each at most 512 bytes including CRLF.
pub fn privmsg_lines(target: &str, text: &str) -> Vec<String> {
    chunk_command("PRIVMSG", target, text)
}

pub fn notice_lines(target: &str, text: &str) -> Vec<String> {
    chunk_command("NOTICE", target, text)
}

fn chunk_command(command: &str, target: &str, text: &str) -> Vec<String> {
    let overhead = command.len() + 1 + target.len() + 2 + 2;
    let budget = 512usize.saturating_sub(overhead).max(1);
    if text.is_empty() {
        return vec![Message::command(command, [target, ""]).encode()];
    }
    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let take = take_bytes(rest, budget);
        let (chunk, next) = rest.split_at(take);
        lines.push(Message::command(command, [target, chunk]).encode());
        rest = next;
    }
    lines
}

fn take_bytes(text: &str, budget: usize) -> usize {
    if text.len() <= budget {
        return text.len();
    }
    let mut end = budget;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == 0 {
        text.chars().next().map(|ch| ch.len_utf8()).unwrap_or(1)
    } else {
        end
    }
}

fn parse_prefix(raw: &str) -> Prefix {
    if let Some((nick, rest)) = raw.split_once('!') {
        if let Some((user, host)) = rest.split_once('@') {
            Prefix::User {
                nick: nick.to_string(),
                user: Some(user.to_string()),
                host: Some(host.to_string()),
            }
        } else {
            Prefix::User {
                nick: nick.to_string(),
                user: Some(rest.to_string()),
                host: None,
            }
        }
    } else if let Some((nick, host)) = raw.split_once('@') {
        Prefix::User {
            nick: nick.to_string(),
            user: None,
            host: Some(host.to_string()),
        }
    } else {
        Prefix::Server(raw.to_string())
    }
}

fn parse_tags(source: &str) -> Result<Vec<(String, String)>, ParseError> {
    let mut tags = Vec::new();
    if source.is_empty() {
        return Ok(tags);
    }
    for part in source.split(';') {
        if part.is_empty() {
            continue;
        }
        let (key, value) = match part.split_once('=') {
            Some((key, value)) => (key, unescape_tag(value)),
            None => (part, String::new()),
        };
        if key.is_empty() {
            return Err(ParseError {
                detail: "empty IRC tag name".into(),
            });
        }
        tags.push((key.to_string(), value));
    }
    Ok(tags)
}

fn unescape_tag(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some(':') => out.push(';'),
                Some('s') => out.push(' '),
                Some('\\') => out.push('\\'),
                Some('r') => out.push('\r'),
                Some('n') => out.push('\n'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn parse_params(mut source: &str) -> Vec<String> {
    let mut params = Vec::new();
    loop {
        source = source.trim_start_matches(' ');
        if source.is_empty() {
            break;
        }
        if let Some(rest) = source.strip_prefix(':') {
            params.push(rest.to_string());
            break;
        }
        if params.len() == 14 {
            params.push(source.to_string());
            break;
        }
        match source.split_once(' ') {
            Some((middle, rest)) => {
                params.push(middle.to_string());
                source = rest;
            }
            None => {
                params.push(source.to_string());
                break;
            }
        }
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_buffer_splits_lines_and_caps_runaway_ones() {
        let mut buffer = LineBuffer::default();
        buffer.extend(b"one\r\ntw");
        assert_eq!(buffer.next_line(), Some(&b"one"[..]));
        assert_eq!(buffer.next_line(), None);
        buffer.extend(b"o\n");
        assert_eq!(buffer.next_line(), Some(&b"two"[..]));
        assert_eq!(buffer.pending(), 0);
        buffer.extend(&vec![b'x'; LineBuffer::MAX_LINE + 10]);
        assert_eq!(
            buffer.next_line().map(<[u8]>::len),
            Some(LineBuffer::MAX_LINE)
        );
        assert_eq!(buffer.pending(), 10);
    }

    #[test]
    fn decode_line_keeps_utf8_and_reads_other_bytes_as_latin1() {
        assert_eq!(decode_line("caf\u{e9}".as_bytes()), "caf\u{e9}");
        assert_eq!(decode_line(b"caf\xe9 au lait"), "caf\u{e9} au lait");
        assert_eq!(decode_line(b"\xff\xfe"), "\u{ff}\u{fe}");
        assert_eq!(decode_line(b"\x93quoted\x94"), "\u{201c}quoted\u{201d}");
    }

    #[test]
    fn parses_privmsg_with_user_prefix() {
        let message = Message::parse(":alice!a@host PRIVMSG #openirc :hello there\r\n").unwrap();
        assert_eq!(message.command, "PRIVMSG");
        assert_eq!(
            message.prefix,
            Some(Prefix::User {
                nick: "alice".into(),
                user: Some("a".into()),
                host: Some("host".into()),
            })
        );
        assert_eq!(message.params, vec!["#openirc", "hello there"]);
    }

    #[test]
    fn parses_tags_ping_and_numeric() {
        let tagged = Message::parse("@account=alice\\:bob :irc PING :token").unwrap();
        assert_eq!(tagged.tags, vec![("account".into(), "alice;bob".into())]);
        assert_eq!(tagged.command, "PING");
        assert_eq!(tagged.params, vec!["token"]);

        let welcome = Message::parse(":irc.example 001 bob :Welcome").unwrap();
        assert_eq!(welcome.command, "001");
        assert_eq!(welcome.params, vec!["bob", "Welcome"]);
    }

    #[test]
    fn unescapes_tag_spaces() {
        let message = Message::parse("@note=hello\\sworld PING :x").unwrap();
        assert_eq!(message.tags, vec![("note".into(), "hello world".into())]);
    }

    #[test]
    fn round_trips_spaced_trailing_parameter() {
        let message = Message::command("PRIVMSG", ["#openirc", "hello there"]);
        let encoded = message.encode();
        assert_eq!(encoded, "PRIVMSG #openirc :hello there");
        assert_eq!(Message::parse(&encoded).unwrap(), message);
    }

    #[test]
    fn splits_long_privmsg_on_char_boundaries() {
        let text = "é".repeat(400);
        let lines = privmsg_lines("#c", &text);
        assert!(lines.len() > 1);
        let mut joined = String::new();
        for line in &lines {
            let parsed = Message::parse(line).unwrap();
            assert!(line.len() + 2 <= 512);
            joined.push_str(&parsed.params[1]);
        }
        assert_eq!(joined, text);
    }

    #[test]
    fn rejects_an_empty_line() {
        assert!(Message::parse("\r\n").is_err());
    }
}
