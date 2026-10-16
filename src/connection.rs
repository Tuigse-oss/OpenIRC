use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::pump::{Config, Effect, Pump};
use crate::tls::{self, Socket};

pub struct Connection {
    socket: Socket,
    /// Bytes read from the socket that do not yet end in a newline. They
    /// survive an idle timeout, so a line split across packets stays whole.
    pending: crate::message::LineBuffer,
    pump: Pump,
    /// Own lines waiting for the flood limit. `None` writes at once.
    queue: Option<SendQueue>,
    /// Text encoding on the wire; `None` is UTF-8 with a Windows-1252
    /// fallback for incoming lines that are not UTF-8.
    encoding: Option<&'static encoding_rs::Encoding>,
    /// While set, raw lines are kept for the debug window.
    debug: Option<Arc<AtomicBool>>,
    /// Raw lines seen while debugging: true for lines from the server.
    wire: Vec<(bool, String)>,
}

/// Keep a raw line for the debug window, with secrets masked.
fn record(
    debug: &Option<Arc<AtomicBool>>,
    wire: &mut Vec<(bool, String)>,
    modes: &dyn ModeTable,
    from_server: bool,
    line: &str,
) {
    if debug
        .as_ref()
        .is_some_and(|flag| flag.load(Ordering::Relaxed))
    {
        wire.push((from_server, mask_secrets_with(line, modes)));
    }
}

/// Which mode letters take an argument, for finding a channel key among a
/// mode change's arguments.
pub trait ModeTable {
    /// Whether `letter` takes an argument when added or removed, or `None`
    /// when it is not known.
    fn mode_param(&self, letter: char, adding: bool) -> Option<bool>;
}

/// Without the server's own list of modes nothing about other letters is
/// certain, so every argument of a change that has a key is hidden.
struct Unknown;

impl ModeTable for crate::state::Directory {
    fn mode_param(&self, letter: char, adding: bool) -> Option<bool> {
        crate::state::Directory::mode_param(self, letter, adding)
    }
}

impl ModeTable for Unknown {
    fn mode_param(&self, _: char, _: bool) -> Option<bool> {
        None
    }
}

/// A raw line with passwords and channel keys replaced by `****`, for the
/// Debug window. Tags and the sender prefix are kept as they are.
pub fn mask_secrets(line: &str) -> String {
    mask_secrets_with(line, &Unknown)
}

/// `mask_secrets`, finding keys among mode arguments with `modes`: the
/// server's own description of its modes.
pub fn mask_secrets_with(line: &str, modes: &dyn ModeTable) -> String {
    let mut head = 0;
    for mark in ['@', ':'] {
        if line[head..].starts_with(mark) {
            match line[head..].find(' ') {
                Some(space) => head += space + 1,
                None => return line.to_string(),
            }
        }
    }
    format!("{}{}", &line[..head], mask_command(&line[head..], modes))
}

/// Whether `target` is a services bot by its name (`NickServ`, `ns`,
/// `NickServ@services.example` …), in any case.
pub fn is_service(target: &str) -> bool {
    let name = target
        .split('@')
        .next()
        .unwrap_or(target)
        .to_ascii_lowercase();
    name.ends_with("serv") || matches!(name.as_str(), "ns" | "cs" | "ms" | "hs" | "bs" | "os")
}

/// Whether a message opens with a login or a password change, which some
/// networks' bots take under names of their own (`AUTH`, `LOGIN`,
/// `SET PASSWORD` …), whatever it is sent to. The words are read inside
/// CTCP framing and after `ACTION`, as `mask_service_text` reads them.
pub fn opens_with_login(text: &str) -> bool {
    let (_, _, body, _) = service_words(text);
    login_verb(body)
}

/// The words a bot reads as its command: inside CTCP framing and after
/// `ACTION`. Gives the opening framing, the `ACTION `, the words, and the
/// closing framing.
fn service_words(text: &str) -> (&str, &str, &str, &str) {
    let (open, inner) = match text.strip_prefix('\u{1}') {
        Some(inner) => ("\u{1}", inner),
        None => ("", text),
    };
    let (inner, close) = match inner.strip_suffix('\u{1}') {
        Some(inner) if !open.is_empty() => (inner, "\u{1}"),
        _ => (inner, ""),
    };
    match inner.split_once(' ') {
        Some((word, body)) if word.eq_ignore_ascii_case("ACTION") => {
            (open, &inner[..word.len() + 1], body, close)
        }
        _ => (open, "", inner, close),
    }
}

/// Whether `body` opens with a word that logs in to a bot, or changes a
/// password, under any of the usual names.
fn login_verb(body: &str) -> bool {
    let mut words = body.split_whitespace();
    let verb = words.next().unwrap_or("").to_ascii_uppercase();
    match verb.as_str() {
        "AUTH" | "LOGIN" | "IDENTIFY" | "ID" | "REGISTER" | "GHOST" | "RECOVER" | "RELEASE"
        | "REGAIN" | "NEWPASS" => true,
        "SET" => words
            .next()
            .is_some_and(|word| matches!(word.to_ascii_uppercase().as_str(), "PASSWORD" | "PASS")),
        _ => false,
    }
}

/// The text of a message with its password hidden, such as `IDENTIFY ****`,
/// or `None` when it holds none. To a services bot, any of its commands
/// that take one count (`SET` with anything); to anyone else, a login or a
/// password change (`opens_with_login`), since bots such as `Q` or
/// `X@channels.undernet.org` are not named like services. A channel is no
/// bot, and everyone in it reads what is said there. The words are read
/// inside CTCP framing and after `ACTION`, which keep their places.
pub fn mask_service_text(target: &str, text: &str) -> Option<String> {
    let (open, action, body, close) = service_words(text);
    let verb = body.split_whitespace().next().unwrap_or("");
    let secret = if is_service(target) {
        matches!(
            verb.to_ascii_uppercase().as_str(),
            "IDENTIFY"
                | "ID"
                | "REGISTER"
                | "GHOST"
                | "RECOVER"
                | "RELEASE"
                | "REGAIN"
                | "SET"
                | "LOGIN"
                | "AUTH"
                | "NEWPASS"
        )
    } else {
        !target.starts_with(['#', '&', '+', '!', '@', '%', '~']) && login_verb(body)
    };
    secret.then(|| format!("{open}{action}{verb} ****{close}"))
}

/// Hide the argument of `k` in a mode change such as `+kl key 10`. Once a
/// letter comes up whose arguments are not known, the place of the key is
/// not certain either, so every argument from there on is hidden.
fn mask_mode_args(modes: &str, args: &[&str], table: &dyn ModeTable) -> Vec<String> {
    let mut out: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    if !modes.contains('k') {
        return out;
    }
    let mut adding = true;
    let mut next = 0;
    for letter in modes.chars() {
        match letter {
            '+' => adding = true,
            '-' => adding = false,
            'k' => {
                if let Some(arg) = out.get_mut(next) {
                    *arg = "****".into();
                }
                next += 1;
            }
            letter => match table.mode_param(letter, adding) {
                Some(true) => next += 1,
                Some(false) => {}
                None => {
                    for arg in out.iter_mut().skip(next) {
                        *arg = "****".into();
                    }
                    break;
                }
            },
        }
    }
    out
}

fn mask_command(line: &str, table: &dyn ModeTable) -> String {
    let words: Vec<&str> = line.splitn(3, ' ').collect();
    let command = words
        .first()
        .map(|word| word.to_ascii_uppercase())
        .unwrap_or_default();
    match command.as_str() {
        "PASS" => "PASS ****".into(),
        "OPER" if words.len() == 3 => format!("OPER {} ****", words[1]),
        "AUTHENTICATE"
            if words.get(1).is_some_and(|arg| {
                !matches!(
                    arg.to_ascii_uppercase().as_str(),
                    "+" | "*" | "PLAIN" | "EXTERNAL"
                )
            }) =>
        {
            "AUTHENTICATE ****".into()
        }
        // JOIN #a,#b key1,key2
        "JOIN" if words.len() == 3 => format!("JOIN {} ****", words[1]),
        "PRIVMSG" | "NOTICE" if words.len() == 3 => {
            match mask_service_text(words[1], words[2].trim_start_matches(':')) {
                Some(masked) => format!("{} {} :{masked}", words[0], words[1]),
                None => line.to_string(),
            }
        }
        // MODE #c +k key, and the server's 324 reply: me #c +k key
        "MODE" | "324" if words.len() == 3 => {
            let mut params: Vec<&str> = words[2].split(' ').collect();
            let skip = if command == "324" { 1 } else { 0 };
            if params.len() <= skip + 1 {
                return line.to_string();
            }
            let modes = params[skip].trim_start_matches(':');
            let masked = mask_mode_args(modes, &params[skip + 1..], table);
            params.truncate(skip + 1);
            let mut out = format!("{} {} {}", words[0], words[1], params.join(" "));
            for arg in masked {
                out.push(' ');
                out.push_str(&arg);
            }
            out
        }
        _ => line.to_string(),
    }
}

impl Connection {
    pub fn new(stream: Socket, config: Config) -> Self {
        let queue = config.send_queue.then(SendQueue::new);
        let encoding = wire_encoding(&config.encoding);
        Self {
            socket: stream,
            pending: crate::message::LineBuffer::default(),
            pump: Pump::new(config),
            queue,
            encoding,
            debug: None,
            wire: Vec::new(),
        }
    }

    /// Share the flag that turns raw-line capture on and off.
    pub fn set_debug(&mut self, flag: Arc<AtomicBool>) {
        self.debug = Some(flag);
    }

    /// Raw lines captured since the last call.
    pub fn take_wire(&mut self) -> Vec<(bool, String)> {
        std::mem::take(&mut self.wire)
    }

    /// Write queued own lines that the flood limit now allows. Call this
    /// often; it never blocks.
    pub fn flush_queue(&mut self) -> io::Result<()> {
        let Some(queue) = self.queue.as_mut() else {
            return Ok(());
        };
        for line in queue.take_ready(Instant::now()) {
            record(&self.debug, &mut self.wire, &self.pump, false, &line);
            write_line(&mut self.socket, self.encoding, &line)?;
        }
        Ok(())
    }

    /// Own lines still waiting to be sent.
    pub fn queued(&self) -> usize {
        self.queue.as_ref().map_or(0, SendQueue::len)
    }

    pub fn register(&mut self) -> io::Result<()> {
        for line in self.pump.registration_lines() {
            self.write_line(&line)?;
        }
        Ok(())
    }

    pub fn read_effect(&mut self) -> io::Result<Option<Effect>> {
        loop {
            if let Some(raw) = self.pending.next_line() {
                let line = match self.encoding {
                    Some(encoding) => encoding.decode_without_bom_handling(raw).0.into_owned(),
                    None => crate::message::decode_line(raw),
                };
                record(&self.debug, &mut self.wire, &self.pump, true, &line);
                let effect = self.pump.on_server_line(&line);
                for outgoing in &effect.outgoing {
                    record(&self.debug, &mut self.wire, &self.pump, false, outgoing);
                    write_line(&mut self.socket, self.encoding, outgoing)?;
                }
                return Ok(Some(effect));
            }
            let mut buf = [0u8; 4096];
            match self.socket.read(&mut buf) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "connection closed",
                    ));
                }
                Ok(read) => self.pending.extend(&buf[..read]),
                Err(err) if is_idle(&err) => return Ok(None),
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
    }

    pub fn handle_user(&mut self, line: &str) -> io::Result<Effect> {
        let effect = self.pump.on_user_line(line);
        match self.queue.as_mut() {
            // QUIT goes out at once, after anything already queued, so a
            // closing window still says goodbye.
            Some(queue) if effect.quit => {
                for queued in queue.drain() {
                    record(&self.debug, &mut self.wire, &self.pump, false, &queued);
                    write_line(&mut self.socket, self.encoding, &queued)?;
                }
                for outgoing in &effect.outgoing {
                    record(&self.debug, &mut self.wire, &self.pump, false, outgoing);
                    write_line(&mut self.socket, self.encoding, outgoing)?;
                }
            }
            Some(queue) => {
                for outgoing in &effect.outgoing {
                    queue.push(outgoing.clone());
                }
                self.flush_queue()?;
            }
            None => {
                for outgoing in &effect.outgoing {
                    record(&self.debug, &mut self.wire, &self.pump, false, outgoing);
                    write_line(&mut self.socket, self.encoding, outgoing)?;
                }
            }
        }
        Ok(effect)
    }

    fn write_line(&mut self, line: &str) -> io::Result<()> {
        record(&self.debug, &mut self.wire, &self.pump, false, line);
        write_line(&mut self.socket, self.encoding, line)
    }
}

/// Paces own lines the way IRC servers expect: a short burst, then one line
/// every two seconds, so a long paste does not end in an "excess flood"
/// disconnect. Server replies such as PONG do not wait here.
pub struct SendQueue {
    lines: VecDeque<String>,
    tokens: f32,
    refilled: Instant,
}

impl SendQueue {
    const BURST: f32 = 5.0;
    const PER_SECOND: f32 = 0.5;

    pub fn new() -> Self {
        Self::starting_at(Instant::now())
    }

    fn starting_at(now: Instant) -> Self {
        Self {
            lines: VecDeque::new(),
            tokens: Self::BURST,
            refilled: now,
        }
    }

    pub fn push(&mut self, line: String) {
        self.lines.push_back(line);
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// The lines that may be sent at `now`, oldest first.
    pub fn take_ready(&mut self, now: Instant) -> Vec<String> {
        let elapsed = now.saturating_duration_since(self.refilled).as_secs_f32();
        self.tokens = (self.tokens + elapsed * Self::PER_SECOND).min(Self::BURST);
        self.refilled = now;
        let mut ready = Vec::new();
        while self.tokens >= 1.0 {
            let Some(line) = self.lines.pop_front() else {
                break;
            };
            self.tokens -= 1.0;
            ready.push(line);
        }
        ready
    }

    fn drain(&mut self) -> Vec<String> {
        self.lines.drain(..).collect()
    }
}

/// The wire encoding for a server's setting; `None` for `auto` (and for
/// UTF-8, which needs no conversion).
pub fn wire_encoding(label: &str) -> Option<&'static encoding_rs::Encoding> {
    match label.trim().to_ascii_lowercase().as_str() {
        "" | "auto" | "utf-8" | "utf8" => None,
        other => encoding_rs::Encoding::for_label(other.as_bytes())
            .filter(|found| *found != encoding_rs::UTF_8),
    }
}

/// Write one IRC line. CR and LF inside `line` become spaces so text from a
/// script or a paste can never start a second command.
fn write_line(
    socket: &mut Socket,
    encoding: Option<&'static encoding_rs::Encoding>,
    line: &str,
) -> io::Result<()> {
    let clean: String = line
        .chars()
        .map(|ch| if ch == '\r' || ch == '\n' { ' ' } else { ch })
        .collect();
    match encoding {
        Some(encoding) => {
            let (bytes, _, _) = encoding.encode(&clean);
            socket.write_all(&bytes)?;
            socket.write_all(b"\r\n")?;
        }
        None => write!(socket, "{clean}\r\n")?,
    }
    socket.flush()
}

/// How to open a connection.
#[derive(Clone, Debug)]
pub struct ConnectOptions {
    pub tls: tls::TlsOptions,
    /// Limit for the TCP connect and the TLS handshake.
    pub timeout: Duration,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        Self {
            tls: tls::TlsOptions::default(),
            timeout: Duration::from_secs(30),
        }
    }
}

pub fn connect(
    host: &str,
    port: u16,
    use_tls: bool,
    proxy: Option<&crate::socks::Endpoint>,
    options: &ConnectOptions,
) -> io::Result<Socket> {
    let tcp = match proxy {
        Some(proxy) => proxy.connect(host, port)?,
        None => crate::net::dial(host, port, options.timeout)?,
    };
    tcp.set_nodelay(true)?;
    if use_tls {
        tcp.set_read_timeout(Some(options.timeout))?;
        tcp.set_write_timeout(Some(options.timeout))?;
        let (mut stream, rejected) = tls::wrap(host, tcp, &options.tls)?;
        while stream.conn.is_handshaking() {
            if let Err(err) = stream.conn.complete_io(&mut stream.sock) {
                if let Some((fingerprint, reason)) =
                    rejected.lock().ok().and_then(|slot| slot.clone())
                {
                    return Err(io::Error::other(tls::UntrustedCert {
                        host: host.to_string(),
                        fingerprint,
                        reason,
                    }));
                }
                return Err(io::Error::new(
                    err.kind(),
                    format!("TLS handshake with {host} failed: {err}"),
                ));
            }
        }
        stream.sock.set_write_timeout(None)?;
        stream.sock.set_read_timeout(Some(crate::idle_timeout()))?;
        Ok(Socket::tls(stream))
    } else {
        tcp.set_read_timeout(Some(crate::idle_timeout()))?;
        Ok(Socket::plain(tcp))
    }
}

fn is_idle(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;

    use super::*;

    #[test]
    fn debug_lines_hide_passwords() {
        assert_eq!(mask_secrets("PASS hunter2"), "PASS ****");
        assert_eq!(mask_secrets("OPER admin secret"), "OPER admin ****");
        assert_eq!(mask_secrets("AUTHENTICATE PLAIN"), "AUTHENTICATE PLAIN");
        assert_eq!(
            mask_secrets("AUTHENTICATE Ym9iAGJvYgBzZWNyZXQ="),
            "AUTHENTICATE ****"
        );
        assert_eq!(
            mask_secrets("PRIVMSG NickServ :IDENTIFY bob pw"),
            "PRIVMSG NickServ :IDENTIFY ****"
        );
        assert_eq!(mask_secrets("PRIVMSG #c :hello"), "PRIVMSG #c :hello");
        assert_eq!(mask_secrets("JOIN #a,#b k1,k2"), "JOIN #a,#b ****");
        assert_eq!(mask_secrets("JOIN #a"), "JOIN #a");
        // Without the server's mode list, every argument of a change with a
        // key is hidden; with it, only the key.
        assert_eq!(mask_secrets("MODE #c +ok bob key"), "MODE #c +ok **** ****");
        let known = crate::state::Directory::new();
        let masked = |line: &str| mask_secrets_with(line, &known);
        assert_eq!(masked("MODE #c +ok bob key"), "MODE #c +ok bob ****");
        assert_eq!(mask_secrets("MODE #c +b x!*@*"), "MODE #c +b x!*@*");
        assert_eq!(
            masked(":irc 324 me #c +lk 10 key"),
            ":irc 324 me #c +lk 10 ****"
        );
        assert_eq!(
            masked("@time=x :al!a@h MODE #c -l+k key"),
            "@time=x :al!a@h MODE #c -l+k ****"
        );
        assert_eq!(masked("MODE #c -k key"), "MODE #c -k ****");
        // A letter the server never described hides the rest.
        assert_eq!(masked("MODE #c +Xk setting key"), "MODE #c +Xk **** ****");
        assert_eq!(
            mask_secrets("PRIVMSG ChanServ :IDENTIFY #c pw"),
            "PRIVMSG ChanServ :IDENTIFY ****"
        );
    }

    #[test]
    fn services_are_known_by_name_and_logins_by_their_words() {
        for service in [
            "NickServ",
            "nickserv",
            "NS",
            "cs",
            "MemoServ",
            "ms",
            "hs",
            "bs",
            "os",
            "NickServ@services.example",
            "chanserv@services.example",
        ] {
            assert!(is_service(service), "{service}");
        }
        for person in [
            "Q",
            "X",
            "X@channels.undernet.org",
            "Q@CServe.quakenet.org",
            "bob",
            "nsa",
            "bob@nickserv",
        ] {
            assert!(!is_service(person), "{person}");
        }
        // To a service, any of its commands that take a password.
        for verb in [
            "IDENTIFY", "ID", "REGISTER", "GHOST", "RECOVER", "RELEASE", "REGAIN", "SET", "LOGIN",
            "AUTH", "NEWPASS", "identify",
        ] {
            assert_eq!(
                mask_service_text("NickServ@services.example", &format!("{verb} me swordfish")),
                Some(format!("{verb} ****")),
                "{verb}"
            );
        }
        assert_eq!(
            mask_service_text("NickServ", "SET email me@example"),
            Some("SET ****".into())
        );
        assert_eq!(mask_service_text("NickServ", "INFO bob"), None);
        // To anyone else, a login or a password change.
        for verb in [
            "AUTH", "LOGIN", "IDENTIFY", "ID", "REGISTER", "GHOST", "RECOVER", "RELEASE", "REGAIN",
            "NEWPASS", "auth",
        ] {
            for target in ["Q", "X@channels.undernet.org", "bob"] {
                assert_eq!(
                    mask_service_text(target, &format!("{verb} me swordfish")),
                    Some(format!("{verb} ****")),
                    "{verb} to {target}"
                );
            }
        }
        for (text, masked) in [
            ("SET PASSWORD swordfish", Some("SET ****")),
            ("set pass swordfish", Some("set ****")),
            ("set the table", None),
            ("SET email me@example", None),
            ("hello AUTH me swordfish", None),
            (
                "\u{1}ACTION AUTH me swordfish\u{1}",
                Some("\u{1}ACTION AUTH ****\u{1}"),
            ),
            ("\u{1}LOGIN me swordfish\u{1}", Some("\u{1}LOGIN ****\u{1}")),
        ] {
            assert_eq!(
                mask_service_text("Q", text),
                masked.map(str::to_string),
                "{text}"
            );
        }
        // A channel is no bot.
        for channel in ["#ops", "&local", "@#ops", "+modeless"] {
            assert_eq!(
                mask_service_text(channel, "AUTH me swordfish"),
                None,
                "{channel}"
            );
        }
        // The Debug window's copy, through the same reading.
        assert_eq!(
            mask_secrets("PRIVMSG X@channels.undernet.org :LOGIN me swordfish"),
            "PRIVMSG X@channels.undernet.org :LOGIN ****"
        );
        assert_eq!(
            mask_secrets("NOTICE Q :\u{1}AUTH me swordfish\u{1}"),
            "NOTICE Q :\u{1}AUTH ****\u{1}"
        );
        assert_eq!(
            mask_secrets("PRIVMSG #ops :Login page is broken"),
            "PRIVMSG #ops :Login page is broken"
        );
        assert!(opens_with_login("\u{1}ACTION regain carol swordfish\u{1}"));
        assert!(!opens_with_login("regards to all"));
    }

    #[test]
    fn debug_finds_keys_with_the_servers_own_mode_list() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            // X takes an argument here, so the key is the second one.
            sock.write_all(
                b":irc 005 me CHANMODES=beI,Xk,l,imnst PREFIX=(ov)@+ :are supported\r\n\
                  :al!a@h MODE #room +Xk setting swordfish\r\n\
                  :al!a@h MODE #room +l-k 5 swordfish\r\n",
            )
            .unwrap();
            thread::sleep(Duration::from_millis(300));
        });
        let tcp = TcpStream::connect(addr).unwrap();
        tcp.set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let mut conn = Connection::new(
            Socket::plain(tcp),
            Config::new("me", "me", "Me", Vec::new(), Vec::new()),
        );
        conn.set_debug(Arc::new(AtomicBool::new(true)));
        let mut wire = Vec::new();
        for _ in 0..40 {
            let _ = conn.read_effect().unwrap();
            wire.extend(conn.take_wire().into_iter().map(|(_, line)| line));
            if wire.len() >= 3 {
                break;
            }
        }
        assert_eq!(wire[1], ":al!a@h MODE #room +Xk setting ****");
        assert_eq!(wire[2], ":al!a@h MODE #room +l-k 5 ****");
        assert!(
            wire.iter().all(|line| !line.contains("swordfish")),
            "{wire:?}"
        );
        server.join().unwrap();
    }

    #[test]
    fn registers_pongs_joins_and_sends_chat() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut cap = String::new();
            let mut nick = String::new();
            let mut user = String::new();
            // This server does not negotiate capabilities; registration
            // goes on without an answer.
            reader.read_line(&mut cap).unwrap();
            assert_eq!(cap, "CAP LS 302\r\n");
            reader.read_line(&mut nick).unwrap();
            reader.read_line(&mut user).unwrap();
            assert_eq!(nick, "NICK :bob\r\n");
            assert_eq!(user, "USER bob 0 * :Bob\r\n");

            let mut writer = sock.try_clone().unwrap();
            writeln!(writer, "PING :unit").unwrap();
            let mut pong = String::new();
            reader.read_line(&mut pong).unwrap();
            assert_eq!(pong, "PONG :unit\r\n");

            writeln!(writer, ":unit.test 001 bob :Welcome to the test").unwrap();
            let mut join = String::new();
            reader.read_line(&mut join).unwrap();
            assert_eq!(join, "JOIN :#openirc\r\n");

            writeln!(writer, ":alice!a@h PRIVMSG #openirc :hello").unwrap();
            let mut said = String::new();
            reader.read_line(&mut said).unwrap();
            assert_eq!(said, "PRIVMSG #openirc :hi there\r\n");
        });

        let tcp = TcpStream::connect(addr).unwrap();
        tcp.set_nodelay(true).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut conn = Connection::new(
            Socket::plain(tcp),
            Config::new("bob", "bob", "Bob", vec!["#openirc".into()], Vec::new()),
        );
        conn.register().unwrap();

        let ping = conn.read_effect().unwrap().unwrap();
        assert_eq!(ping.lines, vec!["* pong"]);

        let welcome = conn.read_effect().unwrap().unwrap();
        assert!(welcome.lines[0].contains("Welcome"));
        assert!(welcome.outgoing.iter().any(|line| line == "JOIN :#openirc"));

        let chat = conn.read_effect().unwrap().unwrap();
        assert_eq!(chat.lines, vec!["[#openirc] <alice> hello"]);

        let sent = conn.handle_user("hi there").unwrap();
        assert_eq!(sent.outgoing, vec!["PRIVMSG #openirc :hi there"]);

        server.join().unwrap();
    }

    #[test]
    fn split_lines_survive_idle_timeouts_and_latin1_does_not_disconnect() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.write_all(b":alice!a@h PRIVMSG #c :this line is sp")
                .unwrap();
            sock.flush().unwrap();
            thread::sleep(Duration::from_millis(250));
            sock.write_all(b"lit in two\r\n:bob!b@h PRIVMSG #c :caf\xe9\r\n")
                .unwrap();
            thread::sleep(Duration::from_millis(500));
        });

        let tcp = TcpStream::connect(addr).unwrap();
        tcp.set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let mut conn = Connection::new(
            Socket::plain(tcp),
            Config::new("me", "me", "Me", Vec::new(), Vec::new()),
        );
        let mut lines = Vec::new();
        for _ in 0..40 {
            match conn.read_effect().unwrap() {
                Some(effect) => lines.extend(effect.lines),
                None if lines.len() >= 2 => break,
                None => {}
            }
        }
        assert_eq!(
            lines,
            vec![
                "[#c] <alice> this line is split in two".to_string(),
                "[#c] <bob> caf\u{e9}".to_string(),
            ]
        );
        server.join().unwrap();
    }

    #[test]
    fn send_queue_allows_a_burst_then_paces() {
        let start = Instant::now();
        let mut queue = SendQueue::starting_at(start);
        for n in 0..8 {
            queue.push(format!("line {n}"));
        }
        assert_eq!(queue.take_ready(start).len(), 5);
        assert!(
            queue
                .take_ready(start + Duration::from_millis(1_500))
                .is_empty()
        );
        assert_eq!(
            queue.take_ready(start + Duration::from_millis(2_100)),
            vec!["line 5".to_string()]
        );
        assert_eq!(queue.take_ready(start + Duration::from_secs(60)).len(), 2);
        assert_eq!(queue.len(), 0);
    }
}
