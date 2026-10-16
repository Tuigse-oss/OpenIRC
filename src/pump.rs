//! Session state for registration, the message pump, and user commands.
//!
//! This is original OpenIRC behavior. Server lines become events and, when
//! required, replies such as PONG. User lines become IRC commands.

use crate::NAME;
use crate::display::format_event;
use crate::flood::{self, Gate, Limits, Verdict};
use crate::message::{self, Message, Prefix};
use crate::state::Directory;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct Config {
    pub nick: String,
    pub user: String,
    pub realname: String,
    pub autojoin: Vec<String>,
    pub ignore: Vec<String>,
    /// Ignore rules with types, networks, and expiry.
    pub ignore_rules: Vec<crate::rules::IgnoreRule>,
    pub flood_messages: u32,
    pub flood_joins: u32,
    pub flood_window_ms: u64,
    /// CTCP requests answered per flood window, from everyone together.
    pub flood_ctcp: u32,
    pub sasl_password: String,
    /// SASL account. Blank uses the configured nick.
    pub sasl_user: String,
    /// Tried first when the nick is taken during registration.
    pub alt_nick: String,
    /// Default /quit reason. Blank uses the OpenIRC version.
    pub quit_message: String,
    /// CTCP FINGER reply. Blank uses the real name.
    pub finger_reply: String,
    /// Pace own lines with `connection::SendQueue`.
    pub send_queue: bool,
    /// Text encoding on the wire, such as `auto` or `windows-1251`.
    pub encoding: String,
    /// Server password, sent as `PASS` first.
    pub server_password: String,
    /// How to log in to services.
    pub login: crate::servers::Login,
    /// Negotiate IRCv3 capabilities with `CAP LS 302`. Off only asks for
    /// SASL when a login needs it.
    pub ircv3: bool,
    /// Nicks to watch: online and offline are reported.
    pub notify: Vec<String>,
}

impl Config {
    pub fn new(
        nick: impl Into<String>,
        user: impl Into<String>,
        realname: impl Into<String>,
        autojoin: Vec<String>,
        ignore: Vec<String>,
    ) -> Self {
        Self {
            nick: nick.into(),
            user: user.into(),
            realname: realname.into(),
            autojoin,
            ignore,
            ignore_rules: Vec::new(),
            flood_messages: flood::DEFAULT_MESSAGES,
            flood_joins: flood::DEFAULT_JOINS,
            flood_window_ms: flood::DEFAULT_WINDOW_MS,
            flood_ctcp: flood::DEFAULT_CTCP,
            sasl_password: String::new(),
            sasl_user: String::new(),
            alt_nick: String::new(),
            quit_message: String::new(),
            finger_reply: String::new(),
            send_queue: false,
            encoding: "auto".into(),
            server_password: String::new(),
            login: crate::servers::Login::None,
            ircv3: true,
            notify: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Registered {
        nick: String,
        text: String,
    },
    Chat {
        channel: bool,
        target: String,
        from: String,
        text: String,
        action: bool,
    },
    Notice {
        from: String,
        text: String,
    },
    /// A CTCP request someone sent us, such as VERSION.
    Ctcp {
        from: String,
        command: String,
        text: String,
    },
    /// The answer to a CTCP request we sent.
    CtcpReply {
        from: String,
        command: String,
        text: String,
    },
    Join {
        who: String,
        channel: String,
        /// `user@host` of the joiner, when the server sent it.
        address: String,
    },
    /// Someone joined while join lines were being held back for flood.
    /// The window adds them to the nick list without printing a line.
    Member {
        who: String,
        channel: String,
    },
    /// Someone invited you to a channel.
    Invite {
        from: String,
        channel: String,
    },
    /// How the server compares nicks and channel names (CASEMAPPING=).
    CaseMapping {
        map: crate::state::CaseMap,
    },
    /// The network name from the server's NETWORK= token.
    Network {
        name: String,
    },
    /// A channel's member list changed status (op, voice, ...). Quiet:
    /// the window refreshes its nick list without printing a line.
    Members {
        channel: String,
        names: Vec<String>,
    },
    /// Someone else went away or came back (IRCv3 away-notify).
    UserAway {
        who: String,
        reason: Option<String>,
    },
    /// Someone logged in to or out of a services account.
    Account {
        who: String,
        account: Option<String>,
    },
    /// Someone's user name or host changed (IRCv3 chghost).
    HostChange {
        who: String,
        address: String,
    },
    /// Someone invited another person to a channel we are in.
    InviteNotify {
        by: String,
        who: String,
        channel: String,
    },
    /// Typing state: `active`, `paused`, or `done`. The target is the
    /// channel, or the sender for a private conversation.
    Typing {
        who: String,
        target: String,
        state: String,
    },
    /// The server's strict transport security policy: the TLS port to use
    /// on a plain connection, and on a TLS one how long to remember it.
    Sts {
        port: Option<u16>,
        duration: Option<u64>,
    },
    /// The capabilities in use, after each change.
    Caps {
        enabled: Vec<String>,
    },
    /// The individual changes of a channel MODE line, and who made them.
    /// Quiet: the MODE line itself is shown through `Raw`.
    ModeChange {
        by: String,
        channel: String,
        changes: Vec<(bool, char, Option<String>)>,
    },
    /// A channel's flags, such as `+ntk key`.
    ChannelModes {
        channel: String,
        modes: String,
    },
    Part {
        who: String,
        channel: String,
        reason: Option<String>,
    },
    Quit {
        who: String,
        reason: Option<String>,
    },
    Nick {
        old: String,
        new: String,
    },
    Kick {
        who: String,
        channel: String,
        by: String,
        reason: Option<String>,
    },
    Numeric {
        code: u16,
        text: String,
    },
    Topic {
        channel: String,
        topic: Option<String>,
    },
    DccChat {
        from: String,
        host: String,
        port: u16,
    },
    DccSend {
        from: String,
        filename: String,
        host: String,
        port: u16,
        size: Option<u64>,
        token: Option<String>,
    },
    DccAccept {
        from: String,
        filename: String,
        port: u16,
        position: u64,
    },
    /// The receiver of a file we offered asks to continue at `position`.
    DccResume {
        from: String,
        filename: String,
        port: u16,
        position: u64,
    },
    Away {
        reason: Option<String>,
    },
    Echo {
        text: String,
    },
    Clear,
    Names {
        channel: String,
        names: Vec<String>,
        /// The user asked for the list with /names; otherwise it is the
        /// list the server sends on join, which the window summarizes.
        listed: bool,
    },
    /// Someone changed a channel's topic.
    TopicChanged {
        by: String,
        channel: String,
        topic: String,
    },
    /// A line about a channel for its window, such as when its topic was
    /// set or when it was created.
    ChannelInfo {
        channel: String,
        text: String,
    },
    /// Our own user modes, such as `+iSx`.
    UserModes {
        modes: String,
    },
    /// Someone on the notify list came online or left.
    Notify {
        nick: String,
        /// `user@host` when the server gave it.
        address: String,
        online: bool,
    },
    Ping,
    Raw {
        command: String,
        text: String,
    },
    Sent {
        target: String,
        from: String,
        text: String,
        action: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effect {
    pub lines: Vec<String>,
    pub events: Vec<Event>,
    pub outgoing: Vec<String>,
    pub quit: bool,
    /// When the server says the line happened (IRCv3 server-time), in
    /// seconds since 1970.
    pub time: Option<i64>,
    /// The nick that sent the line and its current `user@host` (empty when
    /// the server left it out), so trust checks use who is speaking now.
    pub sender: Option<(String, String)>,
    /// How many nick changes and quits the connection had seen once this
    /// was done: the window side writes its lines against it (`Via::seen`).
    pub renames: u64,
}

impl Effect {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            events: Vec::new(),
            outgoing: Vec::new(),
            quit: false,
            time: None,
            sender: None,
            renames: 0,
        }
    }

    /// Take in what another line led to, after what is here.
    fn absorb(&mut self, other: Effect) {
        self.lines.extend(other.lines);
        self.events.extend(other.events);
        self.outgoing.extend(other.outgoing);
        self.quit |= other.quit;
    }

    fn push_event(&mut self, event: Event) {
        self.lines.push(format_event(&event));
        self.events.push(event);
    }
}

#[derive(Debug, Clone)]
pub struct Pump {
    config: Config,
    nick: String,
    nick_retries: u8,
    registered: bool,
    target: Option<String>,
    /// Set when a command chose the target on purpose (`/target`, `/query`,
    /// `/win`, `/join`), so `/via` keeps that choice even when it names the
    /// same target.
    picked: bool,
    /// The target a line chose on purpose within a run of `/via` lines,
    /// which the rest of that run follows.
    run_choice: Option<RunChoice>,
    /// How many nick changes and quits of others this connection has seen,
    /// and the latest of them (their count, the old nick, and the new one,
    /// or none for a quit), so a line the window side wrote for a nick
    /// before it saw it change still reaches that person, and never whoever
    /// takes the nick of someone who left.
    renames: u64,
    rename_log: std::collections::VecDeque<(u64, String, Option<String>)>,
    joined_auto: bool,
    dir: Directory,
    ignored: Vec<crate::rules::IgnoreRule>,
    /// The network name from `005 NETWORK=`, for network-specific rules.
    network: Option<String>,
    started: std::time::Instant,
    flood: Gate,
    /// Capabilities the server offers, with their values.
    offered: Vec<(String, String)>,
    /// Capabilities in use.
    caps: Vec<String>,
    /// Waiting for capability negotiation to finish before `CAP END`.
    negotiating: bool,
    /// Channels whose names list the user asked for, by folded name.
    names_wanted: Vec<String>,
    /// Our own user modes, without the `+`.
    umodes: String,
    /// The server watches nicks for us (`MONITOR` in 005); without it the
    /// window asks with `ISON` every minute.
    monitor: bool,
    /// The notify list has been sent (after the message of the day).
    notify_started: bool,
    /// Notify nicks seen online, folded.
    notify_online: Vec<String>,
}

/// A target chosen on purpose by a line of a run of `/via` lines.
#[derive(Debug, Clone)]
struct RunChoice {
    /// The run's number.
    run: u64,
    target: String,
    /// The one chosen has left (quit) since, so the rest of the run's words
    /// go nowhere: not to whoever takes the nick, nor to the window the run
    /// began in.
    left: bool,
}

/// Why a line written for a nick before some changes cannot be sent to it.
#[derive(Debug, PartialEq, Eq)]
enum Refusal {
    /// Changes it needs were forgotten (`RENAMES_KEPT`).
    Forgotten,
    /// The one it was for left (quit): whoever has the nick now is someone
    /// else.
    Gone,
}

/// How many nick changes and quits are remembered for lines written before
/// them. Quits count too, so one netsplit on a big channel brings hundreds
/// at once; a line queued across it is still followed rather than refused.
const RENAMES_KEPT: usize = 4096;

/// IRCv3 capabilities OpenIRC asks for when the server offers them.
const WANTED_CAPS: &[&str] = &[
    "multi-prefix",
    "away-notify",
    "account-notify",
    "extended-join",
    "chghost",
    "server-time",
    "message-tags",
    "invite-notify",
    "cap-notify",
    "userhost-in-names",
];

impl crate::connection::ModeTable for Pump {
    fn mode_param(&self, letter: char, adding: bool) -> Option<bool> {
        self.dir.mode_param(letter, adding)
    }
}

impl Pump {
    pub fn new(config: Config) -> Self {
        let nick = config.nick.clone();
        let ignored: Vec<crate::rules::IgnoreRule> = config
            .ignore
            .iter()
            .map(|mask| crate::rules::IgnoreRule::all(mask))
            .chain(config.ignore_rules.iter().cloned())
            .collect();
        let target = config.autojoin.first().cloned();
        let flood = Gate::new(Limits {
            messages: config.flood_messages,
            joins: config.flood_joins,
            window_ms: config.flood_window_ms,
            ctcp: config.flood_ctcp,
        });
        Self {
            config,
            nick,
            nick_retries: 0,
            registered: false,
            target,
            picked: false,
            run_choice: None,
            renames: 0,
            rename_log: std::collections::VecDeque::new(),
            joined_auto: false,
            dir: Directory::new(),
            ignored,
            network: None,
            started: std::time::Instant::now(),
            flood,
            offered: Vec::new(),
            caps: Vec::new(),
            negotiating: false,
            names_wanted: Vec::new(),
            umodes: String::new(),
            monitor: false,
            notify_started: false,
            notify_online: Vec::new(),
        }
    }

    pub fn nick(&self) -> &str {
        &self.nick
    }

    /// The SASL account: the configured one, else the nick from the
    /// settings. Never the current nick, which may be `nick_` after a
    /// collision during registration.
    fn sasl_account(&self) -> &str {
        if self.config.sasl_user.trim().is_empty() {
            &self.config.nick
        } else {
            self.config.sasl_user.trim()
        }
    }

    /// The login method in use: the configured one, or SASL PLAIN when only
    /// the older global SASL password is set.
    fn login(&self) -> crate::servers::Login {
        use crate::servers::Login;
        match self.config.login {
            Login::None if !self.config.sasl_password.is_empty() => Login::SaslPlain,
            Login::SaslPlain if self.config.sasl_password.is_empty() => Login::None,
            Login::NickServ if self.config.sasl_password.is_empty() => Login::None,
            other => other,
        }
    }

    fn uses_sasl(&self) -> bool {
        matches!(
            self.login(),
            crate::servers::Login::SaslPlain | crate::servers::Login::SaslExternal
        )
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    pub fn registration_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if !self.config.server_password.is_empty() {
            lines.push(Message::command("PASS", [self.config.server_password.as_str()]).encode());
        }
        if self.config.ircv3 {
            lines.push("CAP LS 302".to_string());
        } else if self.uses_sasl() {
            lines.push("CAP REQ :sasl".to_string());
        }
        lines.push(Message::command("NICK", [self.nick.as_str()]).encode());
        lines.push(
            Message::command(
                "USER",
                [
                    self.config.user.as_str(),
                    "0",
                    "*",
                    self.config.realname.as_str(),
                ],
            )
            .encode(),
        );
        lines
    }

    pub fn on_server_line(&mut self, raw: &str) -> Effect {
        let mut effect = self.server_line(raw);
        effect.renames = self.renames;
        effect
    }

    fn server_line(&mut self, raw: &str) -> Effect {
        let mut effect = Effect::new();
        let message = match Message::parse(raw) {
            Ok(message) => message,
            Err(err) => {
                effect.lines.push(format!("* ignored server line ({err})"));
                return effect;
            }
        };

        effect.sender = match &message.prefix {
            Some(Prefix::User { nick, user, host }) => {
                let address = match (user, host) {
                    (Some(user), Some(host)) => format!("{user}@{host}"),
                    _ => String::new(),
                };
                Some((nick.clone(), address))
            }
            // A bare name without a dot is a nick whose address the server
            // left out.
            Some(Prefix::Server(name)) if !name.contains('.') => {
                Some((name.clone(), String::new()))
            }
            _ => None,
        };
        effect.time = message
            .tags
            .iter()
            .find(|(key, _)| key == "time")
            .and_then(|(_, value)| crate::clock::parse_server_time(value));

        match message.command.as_str() {
            "PING" => {
                let token = message.params.first().cloned().unwrap_or_default();
                effect
                    .outgoing
                    .push(Message::command("PONG", [token]).encode());
                effect.push_event(Event::Ping);
            }
            "PRIVMSG" => self.on_privmsg(&message, &mut effect),
            "NOTICE" => {
                let from = message
                    .prefix
                    .as_ref()
                    .map(Prefix::name)
                    .unwrap_or("server")
                    .to_string();
                if self.is_ignored(&message, 'n') {
                    return effect;
                }
                let text = message.trailing().unwrap_or("").to_string();
                if let Some(body) = ctcp_body(&text) {
                    if self.is_ignored(&message, 't') {
                        return effect;
                    }
                    let (command, rest) = body.split_once(' ').unwrap_or((body, ""));
                    let mut text = rest.trim().to_string();
                    if command.eq_ignore_ascii_case("PING")
                        && let Ok(sent) = text.parse::<u64>()
                    {
                        let lag = unix_ms().saturating_sub(sent);
                        text = format!("{}.{:03} seconds", lag / 1_000, lag % 1_000);
                    }
                    effect.push_event(Event::CtcpReply {
                        from,
                        command: command.to_ascii_uppercase(),
                        text,
                    });
                    return effect;
                }
                effect.push_event(Event::Notice { from, text });
            }
            "JOIN" => {
                let who = who_name(&message);
                let address = match &message.prefix {
                    Some(Prefix::User {
                        user: Some(user),
                        host: Some(host),
                        ..
                    }) => format!("{user}@{host}"),
                    _ => String::new(),
                };
                if let Some(account) = message.params.get(1).filter(|account| *account != "*") {
                    effect.events.push(Event::Account {
                        who: who.clone(),
                        account: Some(account.clone()),
                    });
                }
                if let Some(channel) = message.params.first().cloned() {
                    self.dir.joined(&channel, &who);
                    if self.dir.same(&who, &self.nick) {
                        self.target = Some(channel.clone());
                        // The channel's modes and creation time.
                        effect.outgoing.push(format!("MODE {channel}"));
                        effect.push_event(Event::Join {
                            who,
                            channel,
                            address: address.clone(),
                        });
                    } else {
                        match self.flood.observe_join(&channel, self.now_ms()) {
                            Verdict::Allow => effect.push_event(Event::Join {
                                who,
                                channel,
                                address,
                            }),
                            Verdict::Holding => {
                                effect.events.push(Event::Member { who, channel });
                            }
                            Verdict::Tripped => {
                                effect
                                    .lines
                                    .push(format!("* hiding join lines in {channel}"));
                                effect.events.push(Event::Member { who, channel });
                            }
                        }
                    }
                }
            }
            "PART" => {
                let who = who_name(&message);
                if let Some(channel) = message.params.first().cloned() {
                    let reason = message.params.get(1).cloned();
                    if self.dir.same(&who, &self.nick) {
                        self.dir.left(&channel);
                        self.select_after_leave(&channel);
                    } else {
                        self.dir.remove_member(&channel, &who);
                    }
                    effect.push_event(Event::Part {
                        who,
                        channel,
                        reason,
                    });
                }
            }
            "KICK" => {
                let by = who_name(&message);
                if let (Some(channel), Some(who)) = (
                    message.params.first().cloned(),
                    message.params.get(1).cloned(),
                ) {
                    let reason = message.params.get(2).cloned();
                    if self.dir.same(&who, &self.nick) {
                        self.dir.left(&channel);
                        self.select_after_leave(&channel);
                    } else {
                        self.dir.remove_member(&channel, &who);
                    }
                    effect.push_event(Event::Kick {
                        who,
                        channel,
                        by,
                        reason,
                    });
                }
            }
            "QUIT" => {
                let who = who_name(&message);
                // Someone else's nick is free now, for anyone to take: what
                // was meant for them before this must not reach the next
                // holder. A kill reaches us as a quit too.
                if !self.dir.same(&who, &self.nick) {
                    if self
                        .target
                        .as_ref()
                        .is_some_and(|target| self.dir.same(target, &who))
                    {
                        self.target = None;
                    }
                    // The rest of a run that chose them says nothing, rather
                    // than speak to the window it began in.
                    if let Some(choice) = &mut self.run_choice
                        && self.dir.same(&choice.target, &who)
                    {
                        choice.left = true;
                    }
                    self.note_rename(&who, None);
                }
                self.dir.quit(&who);
                let reason = message.params.first().cloned();
                effect.push_event(Event::Quit { who, reason });
            }
            "NICK" => {
                let old = who_name(&message);
                if let Some(new) = message.params.first().cloned() {
                    self.dir.rename(&old, &new);
                    if self.dir.same(&old, &self.nick) {
                        self.nick = new.clone();
                    }
                    if self
                        .target
                        .as_ref()
                        .is_some_and(|target| self.dir.same(target, &old))
                    {
                        self.target = Some(new.clone());
                    }
                    // So the rest of a run never reaches whoever takes the
                    // old nick.
                    if let Some(choice) = &mut self.run_choice
                        && !choice.left
                        && self.dir.same(&choice.target, &old)
                    {
                        choice.target = new.clone();
                    }
                    self.note_rename(&old, Some(&new));
                    effect.push_event(Event::Nick { old, new });
                }
            }
            "MODE"
                if message
                    .params
                    .first()
                    .is_some_and(|target| self.dir.same(target, &self.nick)) =>
            {
                let change = message.params.get(1).map(String::as_str).unwrap_or("");
                let mut adding = true;
                for letter in change.chars() {
                    match letter {
                        '+' => adding = true,
                        '-' => adding = false,
                        letter if adding => {
                            if !self.umodes.contains(letter) {
                                self.umodes.push(letter);
                            }
                        }
                        letter => self.umodes.retain(|held| held != letter),
                    }
                }
                effect.events.push(Event::UserModes {
                    modes: format!("+{}", self.umodes),
                });
                effect.push_event(Event::Raw {
                    command: "MODE".into(),
                    text: message.params.join(" "),
                });
            }
            "MODE" => {
                let channel = message.params.first().cloned().unwrap_or_default();
                if self.dir.is_channel(&channel) {
                    let changes = self.dir.mode_changes(&message.params);
                    if !changes.is_empty() {
                        effect.events.push(Event::ModeChange {
                            by: who_name(&message),
                            channel: channel.clone(),
                            changes,
                        });
                    }
                }
                let before = self.dir.mode_string(&channel);
                let changed = self.dir.apply_mode(&message.params);
                let after = self.dir.mode_string(&channel);
                if !changed.is_empty()
                    && let Some(names) = self.dir.names(&channel)
                {
                    effect.events.push(Event::Members {
                        channel: channel.clone(),
                        names,
                    });
                }
                if before != after
                    && let Some(modes) = after
                {
                    effect.events.push(Event::ChannelModes {
                        channel: channel.clone(),
                        modes,
                    });
                }
                effect.push_event(Event::Raw {
                    command: "MODE".into(),
                    text: message.params.join(" "),
                });
            }
            "INVITE" => {
                let from = who_name(&message);
                if let Some(channel) = message.params.get(1).cloned() {
                    if self.is_ignored(&message, 'i') {
                        return effect;
                    }
                    if let Some(who) = message.params.first()
                        && !self.dir.same(who, &self.nick)
                    {
                        effect.push_event(Event::InviteNotify {
                            by: from,
                            who: who.clone(),
                            channel,
                        });
                        return effect;
                    }
                    effect.push_event(Event::Invite { from, channel });
                }
            }
            "WALLOPS" => {
                let text = message.trailing().unwrap_or("").to_string();
                effect.push_event(Event::Notice {
                    from: "WALLOPS".into(),
                    text,
                });
            }
            "CAP" => self.on_cap(&message, &mut effect),
            "TOPIC" => {
                if let Some(channel) = message.params.first().cloned() {
                    let topic = message.params.get(1).cloned().unwrap_or_default();
                    self.dir.set_topic(&channel, Some(topic.clone()));
                    effect.push_event(Event::TopicChanged {
                        by: who_name(&message),
                        channel,
                        topic,
                    });
                }
            }
            "AWAY" => {
                let who = who_name(&message);
                let reason = message
                    .params
                    .first()
                    .cloned()
                    .filter(|reason| !reason.is_empty());
                effect.events.push(Event::UserAway { who, reason });
            }
            "ACCOUNT" => {
                let who = who_name(&message);
                let account = message
                    .params
                    .first()
                    .cloned()
                    .filter(|account| account != "*");
                effect.events.push(Event::Account { who, account });
            }
            "CHGHOST" => {
                if let [user, host, ..] = message.params.as_slice() {
                    effect.events.push(Event::HostChange {
                        who: who_name(&message),
                        address: format!("{user}@{host}"),
                    });
                }
            }
            "TAGMSG" => {
                let who = who_name(&message);
                let Some(target) = message.params.first() else {
                    return effect;
                };
                if self.dir.same(&who, &self.nick) || self.is_ignored(&message, 'p') {
                    return effect;
                }
                if let Some((_, state)) = message.tags.iter().find(|(key, _)| key == "+typing") {
                    let target = if self.dir.is_channel(target) {
                        target.clone()
                    } else {
                        who.clone()
                    };
                    effect.events.push(Event::Typing {
                        who,
                        target,
                        state: state.clone(),
                    });
                }
            }
            "AUTHENTICATE" => {
                if message.params.first().is_some_and(|param| param == "+") {
                    match self.login() {
                        crate::servers::Login::SaslPlain => {
                            effect.outgoing.extend(authenticate_lines(&sasl_plain(
                                self.sasl_account(),
                                &self.config.sasl_password,
                            )))
                        }
                        // The certificate is the credential; the answer is empty.
                        crate::servers::Login::SaslExternal => {
                            effect.outgoing.push("AUTHENTICATE +".into())
                        }
                        _ => {}
                    }
                }
            }
            other if is_numeric(other) => self.on_numeric(other, &message, &mut effect),
            other => {
                let text = message.params.join(" ");
                effect.push_event(Event::Raw {
                    command: other.to_string(),
                    text,
                });
            }
        }
        effect
    }

    pub fn on_user_line(&mut self, raw: &str) -> Effect {
        let mut effect = Effect::new();
        let line = raw.trim();
        if let Some(command) = line.strip_prefix('/') {
            self.on_command(command, &mut effect);
        } else if !line.is_empty() {
            self.send_chat(line, false, &mut effect);
        }
        effect.renames = self.renames;
        effect
    }

    fn on_privmsg(&mut self, message: &Message, effect: &mut Effect) {
        let Some(target) = message.params.first().cloned() else {
            return;
        };
        let from = who_name(message);
        let raw_text = message.trailing().unwrap_or("");
        // The CTCP command word, read once and in capitals, so the ignore
        // check and the handling below always agree on what it is.
        let ctcp_command = ctcp_body(raw_text).map(|ctcp| {
            ctcp.split_once(' ')
                .map_or(ctcp, |(command, _)| command)
                .to_ascii_uppercase()
        });
        let kind = match ctcp_command.as_deref() {
            Some("ACTION") => {
                if self.dir.is_channel(&target) {
                    'c'
                } else {
                    'p'
                }
            }
            Some("DCC") => 'd',
            Some(_) => 't',
            None if self.dir.is_channel(&target) => 'c',
            None => 'p',
        };
        if self.is_ignored(message, kind) {
            return;
        }
        match self.flood.observe_message(&from, self.now_ms()) {
            Verdict::Allow => {}
            Verdict::Holding => return,
            Verdict::Tripped => {
                effect
                    .lines
                    .push(format!("* dropping messages from {from} (flood)"));
                return;
            }
        }
        // A `k` ignore removes colors from what is shown. It never changes
        // what the message is: the CTCP command read above, from the text as
        // it came, decides both the ignore and the handling.
        let strip_codes = self.is_ignored(message, 'k');
        let shown_text = |text: &str| {
            if strip_codes {
                crate::colors::strip(text)
            } else {
                text.to_string()
            }
        };
        let channel = self.dir.is_channel(&target);
        if let (Some(ctcp), Some(command)) = (ctcp_body(raw_text), ctcp_command.as_deref()) {
            let (cmd, arg) = ctcp
                .split_once(' ')
                .map(|(cmd, arg)| (cmd, arg.trim()))
                .unwrap_or((ctcp, ""));
            if command == "ACTION" {
                let shown = if channel { target } else { from.clone() };
                effect.push_event(Event::Chat {
                    channel,
                    target: shown,
                    from,
                    text: shown_text(arg),
                    action: true,
                });
                return;
            }
            if command == "DCC"
                && let Some((host, port)) = crate::dcc::parse_dcc_chat(arg)
            {
                effect.push_event(Event::DccChat { from, host, port });
                return;
            }
            if command == "DCC"
                && let Some(offer) = crate::dcc_send::parse_dcc_send(arg)
            {
                effect.push_event(Event::DccSend {
                    from,
                    filename: offer.filename,
                    host: offer.host,
                    port: offer.port,
                    size: offer.size,
                    token: offer.token,
                });
                return;
            }
            if command == "DCC"
                && let Some((filename, port, position)) = crate::dcc_send::parse_dcc_resume(arg)
            {
                effect.push_event(Event::DccResume {
                    from,
                    filename,
                    port,
                    position,
                });
                return;
            }
            if command == "DCC"
                && let Some((filename, port, position)) = crate::dcc_send::parse_dcc_accept(arg)
            {
                effect.push_event(Event::DccAccept {
                    from,
                    filename,
                    port,
                    position,
                });
                return;
            }
            let request = Event::Ctcp {
                from: from.clone(),
                command: command.to_string(),
                text: arg.to_string(),
            };
            let Some(reply) = self.ctcp_answer(cmd, arg) else {
                effect.push_event(request);
                return;
            };
            match self.flood.observe_ctcp(self.now_ms()) {
                Verdict::Allow => {
                    effect.push_event(request);
                    effect.outgoing.extend(message::notice_lines(&from, &reply));
                }
                Verdict::Tripped => effect
                    .lines
                    .push("* not answering CTCP requests for a while (flood)".into()),
                Verdict::Holding => {}
            }
            return;
        }
        let shown = if channel { target } else { from.clone() };
        effect.push_event(Event::Chat {
            channel,
            target: shown,
            from,
            text: shown_text(raw_text),
            action: false,
        });
    }

    fn on_cap(&mut self, message: &Message, effect: &mut Effect) {
        let sub = message
            .params
            .get(1)
            .map(|sub| sub.to_ascii_uppercase())
            .unwrap_or_default();
        let list = message.trailing().unwrap_or("");
        // `CAP * LS * :...` means more lines follow.
        let more = message.params.len() >= 4 && message.params[2] == "*";
        match sub.as_str() {
            "LS" | "NEW" => {
                let mut fresh = Vec::new();
                for token in list.split_whitespace() {
                    let (name, value) = token.split_once('=').unwrap_or((token, ""));
                    let name = name.to_ascii_lowercase();
                    self.offered.retain(|(offered, _)| offered != &name);
                    self.offered.push((name.clone(), value.to_string()));
                    fresh.push(name);
                }
                if sub == "LS" {
                    self.negotiating = !self.registered;
                    if more {
                        return;
                    }
                    fresh = self.offered.iter().map(|(name, _)| name.clone()).collect();
                }
                if let Some((_, value)) = self.offered.iter().find(|(name, _)| name == "sts")
                    && fresh.iter().any(|name| name == "sts")
                {
                    effect.events.push(sts_event(value));
                }
                self.request_caps(&fresh, effect);
            }
            "ACK" => {
                for cap in list.split_whitespace() {
                    let cap = cap.to_ascii_lowercase();
                    match cap.strip_prefix('-') {
                        Some(removed) => self.caps.retain(|enabled| enabled != removed),
                        None if !self.caps.contains(&cap) => self.caps.push(cap),
                        None => {}
                    }
                }
                effect.events.push(Event::Caps {
                    enabled: self.caps.clone(),
                });
                let sasl = list
                    .split_whitespace()
                    .any(|cap| cap.eq_ignore_ascii_case("sasl"));
                if sasl && self.uses_sasl() && !self.registered {
                    let mechanism = if self.login() == crate::servers::Login::SaslExternal {
                        "EXTERNAL"
                    } else {
                        "PLAIN"
                    };
                    effect.outgoing.push(format!("AUTHENTICATE {mechanism}"));
                } else {
                    self.end_negotiation(effect);
                }
            }
            "NAK" => {
                if list
                    .split_whitespace()
                    .any(|cap| cap.eq_ignore_ascii_case("sasl"))
                    && self.uses_sasl()
                {
                    effect
                        .lines
                        .push("* the server refused SASL; continuing without it".into());
                }
                if !self.registered {
                    self.negotiating = true;
                }
                self.end_negotiation(effect);
            }
            "DEL" => {
                for cap in list.split_whitespace() {
                    let cap = cap.to_ascii_lowercase();
                    self.caps.retain(|enabled| enabled != &cap);
                    self.offered.retain(|(offered, _)| offered != &cap);
                }
                effect.events.push(Event::Caps {
                    enabled: self.caps.clone(),
                });
            }
            _ => {}
        }
    }

    /// Ask for the wanted capabilities among `offered`, or finish
    /// negotiation when there are none.
    fn request_caps(&mut self, offered: &[String], effect: &mut Effect) {
        let mut wanted: Vec<&str> = WANTED_CAPS
            .iter()
            .copied()
            .filter(|cap| offered.iter().any(|name| name == cap))
            .filter(|cap| !self.caps.iter().any(|enabled| enabled == cap))
            .collect();
        if self.uses_sasl() && !self.registered {
            let mechanism = if self.login() == crate::servers::Login::SaslExternal {
                "EXTERNAL"
            } else {
                "PLAIN"
            };
            match self.offered.iter().find(|(name, _)| name == "sasl") {
                Some((_, mechanisms))
                    if mechanisms.is_empty()
                        || mechanisms
                            .split(',')
                            .any(|item| item.eq_ignore_ascii_case(mechanism)) =>
                {
                    if offered.iter().any(|name| name == "sasl") {
                        wanted.push("sasl");
                    }
                }
                Some(_) => effect.lines.push(format!(
                    "* the server does not offer SASL {mechanism}; continuing without it"
                )),
                None => effect
                    .lines
                    .push("* the server does not offer SASL; continuing without it".into()),
            }
        }
        if wanted.is_empty() {
            self.end_negotiation(effect);
        } else {
            effect
                .outgoing
                .push(format!("CAP REQ :{}", wanted.join(" ")));
        }
    }

    fn end_negotiation(&mut self, effect: &mut Effect) {
        if self.negotiating && !self.registered {
            effect.outgoing.push("CAP END".into());
        }
        self.negotiating = false;
    }

    /// Whether the server agreed to `cap`.
    pub fn has_cap(&self, cap: &str) -> bool {
        self.caps.iter().any(|enabled| enabled == cap)
    }

    /// The answer to a CTCP request, or `None` for one OpenIRC does not
    /// answer.
    fn start_notify(&mut self, effect: &mut Effect) {
        self.notify_started = true;
        if self.config.notify.is_empty() {
            return;
        }
        if self.monitor {
            for chunk in self.config.notify.chunks(20) {
                effect
                    .outgoing
                    .push(format!("MONITOR + {}", chunk.join(",")));
            }
        } else {
            effect
                .outgoing
                .push(format!("ISON {}", self.config.notify.join(" ")));
        }
    }

    /// Record a notify nick as online or not, reporting changes only.
    fn notify_seen(&mut self, nick: &str, address: &str, online: bool, effect: &mut Effect) {
        let key = self.dir.fold(nick);
        let was = self.notify_online.contains(&key);
        if online && !was {
            self.notify_online.push(key);
        } else if !online && was {
            self.notify_online.retain(|item| item != &key);
        } else {
            return;
        }
        effect.push_event(Event::Notify {
            nick: nick.to_string(),
            address: address.to_string(),
            online,
        });
    }

    /// Replace the notify list, telling the server what changed.
    fn set_notify(&mut self, list: &str, effect: &mut Effect) {
        let next: Vec<String> = list
            .split([',', ' '])
            .filter(|nick| !nick.is_empty())
            .map(str::to_string)
            .collect();
        let had =
            |nick: &String, list: &[String]| list.iter().any(|item| self.dir.same(item, nick));
        let added: Vec<String> = next
            .iter()
            .filter(|nick| !had(nick, &self.config.notify))
            .cloned()
            .collect();
        let removed: Vec<String> = self
            .config
            .notify
            .iter()
            .filter(|nick| !had(nick, &next))
            .cloned()
            .collect();
        for nick in &removed {
            let key = self.dir.fold(nick);
            self.notify_online.retain(|item| item != &key);
        }
        self.config.notify = next;
        if !self.notify_started {
            return;
        }
        if self.monitor {
            if !added.is_empty() {
                effect
                    .outgoing
                    .push(format!("MONITOR + {}", added.join(",")));
            }
            if !removed.is_empty() {
                effect
                    .outgoing
                    .push(format!("MONITOR - {}", removed.join(",")));
            }
        } else if !added.is_empty() {
            effect
                .outgoing
                .push(format!("ISON {}", self.config.notify.join(" ")));
        }
    }

    fn ctcp_answer(&self, cmd: &str, arg: &str) -> Option<String> {
        let body = match cmd.to_ascii_uppercase().as_str() {
            "VERSION" => format!("VERSION {NAME} {VERSION}"),
            "PING" if arg.is_empty() => "PING".to_string(),
            "PING" => format!("PING {arg}"),
            "TIME" => format!("TIME {}", crate::clock::stamp()),
            "FINGER" => {
                let finger = if self.config.finger_reply.trim().is_empty() {
                    &self.config.realname
                } else {
                    &self.config.finger_reply
                };
                format!("FINGER {finger}")
            }
            "CLIENTINFO" => "CLIENTINFO ACTION CLIENTINFO DCC FINGER PING TIME VERSION".to_string(),
            _ => return None,
        };
        Some(format!("\u{1}{body}\u{1}"))
    }

    fn on_numeric(&mut self, command: &str, message: &Message, effect: &mut Effect) {
        let code: u16 = command.parse().unwrap_or(0);
        let text = message
            .params
            .iter()
            .skip(1)
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        if code == 1 {
            if let Some(nick) = message.params.first() {
                self.nick = nick.clone();
            }
            self.registered = true;
            effect.push_event(Event::Registered {
                nick: self.nick.clone(),
                text: text.clone(),
            });
            if self.login() == crate::servers::Login::NickServ {
                let account = self.config.sasl_user.trim();
                let identify = if account.is_empty() {
                    format!("IDENTIFY {}", self.config.sasl_password)
                } else {
                    format!("IDENTIFY {account} {}", self.config.sasl_password)
                };
                effect
                    .outgoing
                    .extend(message::privmsg_lines("NickServ", &identify));
                effect.lines.push("* identifying to NickServ".into());
            }
            self.queue_autojoin(effect);
            return;
        }
        // A server without capability negotiation; registration goes on.
        if code == 421
            && message
                .params
                .get(1)
                .is_some_and(|command| command.eq_ignore_ascii_case("CAP"))
        {
            return;
        }
        if (903..=906).contains(&code) && self.uses_sasl() {
            if self.config.ircv3 {
                self.negotiating = true;
                self.end_negotiation(effect);
            } else {
                effect.outgoing.push("CAP END".into());
            }
            if code == 903 {
                effect.lines.push("* SASL login succeeded".into());
            } else if code == 904 || code == 905 {
                effect
                    .lines
                    .push("* SASL login failed; check the account and password".into());
            }
        }
        if (code == 433 || code == 432) && !self.registered && self.nick_retries < 4 {
            self.nick_retries += 1;
            let alt = self.config.alt_nick.trim();
            if self.nick_retries == 1 && !alt.is_empty() && !self.dir.same(alt, &self.nick) {
                self.nick = alt.to_string();
            } else {
                self.nick.push('_');
            }
            effect
                .outgoing
                .push(Message::command("NICK", [self.nick.as_str()]).encode());
            effect
                .lines
                .push(format!("* nick in use, trying {}", self.nick));
            return;
        }
        match code {
            5 => {
                let tokens = support_tokens(&message.params);
                self.dir.apply_support(&tokens);
                if tokens
                    .iter()
                    .any(|token| token == "MONITOR" || token.starts_with("MONITOR="))
                {
                    self.monitor = true;
                }
                if let Some(name) = tokens
                    .iter()
                    .find_map(|token| token.strip_prefix("CASEMAPPING="))
                {
                    effect.events.push(Event::CaseMapping {
                        map: crate::state::CaseMap::parse(name),
                    });
                }
                if let Some(name) = tokens
                    .iter()
                    .find_map(|token| token.strip_prefix("NETWORK="))
                {
                    self.network = Some(name.to_string());
                    effect.events.push(Event::Network {
                        name: name.to_string(),
                    });
                }
                if tokens.is_empty() {
                    effect.push_event(Event::Numeric { code, text });
                } else {
                    effect.lines.push(format!("* server {}", tokens.join(" ")));
                }
            }
            324 => {
                if let Some(channel) = message.params.get(1).cloned() {
                    self.dir.set_modes(&channel, &message.params[2..]);
                    let modes = self.dir.mode_string(&channel).unwrap_or_default();
                    // The shown modes leave out a key, which is a password.
                    let shown = modes.split_whitespace().next().unwrap_or("").to_string();
                    effect.push_event(Event::ChannelModes {
                        channel: channel.clone(),
                        modes,
                    });
                    if !shown.is_empty() && shown != "+" {
                        effect.push_event(Event::ChannelInfo {
                            text: format!("{channel} has modes {shown}"),
                            channel,
                        });
                    }
                }
            }
            331 => {
                if let Some(channel) = message.params.get(1) {
                    self.dir.set_topic(channel, None);
                    effect.push_event(Event::Topic {
                        channel: channel.clone(),
                        topic: None,
                    });
                }
            }
            332 => {
                if let (Some(channel), Some(topic)) = (message.params.get(1), message.params.get(2))
                {
                    self.dir.set_topic(channel, Some(topic.clone()));
                    effect.push_event(Event::Topic {
                        channel: channel.clone(),
                        topic: Some(topic.clone()),
                    });
                }
            }
            333 => {
                if let Some(channel) = message.params.get(1) {
                    let setter = message
                        .params
                        .get(2)
                        .map(String::as_str)
                        .unwrap_or("someone");
                    let when = message
                        .params
                        .get(3)
                        .and_then(|at| at.parse::<i64>().ok())
                        .map(|at| format!(" on {}", long_date(at)))
                        .unwrap_or_default();
                    effect.push_event(Event::ChannelInfo {
                        channel: channel.clone(),
                        text: format!("Topic set by {setter}{when}"),
                    });
                }
            }
            353 => {
                let (channel, names) = if message.params.len() >= 4 {
                    (message.params.get(2), message.params.get(3))
                } else {
                    (message.params.get(1), message.params.get(2))
                };
                if let (Some(channel), Some(names)) = (channel, names) {
                    self.dir.add_names(channel, names);
                }
            }
            366 => {
                if let Some(channel) = message.params.get(1) {
                    let names = self.dir.end_names(channel);
                    let key = channel.to_lowercase();
                    let listed = self.names_wanted.contains(&key);
                    self.names_wanted.retain(|wanted| wanted != &key);
                    effect.push_event(Event::Names {
                        channel: channel.clone(),
                        names,
                        listed,
                    });
                }
            }
            // The end of the message of the day: registration is done and
            // the server's features are known, so start watching.
            376 | 422 if !self.notify_started => {
                self.start_notify(effect);
                effect.push_event(Event::Numeric { code, text });
            }
            730 | 731 => {
                let online = code == 730;
                let list = message.trailing().unwrap_or("");
                for target in list.split(',').filter(|target| !target.is_empty()) {
                    let (nick, address) = target.split_once('!').unwrap_or((target, ""));
                    self.notify_seen(nick, address, online, effect);
                }
            }
            734 => effect
                .lines
                .push("* the server's notify list is full".into()),
            303 if self.notify_started && !self.monitor => {
                let online: Vec<String> = message
                    .trailing()
                    .unwrap_or("")
                    .split_whitespace()
                    .map(|nick| self.dir.fold(nick))
                    .collect();
                let watched = self.config.notify.clone();
                for nick in watched {
                    let here = online.contains(&self.dir.fold(&nick));
                    self.notify_seen(&nick, "", here, effect);
                }
            }
            329 => {
                if let (Some(channel), Some(at)) = (
                    message.params.get(1),
                    message.params.get(2).and_then(|at| at.parse::<i64>().ok()),
                ) {
                    effect.push_event(Event::ChannelInfo {
                        channel: channel.clone(),
                        text: format!("{channel} was created on {}", long_date(at)),
                    });
                }
            }
            221 => {
                let modes = message.params.get(1).cloned().unwrap_or_default();
                self.umodes = modes.trim_start_matches('+').to_string();
                effect.events.push(Event::UserModes {
                    modes: format!("+{}", self.umodes),
                });
            }
            _ => effect.push_event(Event::Numeric { code, text }),
        }
    }

    fn queue_autojoin(&mut self, effect: &mut Effect) {
        if self.joined_auto {
            return;
        }
        self.joined_auto = true;
        for channel in &self.config.autojoin {
            effect
                .outgoing
                .push(Message::command("JOIN", [channel.as_str()]).encode());
        }
    }

    fn on_command(&mut self, command: &str, effect: &mut Effect) {
        let (name, rest) = match command.split_once(char::is_whitespace) {
            Some((name, rest)) => (name, rest.trim()),
            None => (command, ""),
        };
        match name.to_ascii_lowercase().as_str() {
            "help" => {
                effect.lines.extend(help_lines());
            }
            "join" | "j" => match first_token(rest) {
                Some((channels, keys)) => {
                    // `/join a,#b key` joins #a and #b; a name without a
                    // channel prefix gets `#`.
                    let channels: Vec<String> = channels
                        .split(',')
                        .filter(|name| !name.is_empty())
                        .map(|name| {
                            if self.dir.is_channel(name) {
                                name.to_string()
                            } else {
                                format!("#{name}")
                            }
                        })
                        .collect();
                    let Some(first) = channels.first().cloned() else {
                        effect
                            .lines
                            .push("* usage: /join #channel [key]".to_string());
                        return;
                    };
                    self.target = Some(first);
                    self.picked = true;
                    let list = channels.join(",");
                    let keys = keys.split_whitespace().next().unwrap_or("");
                    let line = if keys.is_empty() {
                        Message::command("JOIN", [list])
                    } else {
                        Message::command("JOIN", [list, keys.to_string()])
                    };
                    effect.outgoing.push(line.encode());
                }
                None => effect
                    .lines
                    .push("* usage: /join #channel [key]".to_string()),
            },
            "partall" => {
                let channels: Vec<String> = self
                    .dir
                    .channels()
                    .into_iter()
                    .map(str::to_string)
                    .collect();
                if channels.is_empty() {
                    effect.lines.push("* you are in no channels".into());
                    return;
                }
                let mut params = vec![channels.join(",")];
                if !rest.is_empty() {
                    params.push(rest.to_string());
                }
                effect
                    .outgoing
                    .push(Message::command("PART", params).encode());
            }
            "describe" => {
                let Some((target, text)) = first_token(rest).filter(|(_, text)| !text.is_empty())
                else {
                    effect
                        .lines
                        .push("* usage: /describe <target> <action>".into());
                    return;
                };
                self.write_to(&target, text, true, effect);
            }
            "say" => self.send_chat(rest, false, effect),
            "amsg" => {
                if rest.is_empty() {
                    effect.lines.push("* usage: /amsg <text>".into());
                    return;
                }
                let channels: Vec<String> = self
                    .dir
                    .channels()
                    .into_iter()
                    .map(str::to_string)
                    .collect();
                if channels.is_empty() {
                    effect.lines.push("* you are in no channels".into());
                    return;
                }
                for channel in channels {
                    self.write_to(&channel, rest, false, effect);
                }
            }
            "omsg" | "onotice" => {
                // To the channel's operators only, as `@#channel`.
                let (channel, text) = match first_token(rest) {
                    Some((channel, text)) if self.dir.is_channel(&channel) => (channel, text),
                    _ => match self
                        .target
                        .clone()
                        .filter(|target| self.dir.is_channel(target))
                    {
                        Some(channel) => (channel, rest),
                        None => {
                            effect
                                .lines
                                .push(format!("* usage: /{name} [#channel] <text>"));
                            return;
                        }
                    },
                };
                if text.is_empty() {
                    effect
                        .lines
                        .push(format!("* usage: /{name} [#channel] <text>"));
                    return;
                }
                let target = format!("@{channel}");
                if name.eq_ignore_ascii_case("omsg") {
                    effect
                        .outgoing
                        .extend(message::privmsg_lines(&target, text));
                } else {
                    effect.outgoing.extend(message::notice_lines(&target, text));
                }
                effect.lines.push(format!("-> {target} {text}"));
            }
            "ctcpreply" => {
                let mut parts = rest.splitn(3, char::is_whitespace);
                let (Some(nick), Some(command)) = (parts.next(), parts.next()) else {
                    effect
                        .lines
                        .push("* usage: /ctcpreply <nick> <command> [text]".into());
                    return;
                };
                let text = parts.next().unwrap_or("").trim();
                let body = if text.is_empty() {
                    command.to_ascii_uppercase()
                } else {
                    format!("{} {text}", command.to_ascii_uppercase())
                };
                effect
                    .outgoing
                    .extend(message::notice_lines(nick, &format!("\u{1}{body}\u{1}")));
                let shown = crate::connection::mask_service_text(nick, &body).unwrap_or(body);
                effect.lines.push(format!("* CTCP {shown} reply to {nick}"));
            }
            "ping" => match first_token(rest) {
                Some((nick, _)) => {
                    let payload = format!("\u{1}PING {}\u{1}", unix_ms());
                    effect
                        .outgoing
                        .extend(message::privmsg_lines(&nick, &payload));
                    effect.lines.push(format!("* CTCP PING to {nick}"));
                }
                None => effect.lines.push("* usage: /ping <nick>".into()),
            },
            "chanserv" | "cs" | "memoserv" | "ms" | "hostserv" | "hs" | "botserv" | "bs"
            | "operserv" | "os" => {
                let service = match name.to_ascii_lowercase().as_str() {
                    "chanserv" | "cs" => "ChanServ",
                    "memoserv" | "ms" => "MemoServ",
                    "hostserv" | "hs" => "HostServ",
                    "botserv" | "bs" => "BotServ",
                    _ => "OperServ",
                };
                if rest.is_empty() {
                    effect.lines.push(format!("* usage: /{name} <command>"));
                    return;
                }
                effect
                    .outgoing
                    .extend(message::privmsg_lines(service, rest));
                // The transcript and the log show a password as ****.
                let shown = crate::connection::mask_service_text(service, rest)
                    .unwrap_or_else(|| rest.to_string());
                effect.lines.push(format!("-> *{service}* {shown}"));
            }
            "part" => {
                let (channel, reason) = match first_token(rest) {
                    Some((token, rest)) if self.dir.is_channel(&token) => (token, rest),
                    Some(_) => {
                        effect
                            .lines
                            .push("* usage: /part [#channel] [reason]".into());
                        return;
                    }
                    None => match self.target.clone() {
                        Some(channel) => (channel, ""),
                        None => {
                            effect.lines.push("* no channel selected".into());
                            return;
                        }
                    },
                };
                if reason.is_empty() {
                    effect
                        .outgoing
                        .push(Message::command("PART", [channel]).encode());
                } else {
                    effect
                        .outgoing
                        .push(Message::command("PART", [channel, reason.to_string()]).encode());
                }
            }
            "query" | "msg" => {
                let Some((target, text)) = first_token(rest) else {
                    effect.lines.push("* usage: /msg <target> <text>".into());
                    return;
                };
                if text.is_empty() {
                    if name.eq_ignore_ascii_case("query") && !target.is_empty() {
                        self.target = Some(target);
                        self.picked = true;
                        effect.lines.push(format!(
                            "* talking to {}",
                            self.target.as_deref().unwrap_or("")
                        ));
                    } else {
                        effect.lines.push("* usage: /msg <target> <text>".into());
                    }
                    return;
                }
                self.write_to(&target, text, false, effect);
            }
            "notice" => {
                let Some((target, text)) = first_token(rest) else {
                    effect.lines.push("* usage: /notice <target> <text>".into());
                    return;
                };
                if text.is_empty() {
                    effect.lines.push("* usage: /notice <target> <text>".into());
                    return;
                }
                effect.outgoing.extend(message::notice_lines(&target, text));
                let shown = crate::connection::mask_service_text(&target, text)
                    .unwrap_or_else(|| text.to_string());
                effect.lines.push(format!("-{target}- {shown}"));
            }
            "me" => self.send_chat(rest, true, effect),
            "names" => {
                let channel = match first_token(rest) {
                    Some((channel, "")) if self.dir.is_channel(&channel) => Some(channel),
                    Some(_) => {
                        effect.lines.push("* usage: /names [#channel]".into());
                        return;
                    }
                    None => self.current_channel(),
                };
                let Some(channel) = channel else {
                    effect.lines.push("* no channel selected".into());
                    return;
                };
                self.names_wanted.push(channel.to_lowercase());
                effect
                    .outgoing
                    .push(Message::command("NAMES", [channel]).encode());
            }
            "topic" => self.command_topic(rest, effect),
            "whois" => match first_token(rest) {
                Some((nick, "")) => {
                    effect
                        .outgoing
                        .push(Message::command("WHOIS", [nick.clone(), nick]).encode());
                }
                _ => effect.lines.push("* usage: /whois <nick>".into()),
            },
            "mode" => {
                let tokens: Vec<String> = rest.split_whitespace().map(str::to_string).collect();
                if tokens.is_empty() {
                    effect
                        .lines
                        .push("* usage: /mode <#channel> <flags> [nick]".into());
                    return;
                }
                effect
                    .outgoing
                    .push(Message::command("MODE", tokens).encode());
            }
            // Without a nick, /admin asks the server who runs it.
            "admin" if rest.is_empty() || rest.contains('.') => {
                self.pass_through(name, rest, effect);
            }
            "op" | "deop" | "voice" | "devoice" | "halfop" | "dehalfop" | "owner" | "deowner"
            | "admin" | "deadmin" | "ban" | "unban" | "except" | "unexcept" => {
                self.mode_nick(name, rest, effect);
            }
            "ignore" => match crate::rules::parse_ignore(rest, crate::rules::unix_now()) {
                Ok(rule) => {
                    let mask = rule.full_mask();
                    self.ignored.retain(|item| item.full_mask() != mask);
                    effect
                        .lines
                        .push(format!("* ignoring {mask} ({})", ignore_kinds(&rule.types)));
                    self.ignored.push(rule);
                }
                Err(err) => effect.lines.push(format!("* {err}")),
            },
            "unignore" => match first_token(rest) {
                Some((mask, "")) => {
                    let full = crate::rules::full_mask(&mask);
                    let before = self.ignored.len();
                    self.ignored
                        .retain(|item| !item.full_mask().eq_ignore_ascii_case(&full));
                    if self.ignored.len() < before {
                        effect.lines.push(format!("* no longer ignoring {full}"));
                    } else {
                        effect.lines.push(format!("* {full} was not ignored"));
                    }
                }
                _ => effect
                    .lines
                    .push("* usage: /unignore <nick or mask>".into()),
            },
            "who" => {
                let target = match first_token(rest) {
                    Some((target, _)) => target,
                    None => match self.current_channel() {
                        Some(channel) => channel,
                        None => {
                            effect.lines.push("* usage: /who [#channel|nick]".into());
                            return;
                        }
                    },
                };
                effect.outgoing.push(crate::services::who_line(&target));
            }
            "identify" => match first_token(rest) {
                Some((password, extra)) => {
                    let password = if extra.is_empty() {
                        password
                    } else {
                        format!("{password} {extra}")
                    };
                    effect
                        .outgoing
                        .push(crate::services::identify_line(&password));
                }
                None => effect.lines.push("* usage: /identify <password>".into()),
            },
            "nickserv" | "ns" => {
                if rest.is_empty() {
                    effect.lines.push("* usage: /nickserv <command>".into());
                    return;
                }
                effect.outgoing.push(crate::services::nickserv_line(rest));
            }
            "kick" => self.command_kick(rest, effect),
            "win" | "window" => self.command_window(rest, effect),
            "nick" => match first_token(rest) {
                Some((nick, leftover)) if leftover.is_empty() && valid_nick(&nick) => {
                    effect
                        .outgoing
                        .push(Message::command("NICK", [nick]).encode());
                }
                _ => effect.lines.push("* usage: /nick <name>".into()),
            },
            // `/via <target> <line>`: run the line with that target for its
            // default, in one step, so nothing the server says in between
            // (our own join, say) can send it elsewhere.
            "via" => match unwrap_via(rest) {
                Ok(via) => self.run_via(&via, effect),
                Err(usage) => effect.lines.push(usage.into()),
            },
            "target" => match first_token(rest) {
                // `/target -q` alone chooses, quietly, the target there is:
                // under `/via`, the one the line came for.
                Some((flag, "")) if flag == "-q" => self.picked = true,
                Some((flag, quiet_target)) if flag == "-q" && !quiet_target.is_empty() => {
                    self.target = Some(quiet_target.to_string());
                    self.picked = true;
                }
                Some((target, "")) => {
                    self.target = Some(target.clone());
                    self.picked = true;
                    effect.lines.push(format!("* talking to {target}"));
                }
                _ => effect.lines.push("* usage: /target <#channel|nick>".into()),
            },
            "flood" => {
                let parts: Vec<&str> = rest.split_whitespace().collect();
                let parsed = if parts.len() == 3 || parts.len() == 4 {
                    Some((
                        parts[0].parse::<u32>().ok(),
                        parts[1].parse::<u32>().ok(),
                        parts[2].parse::<u64>().ok(),
                        match parts.get(3) {
                            Some(ctcp) => ctcp.parse::<u32>().ok(),
                            None => Some(self.flood.limits().ctcp),
                        },
                    ))
                } else {
                    None
                };
                match parsed {
                    Some((Some(messages), Some(joins), Some(seconds), Some(ctcp)))
                        if seconds > 0 =>
                    {
                        self.flood.set_limits(Limits {
                            messages,
                            joins,
                            window_ms: seconds.saturating_mul(1_000),
                            ctcp,
                        });
                        effect.lines.push(format!(
                            "* flood limits {messages} messages, {joins} joins, {ctcp} CTCP answers, every {seconds}s"
                        ));
                    }
                    _ => effect
                        .lines
                        .push("* usage: /flood <messages> <joins> <seconds> [ctcp]".into()),
                }
            }
            "away" => {
                let reason = crate::away::display_reason(rest);
                effect.outgoing.push(crate::away::away_line(rest));
                effect.push_event(Event::Away {
                    reason: Some(reason),
                });
            }
            "back" => {
                if !rest.is_empty() {
                    effect.lines.push("* usage: /back".into());
                    return;
                }
                effect.outgoing.push(crate::away::back_line());
                effect.push_event(Event::Away { reason: None });
            }
            "invite" => self.command_invite(rest, effect),
            "knock" => self.command_knock(rest, effect),
            "hop" => self.command_hop(rest, effect),
            "ame" => {
                if rest.is_empty() {
                    effect.lines.push("* usage: /ame <action>".into());
                    return;
                }
                let channels: Vec<String> = self
                    .dir
                    .channels()
                    .into_iter()
                    .map(str::to_string)
                    .collect();
                if channels.is_empty() {
                    effect.lines.push("* no channel selected".into());
                    return;
                }
                for channel in channels {
                    self.write_to(&channel, rest, true, effect);
                }
            }
            "ctcp" => {
                let Some((nick, command)) = first_token(rest) else {
                    effect.lines.push("* usage: /ctcp <nick> <command>".into());
                    return;
                };
                if command.is_empty() {
                    effect.lines.push("* usage: /ctcp <nick> <command>".into());
                    return;
                }
                // A bare PING carries the time it was sent, so the reply
                // shows the round trip.
                let payload = if command.eq_ignore_ascii_case("ping") {
                    format!("\u{1}PING {}\u{1}", unix_ms())
                } else {
                    format!("\u{1}{command}\u{1}")
                };
                effect
                    .outgoing
                    .extend(message::privmsg_lines(&nick, &payload));
                let shown = crate::connection::mask_service_text(&nick, command)
                    .unwrap_or_else(|| command.to_string());
                effect.lines.push(format!("* CTCP {shown} to {nick}"));
            }
            "echo" => {
                if rest.is_empty() {
                    effect.lines.push("* usage: /echo <text>".into());
                    return;
                }
                effect.push_event(Event::Echo {
                    text: rest.to_string(),
                });
            }
            "clear" | "cls" => {
                if !rest.is_empty() {
                    effect.lines.push("* usage: /clear".into());
                    return;
                }
                effect.push_event(Event::Clear);
            }
            "banlist" | "bans" => {
                let channel = if rest.is_empty() {
                    match self.current_channel() {
                        Some(channel) => channel,
                        None => {
                            effect.lines.push("* usage: /banlist [#channel]".into());
                            return;
                        }
                    }
                } else {
                    match first_token(rest) {
                        Some((channel, "")) if self.dir.is_channel(&channel) => channel,
                        _ => {
                            effect.lines.push("* usage: /banlist [#channel]".into());
                            return;
                        }
                    }
                };
                effect
                    .outgoing
                    .push(Message::command("MODE", [channel, "+b".to_string()]).encode());
            }
            "whowas" => match first_token(rest) {
                Some((nick, _)) => {
                    effect
                        .outgoing
                        .push(Message::command("WHOWAS", [nick]).encode());
                }
                None => effect.lines.push("* usage: /whowas <nick>".into()),
            },
            "umode" => match first_token(rest) {
                Some((flags, _)) => {
                    effect
                        .outgoing
                        .push(Message::command("MODE", [self.nick.clone(), flags]).encode());
                }
                None => effect.lines.push("* usage: /umode <flags>".into()),
            },
            "key" => self.channel_mode(rest, "k", effect),
            "limit" => self.channel_mode(rest, "l", effect),
            "silence" => match first_token(rest) {
                Some((mask, _)) => {
                    effect
                        .outgoing
                        .push(Message::command("SILENCE", [mask]).encode());
                }
                None => effect.lines.push("* usage: /silence <+nick|-nick>".into()),
            },
            "quit" => {
                let reason = if rest.is_empty() {
                    if self.config.quit_message.trim().is_empty() {
                        format!("{NAME} {VERSION}")
                    } else {
                        self.config.quit_message.clone()
                    }
                } else {
                    rest.to_string()
                };
                effect
                    .outgoing
                    .push(Message::command("QUIT", [reason]).encode());
                effect.quit = true;
                effect.lines.push("* disconnected".into());
            }
            // Sent by the window, not typed: the whole notify list, and the
            // once-a-minute check on servers without MONITOR.
            "notifylist" => self.set_notify(rest, effect),
            "notifypoll" => {
                if self.notify_started && !self.monitor && !self.config.notify.is_empty() {
                    effect
                        .outgoing
                        .push(format!("ISON {}", self.config.notify.join(" ")));
                }
            }
            "typing" => {
                // `/typing <target> <active|paused|done>`, sent by the window
                // while you type; dropped when the server has no tags.
                // `/typing <state>` alone is for the target there is: under
                // `/via`, the one the window meant, followed through renames.
                let (target, state) = match first_token(rest) {
                    Some((state, "")) => (self.target.clone(), state),
                    Some((target, state)) => (Some(target), state.to_string()),
                    None => (None, String::new()),
                };
                if let Some(target) = target
                    && matches!(state.as_str(), "active" | "paused" | "done")
                    && self.has_cap("message-tags")
                {
                    effect
                        .outgoing
                        .push(format!("@+typing={state} TAGMSG {target}"));
                }
            }
            "raw" | "quote" => {
                if rest.is_empty() {
                    effect.lines.push("* usage: /raw <IRC line>".into());
                    return;
                }
                match Message::parse(rest) {
                    Ok(message) => effect.outgoing.push(message.encode()),
                    Err(err) => effect.lines.push(format!("* {err}")),
                }
            }
            // Anything else is a server command, such as /lusers, /motd,
            // /stats, /oper, or /userhost: it goes to the server as typed.
            _ => self.pass_through(name, rest, effect),
        }
    }

    fn command_invite(&mut self, rest: &str, effect: &mut Effect) {
        let Some((nick, rest)) = first_token(rest) else {
            effect
                .lines
                .push("* usage: /invite <nick> [#channel]".into());
            return;
        };
        let channel = if rest.is_empty() {
            match self.current_channel() {
                Some(channel) => channel,
                None => {
                    effect
                        .lines
                        .push("* usage: /invite <nick> [#channel]".into());
                    return;
                }
            }
        } else {
            match first_token(rest) {
                Some((channel, "")) if self.dir.is_channel(&channel) => channel,
                _ => {
                    effect
                        .lines
                        .push("* usage: /invite <nick> [#channel]".into());
                    return;
                }
            }
        };
        effect
            .outgoing
            .push(Message::command("INVITE", [nick, channel]).encode());
    }

    fn command_knock(&mut self, rest: &str, effect: &mut Effect) {
        let Some((channel, message)) = first_token(rest) else {
            effect
                .lines
                .push("* usage: /knock <#channel> [message]".into());
            return;
        };
        if !self.dir.is_channel(&channel) {
            effect
                .lines
                .push("* usage: /knock <#channel> [message]".into());
            return;
        }
        if message.is_empty() {
            effect
                .outgoing
                .push(Message::command("KNOCK", [channel]).encode());
        } else {
            effect
                .outgoing
                .push(Message::command("KNOCK", [channel, message.to_string()]).encode());
        }
    }

    fn command_hop(&mut self, rest: &str, effect: &mut Effect) {
        let channel = if rest.is_empty() {
            match self.current_channel() {
                Some(channel) => channel,
                None => {
                    effect.lines.push("* usage: /hop [#channel]".into());
                    return;
                }
            }
        } else {
            match first_token(rest) {
                Some((channel, "")) if self.dir.is_channel(&channel) => channel,
                _ => {
                    effect.lines.push("* usage: /hop [#channel]".into());
                    return;
                }
            }
        };
        effect
            .outgoing
            .push(Message::command("PART", [channel.clone(), "hopping".to_string()]).encode());
        effect
            .outgoing
            .push(Message::command("JOIN", [channel.clone()]).encode());
        effect.lines.push(format!("* hopping {channel}"));
    }

    fn send_chat(&mut self, text: &str, action: bool, effect: &mut Effect) {
        let Some(target) = self.target.clone() else {
            effect
                .lines
                .push("* no channel selected; use /join #channel".into());
            return;
        };
        self.write_to(&target, text, action, effect);
    }

    /// Send `/name rest` to the server as `NAME rest`.
    fn pass_through(&self, name: &str, rest: &str, effect: &mut Effect) {
        let line = if rest.is_empty() {
            name.to_ascii_uppercase()
        } else {
            format!("{} {rest}", name.to_ascii_uppercase())
        };
        match Message::parse(&line) {
            Ok(_) if name.chars().all(|ch| ch.is_ascii_alphanumeric()) => {
                effect.outgoing.push(line)
            }
            _ => effect.lines.push(format!("* /{name} is not a command")),
        }
    }

    fn write_to(&self, target: &str, text: &str, action: bool, effect: &mut Effect) {
        if text.is_empty() {
            return;
        }
        let wire = if action {
            format!("\u{1}ACTION {text}\u{1}")
        } else {
            text.to_string()
        };
        effect
            .outgoing
            .extend(message::privmsg_lines(target, &wire));
        // The transcript and the log show a services password as ****.
        let shown =
            crate::connection::mask_service_text(target, text).unwrap_or_else(|| text.to_string());
        effect.push_event(Event::Sent {
            target: target.to_string(),
            from: self.nick.clone(),
            text: shown,
            action,
        });
    }

    /// Whether an ignore rule silences this kind of message from its
    /// sender (see `rules::IGNORE_TYPES`).
    fn is_ignored(&self, message: &Message, kind: char) -> bool {
        let Some(prefix) = message.prefix.as_ref() else {
            return false;
        };
        let source = match prefix {
            Prefix::User { nick, user, host } => format!(
                "{nick}!{}@{}",
                user.as_deref().unwrap_or("*"),
                host.as_deref().unwrap_or("*")
            ),
            Prefix::Server(name) => name.clone(),
        };
        let now = crate::rules::unix_now();
        let network = self.network.as_deref().unwrap_or("");
        self.ignored
            .iter()
            .any(|rule| rule.applies(&source, kind, network, now))
    }

    fn mode_nick(&mut self, name: &str, rest: &str, effect: &mut Effect) {
        let Some((first, rest)) = first_token(rest) else {
            effect
                .lines
                .push(format!("* usage: /{name} [#channel] <nick>"));
            return;
        };
        let (channel, nick) = if self.dir.is_channel(&first) {
            let Some((nick, _)) = first_token(rest) else {
                effect
                    .lines
                    .push(format!("* usage: /{name} [#channel] <nick>"));
                return;
            };
            (first, nick)
        } else {
            let Some(channel) = self.current_channel() else {
                effect.lines.push("* no channel selected".into());
                return;
            };
            (channel, first)
        };
        let (flag, mask) = match name {
            "op" => ("+o", nick),
            "deop" => ("-o", nick),
            "voice" => ("+v", nick),
            "devoice" => ("-v", nick),
            "ban" => ("+b", format!("{nick}!*@*")),
            "unban" => ("-b", format!("{nick}!*@*")),
            "except" => ("+e", format!("{nick}!*@*")),
            "unexcept" => ("-e", format!("{nick}!*@*")),
            "halfop" => ("+h", nick),
            "dehalfop" => ("-h", nick),
            "owner" => ("+q", nick),
            "deowner" => ("-q", nick),
            "admin" => ("+a", nick),
            "deadmin" => ("-a", nick),
            _ => return,
        };
        effect
            .outgoing
            .push(Message::command("MODE", [channel, flag.to_string(), mask]).encode());
    }

    fn channel_mode(&mut self, rest: &str, flag: &str, effect: &mut Effect) {
        let Some(channel) = self.current_channel() else {
            effect.lines.push("* no channel selected".into());
            return;
        };
        if rest.is_empty() {
            effect
                .outgoing
                .push(Message::command("MODE", [channel, format!("-{flag}")]).encode());
        } else {
            effect.outgoing.push(
                Message::command("MODE", [channel, format!("+{flag}"), rest.to_string()]).encode(),
            );
        }
    }

    fn current_channel(&self) -> Option<String> {
        self.target
            .clone()
            .filter(|target| self.dir.is_channel(target))
    }

    fn select_after_leave(&mut self, channel: &str) {
        if self
            .target
            .as_ref()
            .is_some_and(|target| self.dir.same(target, channel))
        {
            self.target = self.dir.fallback().map(str::to_string);
        }
    }

    fn command_topic(&mut self, rest: &str, effect: &mut Effect) {
        let (channel, text) = match first_token(rest) {
            Some((token, remainder)) if self.dir.is_channel(&token) => (Some(token), remainder),
            Some(_) => (self.current_channel(), rest),
            None => (self.current_channel(), ""),
        };
        let Some(channel) = channel else {
            effect.lines.push("* no channel selected".into());
            return;
        };
        if text.is_empty() {
            match self.dir.topic(&channel) {
                Some(Some(topic)) => effect.lines.push(format!("* {channel} topic: {topic}")),
                Some(None) => effect.lines.push(format!("* {channel} has no topic")),
                None => effect
                    .outgoing
                    .push(Message::command("TOPIC", [channel]).encode()),
            }
        } else {
            effect
                .outgoing
                .push(Message::command("TOPIC", [channel, text.to_string()]).encode());
        }
    }

    fn command_kick(&mut self, rest: &str, effect: &mut Effect) {
        let Some((first, rest)) = first_token(rest) else {
            effect
                .lines
                .push("* usage: /kick [#channel] <nick> [reason]".into());
            return;
        };
        let (channel, nick, reason) = if self.dir.is_channel(&first) {
            let Some((nick, reason)) = first_token(rest) else {
                effect
                    .lines
                    .push("* usage: /kick [#channel] <nick> [reason]".into());
                return;
            };
            (first, nick, reason)
        } else {
            let Some(channel) = self.current_channel() else {
                effect.lines.push("* no channel selected".into());
                return;
            };
            (channel, first, rest)
        };
        if reason.is_empty() {
            effect
                .outgoing
                .push(Message::command("KICK", [channel, nick]).encode());
        } else {
            effect
                .outgoing
                .push(Message::command("KICK", [channel, nick, reason.to_string()]).encode());
        }
    }

    /// Run the line of a `/via` for its target (see `unwrap_via`).
    fn run_via(&mut self, via: &Via, effect: &mut Effect) {
        // A choice is kept for its own run only: once another run speaks,
        // it is over, whatever number it claimed.
        if via.run.is_some() && self.run_choice.as_ref().map(|choice| choice.run) != via.run {
            self.run_choice = None;
        }
        // Earlier in the same run, a line chose a target on purpose
        // (`/query alice | /say …`): it leads the rest, except a line with a
        // `/via` of its own.
        let chosen = match &self.run_choice {
            Some(choice) if via.run == Some(choice.run) && !via.nested => {
                Some((choice.target.clone(), choice.left))
            }
            _ => None,
        };
        // The default target the line runs with: `None` keeps the one here,
        // and `Some(None)` takes it away while the line runs.
        let target = match (chosen, via.target) {
            (Some((chosen, false)), _) => Some(Some(chosen)),
            (Some((chosen, true)), _) => {
                let note = format!("* a line for {chosen} was not sent: {chosen} left meanwhile");
                if !self.may_run_without_target(via.line, note, effect) {
                    return;
                }
                Some(None)
            }
            // The server side's own target, as for a line with no `/via`.
            (None, "*") => None,
            (None, "-") => {
                // The line itself is never shown: it may hold a password.
                effect
                    .lines
                    .push("* a line was not sent: nothing was chosen to send it to".into());
                return;
            }
            (None, target) if via.nested => Some(Some(target.to_string())),
            (None, target) => match self.followed(target, via.seen) {
                Ok(target) => Some(Some(target)),
                Err(refusal) => {
                    // The line itself is never shown: it may hold a password.
                    let why = match refusal {
                        Refusal::Forgotten => "nicks changed too often meanwhile".to_string(),
                        Refusal::Gone => format!("{target} left meanwhile"),
                    };
                    let note = format!("* a line for {target} was not sent: {why}");
                    if !self.may_run_without_target(via.line, note, effect) {
                        return;
                    }
                    Some(None)
                }
            },
        };
        let outer = target.map(|target| std::mem::replace(&mut self.target, target));
        self.picked = false;
        // The line is no `/via` itself: `unwrap_via` unwrapped them all, so
        // this goes one level deep at most.
        let inner = self.on_user_line(via.line);
        effect.absorb(inner);
        // A line that chose a target on purpose keeps it, even the same one,
        // and leads the rest of its run; otherwise the target is as it was.
        if self.picked {
            if let (Some(run), Some(chosen)) = (via.run, self.target.clone()) {
                self.run_choice = Some(RunChoice {
                    run,
                    target: chosen,
                    left: false,
                });
            }
        } else if let Some(outer) = outer {
            self.target = outer;
        }
    }

    /// Whether `line`, whose target is not known now (`note` says why,
    /// without the line, which may hold a password), may run with no
    /// default target. Words, a window switch and a typing notice need the
    /// target, so they go nowhere and the note is shown. A refused window
    /// switch also leaves no target behind: where the window means its
    /// words to go is not known until it says. Any other line runs: a
    /// target it names itself still works, and what would have gone to the
    /// window's target gets the usual "no channel selected".
    fn may_run_without_target(&mut self, line: &str, note: String, effect: &mut Effect) -> bool {
        let switch = is_window_switch(line);
        if !(switch || crate::ui::input::is_speech(line) || is_bare_typing(line)) {
            return true;
        }
        effect.lines.push(note);
        if switch {
            self.target = None;
        }
        false
    }

    /// `target` as it is now, for a line written when the window side had
    /// seen `seen` nick changes and quits: renamed through every change
    /// since, in order. Refused when changes it needs were forgotten, or
    /// when the one it is for quit since: whoever holds that nick now is
    /// someone else, and so is anyone who takes it later. A channel, or a
    /// line written without a count, stays as it is.
    fn followed(&self, target: &str, seen: Option<u64>) -> Result<String, Refusal> {
        let mut target = target.to_string();
        let Some(seen) = seen.filter(|seen| *seen < self.renames) else {
            return Ok(target);
        };
        if self.dir.is_channel(&target) {
            return Ok(target);
        }
        if self
            .rename_log
            .front()
            .is_none_or(|(count, _, _)| *count > seen + 1)
        {
            return Err(Refusal::Forgotten);
        }
        for (_, old, new) in self.rename_log.iter().filter(|(count, _, _)| *count > seen) {
            if self.dir.same(&target, old) {
                match new {
                    Some(new) => target = new.clone(),
                    None => return Err(Refusal::Gone),
                }
            }
        }
        Ok(target)
    }

    /// Count a nick change from `old` to `new`, or, with no `new`, someone
    /// leaving, and remember it for lines written before it.
    fn note_rename(&mut self, old: &str, new: Option<&str>) {
        self.renames += 1;
        self.rename_log
            .push_back((self.renames, old.to_string(), new.map(str::to_string)));
        if self.rename_log.len() > RENAMES_KEPT {
            self.rename_log.pop_front();
        }
    }

    fn command_window(&mut self, rest: &str, effect: &mut Effect) {
        if let Some((target, leftover)) = first_token(rest) {
            if !leftover.is_empty() {
                effect.lines.push("* usage: /win [<channel|nick>]".into());
                return;
            }
            self.target = Some(target.clone());
            self.picked = true;
            effect.lines.push(format!("* talking to {target}"));
            return;
        }
        let mut lines = vec!["* windows".to_string()];
        for channel in self.dir.channels() {
            let mark = if self
                .target
                .as_ref()
                .is_some_and(|target| self.dir.same(target, channel))
            {
                "*"
            } else {
                " "
            };
            let count = self
                .dir
                .names(channel)
                .map(|names| names.len())
                .unwrap_or(0);
            lines.push(format!("{mark} {channel} ({count})"));
        }
        if let Some(target) = &self.target
            && !self.dir.is_channel(target)
        {
            lines.push(format!("* {target}"));
        }
        if lines.len() == 1 {
            lines.push("* (none)".into());
        }
        effect.lines.extend(lines);
    }
}

fn ignore_kinds(types: &str) -> String {
    crate::rules::IGNORE_TYPES
        .iter()
        .filter(|(letter, _)| types.contains(*letter))
        .map(|(_, label)| label.to_lowercase())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A SASL answer as AUTHENTICATE lines: 400 bytes each, and when the last
/// one is exactly 400 bytes long, a lone `+` says the answer is complete.
fn authenticate_lines(encoded: &str) -> Vec<String> {
    let mut lines: Vec<String> = encoded
        .as_bytes()
        .chunks(400)
        .map(|chunk| format!("AUTHENTICATE {}", String::from_utf8_lossy(chunk)))
        .collect();
    if encoded.is_empty() || encoded.len().is_multiple_of(400) {
        lines.push("AUTHENTICATE +".into());
    }
    lines
}

fn sasl_plain(nick: &str, password: &str) -> String {
    let mut raw = Vec::new();
    raw.extend(nick.as_bytes());
    raw.push(0);
    raw.extend(nick.as_bytes());
    raw.push(0);
    raw.extend(password.as_bytes());
    crate::socks::base64(&raw)
}

fn who_name(message: &Message) -> String {
    message
        .prefix
        .as_ref()
        .map(|prefix| prefix.name().to_string())
        .unwrap_or_else(|| "*".into())
}

fn is_numeric(command: &str) -> bool {
    command.len() == 3 && command.bytes().all(|byte| byte.is_ascii_digit())
}

fn support_tokens(params: &[String]) -> Vec<String> {
    if params.len() < 3 {
        return Vec::new();
    }
    params[1..params.len() - 1].to_vec()
}

/// The STS policy in a capability value such as `port=6697,duration=86400`.
fn sts_event(value: &str) -> Event {
    let mut port = None;
    let mut duration = None;
    for item in value.split(',') {
        match item.split_once('=') {
            Some(("port", number)) => port = number.parse().ok(),
            Some(("duration", number)) => duration = number.parse().ok(),
            _ => {}
        }
    }
    Event::Sts { port, duration }
}

/// A date for channel lines, such as `2026-10-13 22:48` (local time).
fn long_date(at: i64) -> String {
    let time = crate::clock::from_unix(at);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        time.year, time.month, time.day, time.hour, time.minute
    )
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn ctcp_body(text: &str) -> Option<&str> {
    let inner = text.strip_prefix('\u{1}')?.strip_suffix('\u{1}')?;
    if inner.is_empty() { None } else { Some(inner) }
}

/// A `/via` line taken apart.
#[derive(Debug, PartialEq, Eq)]
pub struct Via<'a> {
    /// The run it belongs to (`=<number>`), for lines that come together.
    pub run: Option<u64>,
    /// How many nick changes and quits the window side had seen when it
    /// wrote the line (`=<run>:<count>`), so a nick that changed since is
    /// followed, and one whose holder left is not.
    pub seen: Option<u64>,
    /// The channel or nick; or, written by the window side only, `*` (the
    /// server side's own target) or `-` (only what the run chose).
    pub target: &'a str,
    /// Whether `target` came from a `/via` inside the first one: a line's
    /// own choice, which the run's earlier choice does not replace.
    pub nested: bool,
    /// The line it runs, with no `/via` left in it.
    pub line: &'a str,
}

/// The arguments of `line` when it is a `/via` command, read the way any
/// command is: the name in any case, then any whitespace.
pub fn via_command(line: &str) -> Option<&str> {
    via_args(line.trim().strip_prefix('/')?)
}

/// The arguments of a command line, with its prefix taken off, when it is
/// `via` (see `via_command`).
pub fn via_args(command: &str) -> Option<&str> {
    let (name, rest) = command
        .split_once(char::is_whitespace)
        .unwrap_or((command, ""));
    name.eq_ignore_ascii_case("via").then(|| rest.trim())
}

/// `/via [=run[:seen]] <target> <line>`, with any `/via` inside unwrapped
/// (the first one's run and the innermost one's target count), at most
/// eight deep in all, however they are spelled. Only the first may name a
/// run, or be `*` or `-`. The target ends at an ASCII space: IRC names never
/// hold one, but may hold other spaces, which must stay part of the name.
pub fn unwrap_via(rest: &str) -> Result<Via<'_>, &'static str> {
    let usage = "* usage: /via <#channel|nick> <line>";
    let mut rest = rest;
    let mut run = None;
    let mut seen = None;
    for depth in 0..8 {
        let mut words = rest.trim_start_matches(' ');
        // Only the outermost `/via` may name its run: the window side puts it
        // there, outside anything written by hand.
        if let Some(number) = words.strip_prefix('=').filter(|_| depth == 0) {
            let (number, after) = number.split_once(' ').ok_or(usage)?;
            let (number, count) = match number.split_once(':') {
                Some((number, count)) => (number, Some(count.parse().map_err(|_| usage)?)),
                None => (number, None),
            };
            // `=:<count>`: no run, as for a window switch.
            run = match number {
                "" if count.is_some() => None,
                number => Some(number.parse().map_err(|_| usage)?),
            };
            seen = count;
            words = after.trim_start_matches(' ');
        }
        let (target, line) = words.split_once(' ').ok_or(usage)?;
        let line = line.trim_start_matches(' ');
        let special = target.starts_with('=') || (depth > 0 && matches!(target, "*" | "-"));
        if target.is_empty() || special || line.is_empty() {
            return Err(usage);
        }
        match via_command(line) {
            Some(inner) => rest = inner,
            None => {
                return Ok(Via {
                    run,
                    seen,
                    target,
                    nested: depth > 0,
                    line,
                });
            }
        }
    }
    Err("* /via is nested too deeply")
}

/// Whether the line, run here, chooses a target on purpose: `/join` with a
/// channel, `/query` with a nick and nothing more, `/target` and `/win`
/// with a name, or a `/via` around one. The window side asks this too, so
/// both read the line the same way; `on_command` sets `picked` on exactly
/// these (a test holds them to it).
pub fn picks_target(line: &str) -> bool {
    let Some(command) = line.trim().strip_prefix('/') else {
        return false;
    };
    let (name, rest) = match command.split_once(char::is_whitespace) {
        Some((name, rest)) => (name, rest.trim()),
        None => (command, ""),
    };
    match name.to_ascii_lowercase().as_str() {
        "join" | "j" => first_token(rest)
            .is_some_and(|(channels, _)| channels.split(',').any(|name| !name.is_empty())),
        "query" => matches!(first_token(rest), Some((nick, "")) if !nick.is_empty()),
        "target" => match first_token(rest) {
            Some((flag, quiet)) if flag == "-q" && !quiet.is_empty() => true,
            Some((_, rest)) => rest.is_empty(),
            None => false,
        },
        "win" | "window" => matches!(first_token(rest), Some((_, ""))),
        "via" => unwrap_via(rest).is_ok_and(|via| picks_target(via.line)),
        _ => false,
    }
}

/// The nick of `/query <nick>` with nothing after it: the line that makes
/// the nick the target (see `picks_target`).
pub fn query_nick(line: &str) -> Option<String> {
    let command = line.trim().strip_prefix('/')?;
    let (name, rest) = command
        .split_once(char::is_whitespace)
        .unwrap_or((command, ""));
    if !name.eq_ignore_ascii_case("query") {
        return None;
    }
    match first_token(rest) {
        Some((nick, "")) if !nick.is_empty() => Some(nick),
        _ => None,
    }
}

/// A command line's name in lower case, and its arguments.
fn command_name(line: &str) -> Option<(String, &str)> {
    let command = line.trim().strip_prefix('/')?;
    let (name, rest) = command
        .split_once(char::is_whitespace)
        .unwrap_or((command, ""));
    Some((name.to_ascii_lowercase(), rest.trim()))
}

/// Whether the line is a window switch: `/target -q` alone, which chooses
/// the target the line came for.
fn is_window_switch(line: &str) -> bool {
    command_name(line).is_some_and(|(name, rest)| name == "target" && rest == "-q")
}

/// Whether the line is a typing notice for the target there is:
/// `/typing <state>` alone.
fn is_bare_typing(line: &str) -> bool {
    command_name(line).is_some_and(|(name, rest)| {
        name == "typing" && first_token(rest).is_some_and(|(_, more)| more.is_empty())
    })
}

fn first_token(source: &str) -> Option<(String, &str)> {
    let source = source.trim();
    if source.is_empty() {
        return None;
    }
    match source.split_once(char::is_whitespace) {
        Some((token, rest)) => Some((token.to_string(), rest.trim())),
        None => Some((source.to_string(), "")),
    }
}

pub fn valid_nick(nick: &str) -> bool {
    let mut chars = nick.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if nick.len() > 16 || !is_nick_leader(first) {
        return false;
    }
    chars.all(is_nick_char)
}

fn is_nick_leader(ch: char) -> bool {
    ch.is_ascii_alphabetic() || matches!(ch, '[' | ']' | '\\' | '`' | '_' | '^' | '{' | '|' | '}')
}

fn is_nick_char(ch: char) -> bool {
    is_nick_leader(ch) || ch.is_ascii_digit() || ch == '-'
}

pub fn help_lines() -> Vec<String> {
    [
        "* /join #channel[,#other] [key]",
        "* /partall [reason]",
        "* /part [#channel] [reason]",
        "* /msg <target> <text>",
        "* /query <nick>",
        "* /notice <target> <text>",
        "* /me <action>",
        "* /describe <target> <action>",
        "* /say <text>",
        "* /amsg <text>",
        "* /omsg /onotice [#channel] <text> (to the channel's operators)",
        "* /names [#channel]",
        "* /topic [#channel] [text]",
        "* /who [#channel|nick]",
        "* /identify <password>",
        "* /nickserv <command>",
        "* /cs /ms /hs /bs /os <command> (ChanServ, MemoServ, HostServ, BotServ, OperServ)",
        "* /kick [#channel] <nick> [reason]",
        "* /op /deop /voice /devoice /ban [#channel] <nick>",
        "* /mode <#channel> <flags> [nick]",
        "* /ignore <nick>",
        "* /unignore <nick>",
        "* /win [<channel|nick>]",
        "* /nick <name>",
        "* /target <#channel|nick>",
        "* /via <#channel|nick> <line> (one line for that target)",
        "* /raw <IRC line>",
        "* /away [reason]",
        "* /back",
        "* /invite <nick> [#channel]",
        "* /knock <#channel> [message]",
        "* /hop [#channel]",
        "* /ame <action>",
        "* /ctcp <nick> <command>",
        "* /ctcpreply <nick> <command> [text]",
        "* /ping <nick>",
        "* /echo <text>",
        "* /clear",
        "* /banlist [#channel]",
        "* /whowas <nick>",
        "* /umode <flags>",
        "* /halfop /owner /admin /except [#channel] <nick>",
        "* /key [secret]",
        "* /limit [count]",
        "* /silence <+nick|-nick>",
        "* /set <%var> <value>",
        "* /timer <seconds> <repeats> <command>",
        "* /flood <messages> <joins> <seconds> [ctcp]",
        "* /quit [reason]",
        "* other /commands go to the server as typed (/lusers, /motd, /stats, /oper, /userhost, ...)",
        "* any other line is sent to the current target",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pump() -> Pump {
        Pump::new(Config::new(
            "bob",
            "bob",
            "Bob",
            vec!["#openirc".into()],
            Vec::new(),
        ))
    }

    #[test]
    fn registration_is_nick_then_user() {
        let lines = pump().registration_lines();
        assert_eq!(lines[0], "CAP LS 302");
        assert_eq!(lines[1], "NICK :bob");
        assert_eq!(lines[2], "USER bob 0 * :Bob");
        let mut plain = Config::new("bob", "bob", "Bob", Vec::new(), Vec::new());
        plain.ircv3 = false;
        assert_eq!(Pump::new(plain).registration_lines()[0], "NICK :bob");
    }

    #[test]
    fn ping_produces_pong() {
        let effect = pump().on_server_line("PING :irc.example\r\n");
        assert_eq!(effect.outgoing, vec!["PONG :irc.example"]);
        assert_eq!(effect.lines, vec!["* pong"]);
    }

    #[test]
    fn welcome_triggers_autojoin_and_nick_collision_retries() {
        let mut session = pump();
        let collision = session.on_server_line(":irc 433 * bob :Nickname is already in use");
        assert_eq!(collision.outgoing, vec!["NICK :bob_"]);
        assert!(collision.lines[0].contains("bob_"));

        let welcome = session.on_server_line(":irc 001 bob_ :Welcome to the network");
        assert_eq!(welcome.outgoing, vec!["JOIN :#openirc"]);
        assert!(welcome.lines[0].contains("Welcome"));
    }

    #[test]
    fn chat_and_commands_write_irc() {
        let mut session = pump();
        session.on_server_line(":irc 001 bob :Welcome");
        let said = session.on_user_line("hello");
        assert_eq!(said.outgoing, vec!["PRIVMSG #openirc :hello"]);
        assert_eq!(said.lines, vec!["[#openirc] <bob> hello"]);

        let action = session.on_user_line("/me waves");
        assert_eq!(
            action.outgoing,
            vec!["PRIVMSG #openirc :\u{1}ACTION waves\u{1}"]
        );

        let quit = session.on_user_line("/quit bye");
        assert!(quit.quit);
        assert_eq!(quit.outgoing, vec!["QUIT :bye"]);
    }

    #[test]
    fn version_ctcp_replies_with_openirc() {
        let effect = pump().on_server_line(":alice!a@h PRIVMSG bob :\u{1}VERSION\u{1}");
        assert_eq!(effect.outgoing.len(), 1);
        assert!(effect.outgoing[0].starts_with("NOTICE alice :"));
        assert!(effect.outgoing[0].contains("OpenIRC"));
    }

    #[test]
    fn channel_state_tracks_names_topic_kick_and_windows() {
        let mut session = pump();
        session.on_server_line(
            ":irc 005 bob CASEMAPPING=rfc1459 PREFIX=(ov)@+ CHANTYPES=# :are supported",
        );
        session.on_server_line(":bob!b@h JOIN :#openirc");
        let topic = session.on_server_line(":irc 332 bob #openirc :build the client");
        assert_eq!(topic.lines, vec!["* #openirc topic: build the client"]);
        session.on_server_line(":irc 353 bob = #openirc :@Alice bob +carol");
        let names = session.on_server_line(":irc 366 bob #openirc :End of /NAMES list");
        assert_eq!(names.lines, vec!["* #openirc names: @Alice +carol bob"]);

        let shown = session.on_user_line("/topic");
        assert_eq!(shown.lines, vec!["* #openirc topic: build the client"]);
        let set = session.on_user_line("/topic new topic");
        assert_eq!(set.outgoing, vec!["TOPIC #openirc :new topic"]);

        let kick = session.on_server_line(":Alice!a@h KICK #openirc carol :bye");
        assert_eq!(kick.lines, vec!["* Alice kicked carol from #openirc (bye)"]);
        let names = session.on_user_line("/names");
        assert_eq!(names.outgoing, vec!["NAMES :#openirc"]);

        let windows = session.on_user_line("/win");
        assert!(windows.lines.iter().any(|line| line.contains("#openirc")));

        let query = session.on_server_line(":dave!d@h PRIVMSG bob :psst");
        assert_eq!(query.lines, vec!["[dave] <dave> psst"]);

        let gone = session.on_server_line(":bob!b@h PART #openirc");
        assert!(gone.lines[0].contains("left #openirc"));
        let said = session.on_user_line("hello");
        assert!(said.lines[0].contains("no channel"));
    }

    #[test]
    fn op_voice_and_ignore() {
        let mut session = pump();
        session.on_server_line(":bob!b@h JOIN :#openirc");
        let op = session.on_user_line("/op alice");
        assert_eq!(op.outgoing, vec!["MODE #openirc +o :alice"]);
        let voice = session.on_user_line("/voice alice");
        assert_eq!(voice.outgoing, vec!["MODE #openirc +v :alice"]);
        let ban = session.on_user_line("/ban alice");
        assert_eq!(ban.outgoing, vec!["MODE #openirc +b :alice!*@*"]);
        session.on_user_line("/ignore alice");
        let hidden = session.on_server_line(":alice!a@h PRIVMSG #openirc :secret");
        assert!(hidden.lines.is_empty());
    }

    #[test]
    fn ctcp_case_never_slips_past_a_typed_ignore() {
        let mut session = pump();
        session.on_user_line("/ignore -d alice");
        let offer = session
            .on_server_line(":alice!a@h PRIVMSG me :\u{1}dcc CHAT chat 2130706433 5000\u{1}");
        assert!(offer.events.is_empty(), "{:?}", offer.events);
        session.on_user_line("/ignore -p bob");
        let action = session.on_server_line(":bob!b@h PRIVMSG me :\u{1}action waves\u{1}");
        assert!(action.events.is_empty(), "{:?}", action.events);
        // A command that only begins with ACTION is a CTCP request, not an
        // action, so a private-message ignore does not cover it.
        let other = session.on_server_line(":bob!b@h PRIVMSG me :\u{1}ACTIONX hi\u{1}");
        assert!(!other.events.is_empty());
    }

    #[test]
    fn color_stripping_never_turns_a_message_into_one_an_ignore_would_stop() {
        // `k` strips codes from what is shown; with a bold byte in front,
        // the command is not DCC or ACTION, and stripping must not make it
        // one after the ignore check.
        let mut session = pump();
        session.on_user_line("/ignore -dk alice");
        let offer = session.on_server_line(
            ":alice!a@h PRIVMSG bob :\u{1}\u{2}dcc CHAT chat 203.0.113.10 9000\u{1}",
        );
        assert!(
            offer
                .events
                .iter()
                .all(|event| !matches!(event, Event::DccChat { .. } | Event::DccSend { .. })),
            "{:?}",
            offer.events
        );
        session.on_user_line("/ignore -pk bob");
        let action = session.on_server_line(":bob!b@h PRIVMSG me :\u{1}\u{2}ACTION waves\u{1}");
        assert!(
            action
                .events
                .iter()
                .all(|event| !matches!(event, Event::Chat { .. })),
            "{:?}",
            action.events
        );
        // A real action from someone with only `k` set loses its colors.
        session.on_user_line("/ignore -k carol");
        let colored =
            session.on_server_line(":carol!c@h PRIVMSG #openirc :\u{1}ACTION \u{3}4red\u{3}\u{1}");
        assert!(colored.events.iter().any(|event| matches!(
            event,
            Event::Chat { text, action: true, .. } if text == "red"
        )));
    }

    #[test]
    fn via_runs_a_line_for_a_target_and_cannot_be_nested_without_end() {
        let mut session = pump();
        session.on_server_line(":bob!b@h JOIN :#openirc");
        session.on_server_line(":bob!b@h JOIN :#other");
        // Deeply nested wrappers end with a note, not a crash.
        let deep = format!("{}hello", "/via #openirc ".repeat(1_024));
        let effect = session.on_user_line(&deep);
        assert!(effect.outgoing.is_empty());
        assert!(
            effect
                .lines
                .iter()
                .any(|line| line.contains("nested too deeply"))
        );
        // However the nested ones are spelled.
        for wrapper in [
            "/VIA #openirc ",
            "/Via\t#openirc ",
            "/via #openirc /via\t#other ",
        ] {
            let deep = format!("{}hello", wrapper.repeat(1_024));
            let effect = session.on_user_line(&deep);
            assert!(effect.outgoing.is_empty(), "{wrapper}");
            assert!(
                effect
                    .lines
                    .iter()
                    .any(|line| line.contains("nested too deeply"))
            );
        }
        // A few levels are fine: the innermost target counts.
        let nested = session.on_user_line("/via #openirc /via #other hi");
        assert_eq!(nested.outgoing, vec!["PRIVMSG #other :hi"]);
        let nested = session.on_user_line("/via #openirc /VIA\t#other hi");
        assert_eq!(nested.outgoing, vec!["PRIVMSG #other :hi"]);
        // A target chosen in a run leads the rest of that run, but not a
        // line with a `/via` of its own, nor another run.
        session.on_user_line("/via =1 #openirc /query alice");
        let rest = session.on_user_line("/via =1 #openirc /say rest");
        assert_eq!(rest.outgoing, vec!["PRIVMSG alice :rest"]);
        let own = session.on_user_line("/via =1 #openirc /via #other own");
        assert_eq!(own.outgoing, vec!["PRIVMSG #other :own"]);
        let next = session.on_user_line("/via =2 #openirc next");
        assert_eq!(next.outgoing, vec!["PRIVMSG #openirc :next"]);
        // A choice made under a run number not reached yet is over by the
        // time it is: another run has spoken since.
        session.on_user_line("/via =9 #other /query mallory");
        session.on_user_line("/via =3 #openirc between");
        let later = session.on_user_line("/via =9 #openirc later");
        assert_eq!(later.outgoing, vec!["PRIVMSG #openirc :later"]);
        // A choice follows a nick change, so the rest of the run never
        // reaches whoever takes the old nick.
        session.on_user_line("/via =4 #openirc /query alice");
        session.on_server_line(":alice!a@h NICK :alice2");
        session.on_server_line(":mallory!m@h NICK :alice");
        let renamed = session.on_user_line("/via =4 #openirc /say confidential");
        assert_eq!(renamed.outgoing, vec!["PRIVMSG alice2 :confidential"]);
        for bad in [
            "/via =x #openirc hi",
            "/via =1",
            "/via =1 #openirc",
            // Only the outermost may name a run.
            "/via #openirc /via =5 #other hi",
            "/via =6 #openirc /via =5 #other hi",
        ] {
            let effect = session.on_user_line(bad);
            assert!(effect.outgoing.is_empty(), "{bad}");
        }
        // A target chosen on purpose inside /via stays, even the same one.
        for choose in [
            "/target #openirc",
            "/target -q #openirc",
            "/join #openirc",
            "/win #openirc",
        ] {
            session.on_user_line("/target -q #other");
            session.on_user_line(&format!("/via #openirc {choose}"));
            let after = session.on_user_line("/say after");
            assert_eq!(after.outgoing, vec!["PRIVMSG #openirc :after"], "{choose}");
        }
        // Without a choice, the target is as it was.
        session.on_user_line("/target -q #other");
        session.on_user_line("/via #openirc /say inside");
        let after = session.on_user_line("/say after");
        assert_eq!(after.outgoing, vec!["PRIVMSG #other :after"]);
    }

    #[test]
    fn echoes_to_a_service_hide_its_password() {
        let mut session = pump();
        let mut shown = Vec::new();
        for line in [
            "/describe NickServ ID swordfish",
            "/ctcp NickServ ID swordfish",
            "/ctcpreply NickServ ID swordfish",
            "/via NickServ /me ID swordfish",
            "/ctcp NickServ ACTION ID swordfish",
            "/ctcpreply NickServ ACTION ID swordfish",
        ] {
            let effect = session.on_user_line(line);
            assert!(!effect.outgoing.is_empty(), "{line}");
            // The Debug window's copy of what went out, too.
            shown.extend(
                effect
                    .outgoing
                    .iter()
                    .map(|wire| crate::connection::mask_secrets(wire)),
            );
            shown.extend(effect.lines);
            for event in effect.events {
                if let Event::Sent { text, .. } = event {
                    shown.push(text);
                }
            }
        }
        assert!(shown.iter().any(|line| line.contains("****")), "{shown:?}");
        assert!(
            shown.iter().all(|line| !line.contains("swordfish")),
            "{shown:?}"
        );
    }

    #[test]
    fn a_typing_notice_written_before_a_rename_follows_it() {
        let mut session = pump();
        session.on_server_line(":irc CAP me ACK :message-tags");
        session.on_server_line(":carol!c@h NICK :caroline");
        session.on_server_line(":mallory!m@h NICK :carol");
        let typing = session.on_user_line("/via =:0 carol /typing active");
        assert_eq!(typing.outgoing, vec!["@+typing=active TAGMSG caroline"]);
        // Written after the window saw them: for the new carol.
        let later = session.on_user_line("/via =:2 carol /typing done");
        assert_eq!(later.outgoing, vec!["@+typing=done TAGMSG carol"]);
    }

    #[test]
    fn the_window_side_reads_a_target_choice_as_the_server_side_makes_it() {
        for line in [
            "/join #a",
            "/j a,b key",
            "/join ,",
            "/query alice",
            "/QUERY\talice",
            "/query alice hi",
            "/query",
            "/target #x",
            "/target -q #x",
            "/target -q",
            "/target a b",
            "/target",
            "/win #x",
            "/window #x",
            "/win",
            "/win a b",
            "/via #x /query a",
            "/via #x /say a",
            "/msg a b",
            "/say hi",
            "/topic",
            "hello",
        ] {
            let mut session = pump();
            session.on_user_line(&format!("/via =1 #base {line}"));
            assert_eq!(session.run_choice.is_some(), picks_target(line), "{line}");
        }
    }

    #[test]
    fn a_line_written_before_many_nick_changes_is_not_sent_to_a_guess() {
        let mut session = pump();
        for count in 0..RENAMES_KEPT + 1 {
            session.on_server_line(&format!(":n{count}!u@h NICK :m{count}"));
        }
        // Written when the window had seen none of them: the first is
        // forgotten, so where it belongs is unknown.
        let early = session.on_user_line("/via =1:0 n0 hello");
        assert!(early.outgoing.is_empty());
        // Refused lines are never shown: they may hold a password.
        for line in [
            "/via =5:0 n0 ID swordfish",
            "/via =6:0 n0 /say ID swordfish",
            "/via =6:0 n0 /me ID swordfish",
            "/via =7 - /msg NickServ ID swordfish",
        ] {
            let refused = session.on_user_line(line);
            assert!(refused.outgoing.is_empty(), "{line}");
            assert!(!refused.lines.is_empty(), "{line}");
            assert!(
                refused
                    .lines
                    .iter()
                    .all(|shown| !shown.contains("swordfish")),
                "{line}: {:?}",
                refused.lines
            );
        }
        // Any other line refused this way leaves the target as it was.
        session.on_user_line("/target -q #chan");
        session.on_user_line("/via =:0 n1 /typing done");
        let kept = session.on_user_line("/via =8:0 * kept");
        assert_eq!(kept.outgoing, vec!["PRIVMSG #chan :kept"]);
        // A window switch refused this way leaves no target behind for
        // words with none of their own.
        session.on_user_line("/via =:0 n1 /target -q");
        let after = session.on_user_line("/via =8:0 * confidential");
        assert!(after.outgoing.is_empty(), "{:?}", after.outgoing);
        // Written after the first: every change it needs is still known.
        let later = session.on_user_line("/via =2:1 n5 hello");
        assert_eq!(later.outgoing, vec!["PRIVMSG m5 :hello"]);
        // The server's own case rules say who renamed.
        for (mapping, sent) in [
            ("rfc1459", "PRIVMSG caroline :hi"),
            ("ascii", "PRIVMSG carol{ :hi"),
        ] {
            let mut cased = pump();
            cased.on_server_line(&format!(":irc 005 me CASEMAPPING={mapping} :are supported"));
            cased.on_server_line(":Carol[!c@h NICK :caroline");
            let effect = cased.on_user_line("/via =1:0 carol{ hi");
            assert_eq!(effect.outgoing, vec![sent], "{mapping}");
        }
        // Channels never change names.
        let channel = session.on_user_line("/via =3:0 #chan hello");
        assert_eq!(channel.outgoing, vec!["PRIVMSG #chan :hello"]);
        // `*` and `-` belong to the window side, and only outermost.
        for bad in ["/via #a /via * hi", "/via #a /via - hi"] {
            assert!(session.on_user_line(bad).outgoing.is_empty(), "{bad}");
        }
        // `-` with nothing chosen sends nothing, even with a target here.
        session.on_user_line("/target -q #chan");
        assert!(
            session
                .on_user_line("/via =4 - /say hi")
                .outgoing
                .is_empty()
        );
    }

    #[test]
    fn a_nick_freed_by_a_quit_and_retaken_is_not_followed() {
        let mut s = pump();
        s.on_server_line(":carol!c@h JOIN :#openirc");
        s.on_server_line(":dave!d@h JOIN :#openirc");
        s.on_server_line(":carol!c@h QUIT :bye");
        s.on_server_line(":dave!d@h NICK :carol");
        assert!(s.on_user_line("/via =1:0 carol hello").outgoing.is_empty()); // sends PRIVMSG carol today
        let mut s = pump();
        s.on_server_line(":carol!c@h NICK :caroline");
        s.on_server_line(":caroline!c@h QUIT :bye");
        s.on_server_line(":mallory!m@h NICK :caroline");
        assert!(s.on_user_line("/via =2:0 carol hello").outgoing.is_empty()); // sends PRIVMSG caroline today
    }

    #[test]
    fn logins_to_q_and_x_are_masked() {
        let mut s = pump();
        for line in [
            "/msg Q AUTH me swordfish",
            "/msg X@channels.undernet.org LOGIN me swordfish",
        ] {
            let e = s.on_user_line(line);
            let shown: Vec<String> = e
                .outgoing
                .iter()
                .map(|w| crate::connection::mask_secrets(w))
                .chain(e.lines)
                .collect();
            assert!(shown.iter().all(|l| !l.contains("swordfish")), "{shown:?}");
        }
        assert!(crate::ui::input::may_hold_secret(
            "/msg Q AUTH me swordfish",
            "/"
        ));
    }

    #[test]
    fn every_echo_of_a_login_to_a_bot_hides_its_password() {
        let mut session = pump();
        for line in [
            "/msg Q AUTH me swordfish",
            "/query Q AUTH me swordfish",
            "/notice Q AUTH me swordfish",
            "/describe Q AUTH me swordfish",
            "/ctcp Q AUTH me swordfish",
            "/ctcp Q ACTION AUTH me swordfish",
            "/ctcpreply X LOGIN me swordfish",
            "/via Q AUTH me swordfish",
            "/via Q /me AUTH me swordfish",
            "/via X@channels.undernet.org /say LOGIN me swordfish",
            "/raw PRIVMSG Q :AUTH me swordfish",
            "/msg bob SET PASSWORD swordfish",
        ] {
            let effect = session.on_user_line(line);
            // The password goes out as typed.
            assert!(
                effect
                    .outgoing
                    .iter()
                    .any(|wire| wire.contains("swordfish")),
                "{line}"
            );
            // The Debug window's copy of what went out, the transcript and
            // the log.
            let mut shown: Vec<String> = effect
                .outgoing
                .iter()
                .map(|wire| crate::connection::mask_secrets(wire))
                .collect();
            shown.extend(effect.lines);
            for event in effect.events {
                if let Event::Sent { text, .. } = event {
                    shown.push(text);
                }
            }
            assert!(shown.iter().any(|line| line.contains("****")), "{line}");
            assert!(
                shown.iter().all(|line| !line.contains("swordfish")),
                "{line}: {shown:?}"
            );
        }
    }

    #[test]
    fn a_quit_ends_what_was_written_for_that_nick() {
        let mut session = pump();
        session.on_server_line(":carol!c@h JOIN :#openirc");
        session.on_server_line(":dave!d@h JOIN :#openirc");
        session.on_server_line(":carol!c@h QUIT :bye");
        session.on_server_line(":dave!d@h NICK :carol");
        // Refused with a note that names carol and nothing of the line.
        let refused = session.on_user_line("/via =1:0 carol confidential");
        assert!(refused.outgoing.is_empty());
        assert_eq!(
            refused.lines,
            vec!["* a line for carol was not sent: carol left meanwhile"]
        );
        // Written once the window had seen her quit: for whoever is carol
        // now, as any line to a nick is.
        for line in ["/via =2:1 carol hello", "/via =3:2 carol hello"] {
            let effect = session.on_user_line(line);
            assert_eq!(effect.outgoing, vec!["PRIVMSG carol :hello"], "{line}");
        }
        // Someone else's quit stops nothing: dave is carol now.
        let dave = session.on_user_line("/via =4:0 dave hi");
        assert_eq!(dave.outgoing, vec!["PRIVMSG carol :hi"]);
        // Channels never leave.
        let channel = session.on_user_line("/via =5:0 #openirc hello");
        assert_eq!(channel.outgoing, vec!["PRIVMSG #openirc :hello"]);
        // Our own quit is not counted: we are the ones leaving.
        let before = session.renames;
        let ours = session.on_server_line(":bob!b@h QUIT :bye");
        assert_eq!((session.renames, ours.renames), (before, before));
        // Rejoining under the same nick before the line arrives is still
        // someone who left: refused, on the safe side.
        let mut back = pump();
        back.on_server_line(":carol!c@h QUIT :bye");
        back.on_server_line(":carol!c@h JOIN :#openirc");
        assert!(back.on_user_line("/via =1:0 carol hi").outgoing.is_empty());
        // The server's own case rules say who left.
        let mut cased = pump();
        cased.on_server_line(":irc 005 me CASEMAPPING=rfc1459 :are supported");
        cased.on_server_line(":Carol[!c@h QUIT :bye");
        cased.on_server_line(":mallory!m@h NICK :carol{");
        assert!(
            cased
                .on_user_line("/via =1:0 carol{ hi")
                .outgoing
                .is_empty()
        );
    }

    #[test]
    fn quits_count_toward_the_kept_changes_like_renames() {
        for (changes, kept) in [
            (RENAMES_KEPT - 1, true),
            (RENAMES_KEPT, true),
            (RENAMES_KEPT + 1, false),
        ] {
            let mut session = pump();
            session.on_server_line(":carol!c@h NICK :caroline");
            for count in 1..changes {
                if count % 2 == 0 {
                    session.on_server_line(&format!(":q{count}!u@h QUIT :bye"));
                } else {
                    session.on_server_line(&format!(":n{count}!u@h NICK :m{count}"));
                }
            }
            assert_eq!(session.renames, changes as u64);
            let effect = session.on_user_line("/via =1:0 carol hello");
            if kept {
                assert_eq!(
                    effect.outgoing,
                    vec!["PRIVMSG caroline :hello"],
                    "{changes}"
                );
            } else {
                assert!(effect.outgoing.is_empty(), "{changes}");
                assert_eq!(
                    effect.lines,
                    vec!["* a line for carol was not sent: nicks changed too often meanwhile"]
                );
            }
        }
    }

    #[test]
    fn a_line_queued_across_a_netsplit_still_reaches_its_nick() {
        // A thousand people quit at once, and carol renames among them: a
        // line written for her before all of it is still hers.
        let mut session = pump();
        for count in 0..1_000 {
            if count == 500 {
                session.on_server_line(":carol!c@h NICK :caroline");
            }
            session.on_server_line(&format!(":split{count}!u@h QUIT :irc.a irc.b"));
        }
        let effect = session.on_user_line("/via =1:0 carol hello");
        assert_eq!(effect.outgoing, vec!["PRIVMSG caroline :hello"]);
    }

    #[test]
    fn what_was_chosen_for_someone_who_left_goes_nowhere() {
        // A run chose alice; she quits and mallory takes her nick. The rest
        // of the run's words reach neither mallory nor the channel the run
        // began in.
        let mut session = pump();
        session.on_server_line(":me!u@h JOIN :#openirc");
        session.on_user_line("/via =5:0 #openirc /query alice");
        session.on_server_line(":alice!a@h QUIT :bye");
        session.on_server_line(":mallory!m@h NICK :alice");
        let words = session.on_user_line("/via =5:0 #openirc /say confidential");
        assert!(words.outgoing.is_empty(), "{:?}", words.outgoing);
        assert_eq!(
            words.lines,
            vec!["* a line for alice was not sent: alice left meanwhile"]
        );
        // A line of that run with a target of its own still runs, and a new
        // choice leads the rest again.
        let whois = session.on_user_line("/via =5:0 #openirc /whois carol");
        assert_eq!(whois.outgoing, vec!["WHOIS carol :carol"]);
        session.on_user_line("/via =5:0 #openirc /query bob");
        let later = session.on_user_line("/via =5:0 #openirc /say for bob");
        assert_eq!(later.outgoing, vec!["PRIVMSG bob :for bob"]);
        // The server side's own target, once she left, is no one.
        let mut own = pump();
        own.on_user_line("/target -q alice");
        own.on_server_line(":alice!a@h QUIT :bye");
        own.on_server_line(":mallory!m@h NICK :alice");
        assert!(own.on_user_line("/via =1:2 * hello").outgoing.is_empty());
        assert!(own.on_user_line("hello").outgoing.is_empty());
    }

    #[test]
    fn a_line_that_needs_no_nick_runs_when_its_window_nick_is_unknown() {
        let mut session = pump();
        for count in 0..RENAMES_KEPT + 1 {
            session.on_server_line(&format!(":n{count}!u@h NICK :m{count}"));
        }
        // Lines written in n0's query before its changes were forgotten,
        // which name a target of their own or need none: they run.
        session.on_user_line("/target -q #chan");
        let join = session.on_user_line("/via =1:0 n0 /join #foo");
        assert_eq!(join.outgoing, vec!["JOIN :#foo"]);
        assert!(join.lines.is_empty(), "{:?}", join.lines);
        // A join chooses its channel, as it always does.
        let after_join = session.on_user_line("/via =9:0 * after the join");
        assert_eq!(after_join.outgoing, vec!["PRIVMSG #foo :after the join"]);
        session.on_user_line("/target -q #chan");
        for (line, sent) in [
            ("/via =2:0 n0 /whois bob", "WHOIS bob :bob"),
            ("/via =3:0 n0 /msg bob hi", "PRIVMSG bob :hi"),
            (
                "/via =4:0 n0 /identify swordfish",
                "PRIVMSG NickServ :IDENTIFY swordfish",
            ),
        ] {
            let effect = session.on_user_line(line);
            assert_eq!(effect.outgoing, vec![sent], "{line}");
            assert!(
                effect
                    .lines
                    .iter()
                    .all(|shown| !shown.contains("swordfish")),
                "{line}: {:?}",
                effect.lines
            );
        }
        // While it runs, the window's target is no one's: what would have
        // gone there gets the usual note instead.
        let topic = session.on_user_line("/via =5:0 n0 /topic new topic");
        assert!(topic.outgoing.is_empty(), "{:?}", topic.outgoing);
        assert_eq!(topic.lines, vec!["* no channel selected"]);
        // Words for n0 are refused, and show nothing of themselves.
        for line in [
            "/via =6:0 n0 hello",
            "/via =7:0 n0 /say hello",
            "/via =8:0 n0 /me hello",
        ] {
            let refused = session.on_user_line(line);
            assert!(refused.outgoing.is_empty(), "{line}");
            assert_eq!(
                refused.lines,
                vec!["* a line for n0 was not sent: nicks changed too often meanwhile"],
                "{line}"
            );
        }
        // None of that moved the target.
        let kept = session.on_user_line("/via =10:0 * kept");
        assert_eq!(kept.outgoing, vec!["PRIVMSG #chan :kept"]);
        // A typing notice with no target of its own is refused, and leaves
        // the target too; one with a target of its own runs.
        session.on_server_line(":irc CAP me ACK :message-tags");
        assert!(
            session
                .on_user_line("/via =:0 n0 /typing active")
                .outgoing
                .is_empty()
        );
        assert_eq!(
            session
                .on_user_line("/via =:0 n0 /typing bob active")
                .outgoing,
            vec!["@+typing=active TAGMSG bob"]
        );
        let kept = session.on_user_line("/via =11:0 * still kept");
        assert_eq!(kept.outgoing, vec!["PRIVMSG #chan :still kept"]);
        // Only a refused window switch leaves no target behind.
        session.on_user_line("/via =:0 n0 /target -q");
        assert!(
            session
                .on_user_line("/via =12:0 * gone")
                .outgoing
                .is_empty()
        );
    }

    #[test]
    fn a_network_ignore_waits_for_the_network_name() {
        let mut session = pump();
        session.on_user_line("/ignore -p -wOFTC alice");
        let early = session.on_server_line(":alice!a@h PRIVMSG me :before 005");
        assert!(!early.events.is_empty(), "unknown network: not ignored");
        session.on_server_line(":irc 005 me NETWORK=Libera :are supported");
        let libera = session.on_server_line(":alice!a@h PRIVMSG me :on Libera");
        assert!(!libera.events.is_empty(), "another network: not ignored");
        let mut oftc = pump();
        oftc.on_user_line("/ignore -p -wOFTC alice");
        oftc.on_server_line(":irc 005 me NETWORK=OFTC :are supported");
        let hidden = oftc.on_server_line(":alice!a@h PRIVMSG me :on OFTC");
        assert!(hidden.events.is_empty());
    }

    #[test]
    fn long_sasl_answers_are_sent_in_400_byte_pieces() {
        assert_eq!(authenticate_lines("abc"), vec!["AUTHENTICATE abc"]);
        let exact = "a".repeat(400);
        assert_eq!(
            authenticate_lines(&exact),
            vec![
                format!("AUTHENTICATE {exact}"),
                "AUTHENTICATE +".to_string()
            ]
        );
        let longer = "b".repeat(404);
        assert_eq!(
            authenticate_lines(&longer),
            vec![
                format!("AUTHENTICATE {}", "b".repeat(400)),
                "AUTHENTICATE bbbb".to_string()
            ]
        );
        // "bob\0bob\0" and 292 password bytes encode to exactly 400.
        let encoded = sasl_plain("bob", &"p".repeat(292));
        assert_eq!(encoded.len(), 400);
        assert_eq!(authenticate_lines(&encoded).len(), 2);

        // Through the server's AUTHENTICATE +, as a login really goes.
        for (password, pieces) in [("p".repeat(292), 2), ("q".repeat(700), 3), ("r".into(), 1)] {
            let mut config = Config::new("bob", "bob", "Bob", Vec::new(), Vec::new());
            config.sasl_password = password.clone();
            let mut session = Pump::new(config);
            session.registration_lines();
            session.on_server_line(":irc CAP * LS :sasl=PLAIN");
            session.on_server_line(":irc CAP bob ACK :sasl");
            let answer = session.on_server_line(":irc AUTHENTICATE +");
            assert_eq!(answer.outgoing.len(), pieces, "{password:?}");
            let joined: String = answer
                .outgoing
                .iter()
                .map(|line| line.strip_prefix("AUTHENTICATE ").unwrap())
                .filter(|piece| *piece != "+")
                .collect();
            assert_eq!(joined, sasl_plain("bob", &password));
            assert!(
                answer
                    .outgoing
                    .iter()
                    .all(|line| line.len() <= "AUTHENTICATE ".len() + 400)
            );
        }
    }

    #[test]
    fn a_services_password_is_sent_but_shown_as_stars() {
        let mut session = pump();
        let sent = session.on_user_line("/msg NickServ IDENTIFY hunter2");
        assert_eq!(sent.outgoing, vec!["PRIVMSG NickServ :IDENTIFY hunter2"]);
        assert!(sent.events.iter().any(|event| matches!(
            event,
            Event::Sent { text, .. } if text == "IDENTIFY ****"
        )));
        assert!(sent.lines.iter().all(|line| !line.contains("hunter2")));

        // Every way of writing to the services reaches the server in full
        // and shows the password nowhere the transcript or log reads.
        for line in [
            "/cs IDENTIFY #room swordfish",
            "/chanserv identify #room swordfish",
            "/os login swordfish",
            "/notice NickServ IDENTIFY swordfish",
            "/msg NickServ IDENTIFY swordfish",
            "/query ChanServ IDENTIFY #room swordfish",
            "/ns IDENTIFY swordfish",
        ] {
            let effect = session.on_user_line(line);
            assert!(
                effect.outgoing.iter().any(|out| out.contains("swordfish")),
                "{line}: {:?}",
                effect.outgoing
            );
            let shown: Vec<String> = effect
                .lines
                .iter()
                .cloned()
                .chain(effect.events.iter().map(crate::display::format_event))
                .collect();
            assert!(
                shown.iter().all(|text| !text.contains("swordfish")),
                "{line}: {shown:?}"
            );
        }
    }

    #[test]
    fn who_and_nickserv_lines() {
        let mut session = pump();
        session.on_server_line(":bob!b@h JOIN :#openirc");
        let who = session.on_user_line("/who");
        assert_eq!(who.outgoing, vec!["WHO :#openirc"]);
        let away = session.on_user_line("/away lunch");
        assert_eq!(away.outgoing, vec!["AWAY :lunch"]);
        assert!(away.lines.iter().any(|line| line.contains("lunch")));
        let back = session.on_user_line("/back");
        assert_eq!(back.outgoing, vec!["AWAY"]);
        let identify = session.on_user_line("/identify secret");
        assert_eq!(identify.outgoing, vec!["PRIVMSG NickServ :IDENTIFY secret"]);
        let send = session.on_server_line(
            ":carol!c@h PRIVMSG bob :\u{1}DCC SEND file.txt 2130706433 5000 9\u{1}",
        );
        assert!(send.lines.iter().any(|line| line.contains("file.txt")));
        let accept =
            session.on_server_line(":carol!c@h PRIVMSG bob :\u{1}DCC ACCEPT file.txt 5000 4\u{1}");
        assert!(accept.lines.iter().any(|line| line.contains("resume")));
    }

    #[test]
    fn action_is_rendered_and_plain_chat_keeps_the_target() {
        let mut session = pump();
        let effect = session.on_server_line(":alice!a@h PRIVMSG #openirc :\u{1}ACTION waves\u{1}");
        assert_eq!(effect.lines, vec!["[#openirc] * alice waves"]);
        let chat = session.on_server_line(":alice!a@h PRIVMSG #openirc :hi");
        assert_eq!(chat.lines, vec!["[#openirc] <alice> hi"]);
    }

    #[test]
    fn flood_drops_extra_messages_and_hides_extra_join_lines() {
        let mut session = pump();
        session.on_server_line(":bob!b@h JOIN :#openirc");
        for n in 0..10 {
            let effect = session.on_server_line(&format!(":alice!a@h PRIVMSG #openirc :hello {n}"));
            assert!(effect.lines.iter().any(|line| line.contains("alice")));
        }
        let tripped = session.on_server_line(":alice!a@h PRIVMSG #openirc :too many");
        assert_eq!(
            tripped.lines,
            vec!["* dropping messages from alice (flood)"]
        );
        let held = session.on_server_line(":alice!a@h PRIVMSG #openirc :still too many");
        assert!(held.lines.is_empty());
        assert!(held.events.is_empty());

        for n in 0..8 {
            let effect = session.on_server_line(&format!(":user{n}!u@h JOIN :#openirc"));
            assert!(effect.lines.iter().any(|line| line.contains("joined")));
        }
        let joins = session.on_server_line(":user8!u@h JOIN :#openirc");
        assert_eq!(joins.lines, vec!["* hiding join lines in #openirc"]);
        let quiet = session.on_server_line(":user9!u@h JOIN :#openirc");
        assert!(quiet.lines.is_empty());
        assert_eq!(
            quiet.events,
            vec![Event::Member {
                who: "user9".into(),
                channel: "#openirc".into(),
            }]
        );
    }

    #[test]
    fn ctcp_answers_are_limited_and_replies_are_events() {
        let mut session = pump();
        session.on_user_line("/flood 0 0 5 2");
        let mut answered = 0;
        let mut notes = Vec::new();
        for nick in ["a", "b", "c", "d"] {
            let effect =
                session.on_server_line(&format!(":{nick}!u@h PRIVMSG bob :\u{1}VERSION\u{1}"));
            answered += effect.outgoing.len();
            notes.extend(effect.lines);
        }
        assert_eq!(answered, 2, "two answers per window, counted over everyone");
        assert!(notes.iter().any(|line| line.contains("not answering CTCP")));

        let reply = session.on_server_line(":alice!a@h NOTICE bob :\u{1}VERSION Other 1.0\u{1}");
        assert_eq!(
            reply.events,
            vec![Event::CtcpReply {
                from: "alice".into(),
                command: "VERSION".into(),
                text: "Other 1.0".into(),
            }]
        );
        let ping = session.on_user_line("/ctcp alice PING");
        let sent = ping.outgoing[0].clone();
        let stamp = sent
            .trim_start_matches("PRIVMSG alice :\u{1}PING ")
            .trim_end_matches('\u{1}');
        let pong =
            session.on_server_line(&format!(":alice!a@h NOTICE bob :\u{1}PING {stamp}\u{1}"));
        match &pong.events[0] {
            Event::CtcpReply { command, text, .. } => {
                assert_eq!(command, "PING");
                assert!(text.ends_with("seconds"), "{text}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn ircv3_capabilities_are_negotiated_and_used() {
        let mut session = pump();
        session.registration_lines();
        let first = session
            .on_server_line(":irc CAP * LS * :multi-prefix away-notify sts=port=6697,duration=60");
        assert!(first.outgoing.is_empty(), "waits for the last LS line");
        let last = session.on_server_line(":irc CAP * LS :server-time message-tags unknown-cap");
        assert_eq!(
            last.outgoing,
            vec!["CAP REQ :multi-prefix away-notify server-time message-tags"]
        );
        assert!(last.events.contains(&Event::Sts {
            port: Some(6697),
            duration: Some(60)
        }));
        let ack = session
            .on_server_line(":irc CAP bob ACK :multi-prefix away-notify server-time message-tags");
        assert_eq!(ack.outgoing, vec!["CAP END"]);
        assert!(session.has_cap("message-tags"));

        let timed =
            session.on_server_line("@time=2026-10-13T12:00:00.000Z :alice!a@h PRIVMSG bob :hi");
        assert_eq!(timed.time, Some(1_791_892_800));
        let away = session.on_server_line(":alice!a@h AWAY :lunch");
        assert_eq!(
            away.events,
            vec![Event::UserAway {
                who: "alice".into(),
                reason: Some("lunch".into())
            }]
        );
        let typing = session.on_server_line("@+typing=active :alice!a@h TAGMSG bob");
        assert_eq!(
            typing.events,
            vec![Event::Typing {
                who: "alice".into(),
                target: "alice".into(),
                state: "active".into()
            }]
        );
        let sent = session.on_user_line("/typing #openirc active");
        assert_eq!(sent.outgoing, vec!["@+typing=active TAGMSG #openirc"]);
        let joined = session.on_server_line(":carol!c@h JOIN #openirc carolacct :Carol");
        assert!(joined.events.contains(&Event::Account {
            who: "carol".into(),
            account: Some("carolacct".into())
        }));
        let host = session.on_server_line(":carol!c@h CHGHOST newuser new.host");
        assert_eq!(
            host.events,
            vec![Event::HostChange {
                who: "carol".into(),
                address: "newuser@new.host".into()
            }]
        );
        let other = session.on_server_line(":alice!a@h INVITE dave #openirc");
        assert!(matches!(other.events[0], Event::InviteNotify { .. }));
        let gone = session.on_server_line(":irc CAP bob DEL :message-tags");
        assert!(!session.has_cap("message-tags"));
        assert!(gone.outgoing.is_empty());
        assert!(
            session
                .on_user_line("/typing #openirc active")
                .outgoing
                .is_empty()
        );
        let unknown = session.on_server_line(":irc 421 bob CAP :Unknown command");
        assert!(unknown.lines.is_empty());
    }

    #[test]
    fn server_commands_pass_through_and_new_message_commands() {
        let mut session = pump();
        session.on_server_line(":bob!b@h JOIN #openirc");
        session.on_server_line(":bob!b@h JOIN #rust");
        let sent = |session: &mut Pump, line: &str| session.on_user_line(line).outgoing;
        assert_eq!(sent(&mut session, "/lusers"), vec!["LUSERS"]);
        assert_eq!(
            sent(&mut session, "/oper admin secret"),
            vec!["OPER admin secret"]
        );
        assert_eq!(sent(&mut session, "/admin"), vec!["ADMIN"]);
        assert_eq!(
            sent(&mut session, "/admin #rust carol"),
            vec!["MODE #rust +a :carol"]
        );
        assert!(sent(&mut session, "/bad!name").is_empty());
        assert_eq!(
            sent(&mut session, "/join chat,#b key1"),
            vec!["JOIN #chat,#b :key1"]
        );
        assert_eq!(
            sent(&mut session, "/describe #rust waves"),
            vec!["PRIVMSG #rust :\u{1}ACTION waves\u{1}"]
        );
        assert_eq!(
            sent(&mut session, "/amsg hi all"),
            vec!["PRIVMSG #openirc :hi all", "PRIVMSG #rust :hi all"]
        );
        assert_eq!(
            sent(&mut session, "/omsg #rust ops only"),
            vec!["PRIVMSG @#rust :ops only"]
        );
        assert_eq!(
            sent(&mut session, "/onotice heads up"),
            vec!["NOTICE @#chat :heads up"],
            "the current channel when none is named"
        );
        assert_eq!(
            sent(&mut session, "/ctcpreply ann version Mine 1.0"),
            vec!["NOTICE ann :\u{1}VERSION Mine 1.0\u{1}"]
        );
        assert!(sent(&mut session, "/ping ann")[0].starts_with("PRIVMSG ann :\u{1}PING "));
        assert_eq!(
            sent(&mut session, "/cs op #rust"),
            vec!["PRIVMSG ChanServ :op #rust"]
        );
        assert_eq!(
            sent(&mut session, "/partall bye"),
            vec!["PART #openirc,#rust :bye"]
        );
        assert_eq!(
            sent(&mut session, "/say plain"),
            vec!["PRIVMSG #chat :plain"]
        );
    }

    #[test]
    fn join_details_names_and_own_modes() {
        let mut session = pump();
        let joined = session.on_server_line(":bob!b@h JOIN #c");
        assert!(
            joined.outgoing.contains(&"MODE #c".to_string()),
            "asks for the modes"
        );
        session.on_server_line(":irc 353 bob = #c :@bob carol");
        let auto = session.on_server_line(":irc 366 bob #c :End");
        assert!(matches!(
            &auto.events[0],
            Event::Names { listed: false, .. }
        ));
        session.on_user_line("/names #c");
        session.on_server_line(":irc 353 bob = #c :@bob carol");
        let asked = session.on_server_line(":irc 366 bob #c :End");
        assert!(matches!(
            &asked.events[0],
            Event::Names { listed: true, .. }
        ));
        let set = session.on_server_line(":irc 333 bob #c alice!a@h 1700000000");
        match &set.events[0] {
            Event::ChannelInfo { channel, text } => {
                assert_eq!(channel, "#c");
                assert!(
                    text.starts_with("Topic set by alice!a@h on 2023-11-1"),
                    "{text}"
                );
            }
            other => panic!("{other:?}"),
        }
        let created = session.on_server_line(":irc 329 bob #c 1431721185");
        assert!(
            matches!(&created.events[0], Event::ChannelInfo { text, .. } if text.contains("was created on 2015-05-"))
        );
        let modes = session.on_server_line(":irc 324 bob #c +ntk secret");
        let shown: Vec<&String> = modes
            .events
            .iter()
            .filter_map(|event| match event {
                Event::ChannelInfo { text, .. } => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(shown.len(), 1);
        assert!(shown[0].starts_with("#c has modes +"), "{shown:?}");
        assert!(
            shown[0].contains('k') && !shown[0].contains("secret"),
            "the key is not shown"
        );
        let mine = session.on_server_line(":bob MODE bob :+iSx");
        assert!(mine.events.contains(&Event::UserModes {
            modes: "+iSx".into()
        }));
        let less = session.on_server_line(":bob MODE bob :-S");
        assert!(less.events.contains(&Event::UserModes {
            modes: "+ix".into()
        }));
    }

    #[test]
    fn topic_changes_are_tracked() {
        let mut session = pump();
        session.on_server_line(":bob!b@h JOIN #c");
        let changed = session.on_server_line(":ann!a@h TOPIC #c :new words");
        assert_eq!(
            changed.events,
            vec![Event::TopicChanged {
                by: "ann".into(),
                channel: "#c".into(),
                topic: "new words".into()
            }]
        );
        let shown = session.on_user_line("/topic #c");
        assert!(shown.lines[0].contains("new words"));
    }

    #[test]
    fn notify_uses_monitor_or_ison() {
        let mut config = Config::new("bob", "bob", "Bob", Vec::new(), Vec::new());
        config.notify = vec!["Ann".into(), "Cy".into()];
        let mut watched = Pump::new(config.clone());
        watched.on_server_line(":irc 005 bob MONITOR=100 :are supported");
        let motd = watched.on_server_line(":irc 376 bob :End of MOTD");
        assert_eq!(motd.outgoing, vec!["MONITOR + Ann,Cy"]);
        let on = watched.on_server_line(":irc 730 bob :Ann!a@host.example");
        assert_eq!(
            on.events,
            vec![Event::Notify {
                nick: "Ann".into(),
                address: "a@host.example".into(),
                online: true
            }]
        );
        assert!(
            watched.on_server_line(":irc 731 bob :Cy").events.is_empty(),
            "never seen online"
        );
        let off = watched.on_server_line(":irc 731 bob :Ann");
        assert!(matches!(off.events[0], Event::Notify { online: false, .. }));
        let edit = watched.on_user_line("/notifylist Ann,Dee");
        assert_eq!(edit.outgoing, vec!["MONITOR + Dee", "MONITOR - Cy"]);

        let mut polled = Pump::new(config);
        let motd = polled.on_server_line(":irc 422 bob :No MOTD");
        assert_eq!(motd.outgoing, vec!["ISON Ann Cy"]);
        let first = polled.on_server_line(":irc 303 bob :cy");
        assert!(
            matches!(&first.events[0], Event::Notify { nick, online: true, .. } if nick == "Cy")
        );
        assert!(
            polled.on_server_line(":irc 303 bob :cy").events.is_empty(),
            "no change"
        );
        let gone = polled.on_server_line(":irc 303 bob :");
        assert!(matches!(
            &gone.events[0],
            Event::Notify { online: false, .. }
        ));
        assert_eq!(
            polled.on_user_line("/notifypoll").outgoing,
            vec!["ISON Ann Cy"]
        );
    }

    #[test]
    fn flood_command_replaces_the_limits() {
        let mut session = pump();
        let set = session.on_user_line("/flood 1 1 5");
        assert!(set.lines[0].contains("1 messages"));
        session.on_server_line(":alice!a@h PRIVMSG bob :one");
        let tripped = session.on_server_line(":alice!a@h PRIVMSG bob :two");
        assert!(tripped.lines[0].contains("dropping messages"));
        let off = session.on_user_line("/flood 0 0 5");
        assert!(off.lines[0].contains("0 messages"));
        let again = session.on_server_line(":alice!a@h PRIVMSG bob :three");
        assert!(again.lines.iter().any(|line| line.contains("three")));
    }

    #[test]
    fn invite_knock_hop_ame_and_ctcp() {
        let mut session = pump();
        session.on_server_line(":bob!b@h JOIN :#openirc");
        let invite = session.on_user_line("/invite carol");
        assert_eq!(invite.outgoing, vec!["INVITE carol :#openirc"]);
        let knock = session.on_user_line("/knock #locked please");
        assert_eq!(knock.outgoing, vec!["KNOCK #locked :please"]);
        let hop = session.on_user_line("/hop");
        assert_eq!(
            hop.outgoing,
            vec!["PART #openirc :hopping", "JOIN :#openirc"]
        );
        session.on_server_line(":bob!b@h JOIN :#rust");
        let ame = session.on_user_line("/ame waves");
        assert_eq!(ame.outgoing.len(), 2);
        assert!(
            ame.outgoing
                .iter()
                .all(|line| line.contains("ACTION waves"))
        );
        let ctcp = session.on_user_line("/ctcp carol TIME");
        assert_eq!(ctcp.outgoing, vec!["PRIVMSG carol :\u{1}TIME\u{1}"]);
        let time = session.on_server_line(":carol!c@h PRIVMSG bob :\u{1}TIME\u{1}");
        assert!(time.outgoing[0].contains("TIME"));
        let finger = session.on_server_line(":carol!c@h PRIVMSG bob :\u{1}FINGER\u{1}");
        assert!(finger.outgoing[0].contains("FINGER Bob"));
        let echoed = session.on_user_line("/echo hello local");
        assert!(echoed.lines.iter().any(|line| line.contains("hello local")));
        let cleared = session.on_user_line("/clear");
        assert!(cleared.lines.iter().any(|line| line.contains("cleared")));
        let bans = session.on_user_line("/banlist #openirc");
        assert_eq!(bans.outgoing, vec!["MODE #openirc :+b"]);
        let half = session.on_user_line("/halfop #openirc carol");
        assert_eq!(half.outgoing, vec!["MODE #openirc +h :carol"]);
        let key = session.on_user_line("/key secret");
        assert_eq!(key.outgoing, vec!["MODE #rust +k :secret"]);
        let was = session.on_user_line("/whowas carol");
        assert_eq!(was.outgoing, vec!["WHOWAS :carol"]);
        let mut secure = pump();
        secure.config.sasl_password = "secret".into();
        let lines = secure.registration_lines();
        assert_eq!(lines[0], "CAP LS 302");
        let ls = secure.on_server_line(":irc CAP * LS :sasl=PLAIN,EXTERNAL");
        assert_eq!(ls.outgoing, vec!["CAP REQ :sasl"]);
        let ack = secure.on_server_line(":irc CAP bob ACK :sasl");
        assert_eq!(ack.outgoing, vec!["AUTHENTICATE PLAIN"]);
        let plus = secure.on_server_line(":irc AUTHENTICATE +");
        assert!(plus.outgoing[0].starts_with("AUTHENTICATE "));
    }

    #[test]
    fn sasl_nak_ends_cap_and_the_account_is_not_the_retry_nick() {
        let mut config = Config::new("bob", "bob", "Bob", Vec::new(), Vec::new());
        config.sasl_password = "secret".into();
        let mut refused = Pump::new(config.clone());
        let nak = refused.on_server_line(":irc CAP * NAK :sasl");
        assert_eq!(nak.outgoing, vec!["CAP END"]);

        let mut session = Pump::new(config);
        session.on_server_line(":irc 433 * bob :Nickname is already in use");
        assert_eq!(session.nick(), "bob_");
        session.on_server_line(":irc CAP bob_ ACK :sasl");
        let plus = session.on_server_line(":irc AUTHENTICATE +");
        assert_eq!(
            plus.outgoing,
            vec![format!(
                "AUTHENTICATE {}",
                crate::socks::base64(b"bob\0bob\0secret")
            )]
        );
    }

    #[test]
    fn alt_nick_quit_message_finger_invite_and_network() {
        let mut config = Config::new("bob", "bob", "Bob", Vec::new(), Vec::new());
        config.alt_nick = "robert".into();
        config.quit_message = "see you".into();
        config.finger_reply = "busy".into();
        let mut session = Pump::new(config);
        let taken = session.on_server_line(":irc 433 * bob :Nickname is already in use");
        assert_eq!(taken.outgoing, vec!["NICK :robert"]);
        let again = session.on_server_line(":irc 433 * robert :Nickname is already in use");
        assert_eq!(again.outgoing, vec!["NICK :robert_"]);
        let quit = session.on_user_line("/quit");
        assert_eq!(quit.outgoing, vec!["QUIT :see you"]);
        let finger = session.on_server_line(":carol!c@h PRIVMSG robert_ :\u{1}FINGER\u{1}");
        assert!(finger.outgoing[0].contains("FINGER busy"));
        let invite = session.on_server_line(":carol!c@h INVITE robert_ :#rust");
        assert_eq!(
            invite.events,
            vec![Event::Invite {
                from: "carol".into(),
                channel: "#rust".into(),
            }]
        );
        let support =
            session.on_server_line(":irc 005 robert_ NETWORK=Example CHANTYPES=# :are supported");
        assert!(support.events.contains(&Event::Network {
            name: "Example".into()
        }));
    }

    #[test]
    fn mode_changes_report_members_and_channel_flags() {
        let mut session = pump();
        session.on_server_line(":bob!b@h JOIN :#c");
        session.on_server_line(":irc 353 bob = #c :bob alice");
        session.on_server_line(":irc 366 bob #c :End");
        let reply = session.on_server_line(":irc 324 bob #c +nt");
        assert!(reply.events.contains(&Event::ChannelModes {
            channel: "#c".into(),
            modes: "+nt".into(),
        }));
        let op = session.on_server_line(":irc MODE #c +ok alice secret");
        assert!(op.events.iter().any(|event| matches!(
            event,
            Event::Members { names, .. } if names.contains(&"@alice".to_string())
        )));
        assert!(op.events.contains(&Event::ChannelModes {
            channel: "#c".into(),
            modes: "+knt secret".into(),
        }));
        let off = session.on_server_line(":irc MODE #c -k secret");
        assert!(off.events.contains(&Event::ChannelModes {
            channel: "#c".into(),
            modes: "+nt".into(),
        }));
    }

    #[test]
    fn server_password_external_sasl_and_nickserv_login() {
        use crate::servers::Login;
        let mut config = Config::new("bob", "bob", "Bob", Vec::new(), Vec::new());
        config.server_password = "letmein".into();
        config.login = Login::SaslExternal;
        let mut session = Pump::new(config.clone());
        let lines = session.registration_lines();
        assert_eq!(lines[0], "PASS :letmein");
        assert_eq!(lines[1], "CAP LS 302");
        session.on_server_line(":irc CAP * LS :sasl");
        let ack = session.on_server_line(":irc CAP bob ACK :sasl");
        assert_eq!(ack.outgoing, vec!["AUTHENTICATE EXTERNAL"]);
        let plus = session.on_server_line(":irc AUTHENTICATE +");
        assert_eq!(plus.outgoing, vec!["AUTHENTICATE +"]);
        let done = session.on_server_line(":irc 903 bob :SASL authentication successful");
        assert_eq!(done.outgoing, vec!["CAP END"]);

        let mut nickserv = config;
        nickserv.server_password.clear();
        nickserv.login = Login::NickServ;
        nickserv.sasl_user = "bobby".into();
        nickserv.sasl_password = "pw".into();
        let mut session = Pump::new(nickserv);
        assert!(
            !session
                .registration_lines()
                .iter()
                .any(|line| line.starts_with("CAP REQ"))
        );
        let ls = session.on_server_line(":irc CAP * LS :sasl");
        assert_eq!(
            ls.outgoing,
            vec!["CAP END"],
            "NickServ login does not ask for SASL"
        );
        let welcome = session.on_server_line(":irc 001 bob :Welcome");
        assert!(
            welcome
                .outgoing
                .contains(&"PRIVMSG NickServ :IDENTIFY bobby pw".to_string())
        );
    }

    #[test]
    fn own_joins_are_not_flood_limited() {
        let mut session = pump();
        for _ in 0..12 {
            let effect = session.on_server_line(":bob!b@h JOIN :#openirc");
            assert!(effect.lines.iter().any(|line| line.contains("joined")));
        }
    }
}
