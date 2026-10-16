//! Typed commands the window handles itself: connections (`/server`,
//! `/disconnect`, `/scon`), windows (`/close`, `/list`, `/url`, `/clearall`,
//! `/savebuf`), settings (`/notify`, `/log`, `/alias`, `/fullname`,
//! `/anick`), user levels, scripts (`/load`, `/unload`, `/timers`), and
//! small helpers (`/beep`, `/flash`, `/tip`, `/run`, `/dns`, `/editbox`,
//! `/play`). Everything else goes to the session and then the server.

use std::time::Duration;

use super::{Alias, Kind, OpenIrcApp, Origin, UiTimer, note, same_name};
use crate::servers::ServerEntry;

/// Lines `/play` sends at most, so a wrong file cannot flood a channel.
const PLAY_LIMIT: usize = 500;

/// What `/server` was asked to do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ServerArgs {
    /// `-m`: a new connection beside the current ones.
    pub new: bool,
    pub host: Option<String>,
    pub port: Option<u16>,
    /// `-e` or a `+port`.
    pub tls: bool,
    pub password: Option<String>,
    /// `-j #a,#b`.
    pub channels: Option<String>,
}

/// Parse `/server [-m] [-e] [-p password] [-j #channels] [host[:port]] [[+]port] [password]`.
pub(super) fn parse_server(rest: &str) -> Result<ServerArgs, String> {
    let mut args = ServerArgs::default();
    let mut words = rest.split_whitespace().peekable();
    let mut plain = Vec::new();
    while let Some(word) = words.next() {
        match word {
            "-m" => args.new = true,
            "-e" => args.tls = true,
            "-p" => args.password = words.next().map(str::to_string),
            "-j" => args.channels = words.next().map(str::to_string),
            flags if flags.starts_with('-') && flags.len() > 1 && plain.is_empty() => {
                for flag in flags[1..].chars() {
                    match flag {
                        'm' => args.new = true,
                        'e' => args.tls = true,
                        _ => return Err(format!("unknown /server option -{flag}")),
                    }
                }
            }
            other => plain.push(other.to_string()),
        }
    }
    let mut plain = plain.into_iter();
    if let Some(host) = plain.next() {
        let (host, port) = match host.rsplit_once(':') {
            // An IPv6 address has several colons and takes its port separately.
            Some((name, port)) if !name.contains(':') => (name.to_string(), Some(port.to_string())),
            _ => (host, None),
        };
        args.host = Some(host);
        if let Some(port) = port.or_else(|| plain.next()) {
            let digits = match port.strip_prefix('+') {
                Some(digits) => {
                    args.tls = true;
                    digits
                }
                None => port.as_str(),
            };
            args.port = Some(
                digits
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port > 0)
                    .ok_or_else(|| format!("`{port}` is not a port"))?,
            );
        }
        if let Some(password) = plain.next() {
            args.password = Some(password);
        }
    }
    Ok(args)
}

impl OpenIrcApp {
    /// Run a window command. Returns false when `name` is not one, so the
    /// line goes on to the session.
    pub(super) fn local_command(&mut self, name: &str, rest: &str) -> bool {
        match name.to_ascii_lowercase().as_str() {
            "server" => self.server_command(rest),
            "disconnect" => self.disconnect(),
            "exit" => self.exit_requested = true,
            "scon" | "scid" => self.on_every_server(rest),
            "list" => {
                let server = self.scope();
                self.show_channel_list(server, false);
                self.channel_list.clear();
                self.channel_list.listing = true;
                let line = if rest.is_empty() {
                    "/raw LIST".to_string()
                } else {
                    format!("/raw LIST {rest}")
                };
                self.send_user(line);
            }
            "url" | "urls" => {
                let index = self.ensure(Kind::List, "URLs");
                self.activate(index);
            }
            "close" => self.close_command(rest),
            "closemsg" => match rest.split_whitespace().next() {
                Some(nick) => {
                    if let Some(index) = self.find(Kind::Query, nick) {
                        self.close_window(index);
                    }
                }
                None => self.note_active("usage: /closemsg <nick>".into()),
            },
            "clearall" => {
                for window in &mut self.windows {
                    window.lines.clear();
                    window.marker = None;
                    window.generation += 1;
                }
                self.find_cache = None;
            }
            "savebuf" => self.save_buffer(rest),
            "notify" => self.notify_command(rest),
            "log" => match rest.to_ascii_lowercase().as_str() {
                "on" | "off" => {
                    self.settings.logging = rest.eq_ignore_ascii_case("on");
                    self.persist();
                    self.note_active(format!(
                        "Logging is {}.",
                        if self.settings.logging { "on" } else { "off" }
                    ));
                }
                _ => self.note_active(format!(
                    "Logging is {}; /log on or /log off changes it.",
                    if self.settings.logging { "on" } else { "off" }
                )),
            },
            "alias" => self.alias_command(rest),
            "timers" => self.timers_command(rest),
            "fullname" => {
                if rest.is_empty() {
                    self.note_active(format!("Real name: {}", self.settings.realname));
                } else {
                    self.settings.realname = rest.to_string();
                    self.persist();
                    self.note_active("Real name saved; it is sent on the next connect.".into());
                }
            }
            "anick" => {
                if rest.is_empty() {
                    self.note_active(format!("Alternative nick: {}", self.settings.alt_nick));
                } else {
                    self.settings.alt_nick = rest.split_whitespace().next().unwrap_or("").into();
                    self.persist();
                    self.note_active(format!("Alternative nick is {}.", self.settings.alt_nick));
                }
            }
            "auser" => {
                let mut words = rest.split_whitespace();
                match (words.next(), words.next()) {
                    (Some(levels), Some(mask)) => {
                        self.settings
                            .users
                            .retain(|(_, item)| !item.eq_ignore_ascii_case(mask));
                        self.settings
                            .users
                            .push((levels.to_string(), mask.to_string()));
                        self.persist();
                        self.note_active(format!("{mask} has levels {levels}."));
                    }
                    _ => self.note_active("usage: /auser <levels> <nick or mask>".into()),
                }
            }
            "ruser" => match rest.split_whitespace().next() {
                Some(mask) => {
                    let before = self.settings.users.len();
                    self.settings
                        .users
                        .retain(|(_, item)| !item.eq_ignore_ascii_case(mask));
                    self.persist();
                    self.note_active(if before == self.settings.users.len() {
                        format!("{mask} is not in the user list.")
                    } else {
                        format!("Removed {mask} from the user list.")
                    });
                }
                None => self.note_active("usage: /ruser <nick or mask>".into()),
            },
            "ulist" => {
                let users = self.settings.users.clone();
                if users.is_empty() {
                    self.note_active(format!(
                        "No user levels; everyone has level {}.",
                        self.settings.default_level
                    ));
                }
                for (levels, mask) in users {
                    self.note_active(format!("{levels}: {mask}"));
                }
            }
            "load" => self.load_script(rest),
            "unload" => {
                let name = rest.trim();
                let before = self.settings.scripts.len();
                self.settings
                    .scripts
                    .retain(|file| !file.name.eq_ignore_ascii_case(name));
                if before == self.settings.scripts.len() {
                    self.note_active(format!("No script file named {name}."));
                } else {
                    self.settings.rebuild_script();
                    self.persist();
                    self.note_active(format!("Unloaded {name}."));
                }
            }
            "beep" => crate::beep::play(),
            "flash" => {
                self.flash = true;
                if !rest.is_empty() {
                    self.tip("OpenIRC", rest.to_string(), None);
                }
            }
            "tip" => {
                if rest.is_empty() {
                    self.note_active("usage: /tip <text>".into());
                } else {
                    self.tip("OpenIRC", rest.to_string(), None);
                }
            }
            "run" => {
                if rest.is_empty() {
                    self.note_active("usage: /run <web address or file>".into());
                } else {
                    super::open_url(rest.trim().trim_matches('"'));
                }
            }
            "dns" => self.dns_command(rest),
            "editbox" => {
                // A new draft, for whoever is there now.
                self.input = rest.to_string();
                self.draft_for = None;
                self.note_draft();
            }
            "play" => self.play_file(rest),
            "ban" if rest.starts_with("-u") => self.timed_ban(rest),
            _ => return false,
        }
        true
    }

    fn server_command(&mut self, rest: &str) {
        let args = match parse_server(rest) {
            Ok(args) => args,
            Err(err) => {
                self.note_active(format!(
                    "{err}. Usage: /server [-m] [-e] [-p password] [-j #channels] host [[+]port] [password]"
                ));
                return;
            }
        };
        let current = self.scope();
        let Some(host) = args.host.clone() else {
            if args.new {
                self.note_active("usage: /server -m <host> [port]".into());
                return;
            }
            // No server: connect the current one again.
            match current {
                Some(id) => {
                    if let Some(session) = self.sessions.iter_mut().find(|session| session.id == id)
                    {
                        session.hangup = false;
                        session.attempts = 0;
                        if let Some(link) = session.link.take() {
                            let _ = link.outgoing.send("/quit".into());
                        }
                    }
                    self.reconnect(id);
                }
                None => self.connect(),
            }
            return;
        };
        let mut entry = self.entry_for(&host, args.port, args.tls);
        if let Some(port) = args.port {
            // A saved entry for the same host on another port keeps its
            // login, but its TLS setting belongs to its own port.
            if port != entry.port {
                entry.tls = args.tls;
            }
            entry.port = port;
        }
        if args.tls {
            entry.tls = true;
        }
        if let Some(password) = args.password {
            entry.password = password;
        }
        if let Some(channels) = args.channels {
            entry.channels = channels.replace(',', " ");
        }
        match current.filter(|_| !args.new) {
            Some(id) => self.switch_server(id, entry),
            None => self.open_server(entry),
        }
    }

    /// A saved server matching `host` (by address, or by network name),
    /// else a new entry with the usual port.
    fn entry_for(&self, host: &str, port: Option<u16>, tls: bool) -> ServerEntry {
        let saved = &self.settings.servers;
        saved
            .iter()
            .find(|entry| {
                same_name(&entry.host, host) && port.is_none_or(|port| port == entry.port)
            })
            .or_else(|| saved.iter().find(|entry| same_name(&entry.host, host)))
            .or_else(|| saved.iter().find(|entry| same_name(&entry.network, host)))
            .cloned()
            .unwrap_or_else(|| {
                let port = port.unwrap_or(if tls { 6697 } else { 6667 });
                ServerEntry::new("", host, port, tls || port == 6697)
            })
    }

    /// Run a command once for every connected server (`/scon -a`).
    fn on_every_server(&mut self, rest: &str) {
        let Some(command) = rest
            .strip_prefix("-a")
            .map(str::trim)
            .filter(|cmd| !cmd.is_empty())
        else {
            self.note_active("usage: /scon -a <command>".into());
            return;
        };
        let command = if command.starts_with('/') {
            command.to_string()
        } else {
            format!("/{command}")
        };
        let connected: Vec<u64> = self
            .sessions
            .iter()
            .filter(|session| session.link.is_some())
            .map(|session| session.id)
            .collect();
        // Each server's own target, not the window a timer running this was
        // set in; the line the timer came from still answers for it.
        let window = self.context.take();
        let chat = self.context_chat.take();
        for id in connected {
            // Names compare each server's way, not the front one's.
            self.on_server(Some(id), |app| {
                app.dispatch(command.clone(), Origin::Script)
            });
        }
        self.context = window;
        self.context_chat = chat;
    }

    fn close_command(&mut self, rest: &str) {
        let rest = rest.trim();
        if rest.is_empty() {
            let active = self.active;
            self.close_window(active);
            return;
        }
        if rest == "-m" {
            // Every query window.
            while let Some(index) = self
                .windows
                .iter()
                .position(|window| window.kind == Kind::Query)
            {
                self.close_window(index);
            }
            return;
        }
        let index = self
            .windows
            .iter()
            .position(|window| window.kind != Kind::Status && same_name(&window.title, rest));
        match index {
            Some(index) => self.close_window(index),
            None => self.note_active(format!("No window named {rest}.")),
        }
    }

    /// Save the active window's lines as text.
    fn save_buffer(&mut self, rest: &str) {
        let Some(window) = self.windows.get(self.active) else {
            return;
        };
        let name = if rest.trim().is_empty() {
            format!("{}.txt", window.title.trim_start_matches(['#', '&']))
        } else {
            rest.trim().trim_matches('"').to_string()
        };
        let path = if std::path::Path::new(&name).is_absolute() {
            std::path::PathBuf::from(&name)
        } else {
            crate::settings::config_dir().join(&name)
        };
        let text: String = window
            .lines
            .iter()
            .map(|line| {
                let stamp = line
                    .time
                    .map(|time| format!("[{:02}:{:02}] ", time.hour, time.minute))
                    .unwrap_or_default();
                format!("{stamp}{}\n", line.plain())
            })
            .collect();
        let count = window.lines.len();
        match std::fs::write(&path, text) {
            Ok(()) => self.note_active(format!("Saved {count} lines to {}.", path.display())),
            Err(err) => self.note_active(format!("Could not save {}: {err}", path.display())),
        }
    }

    fn notify_command(&mut self, rest: &str) {
        let mut words = rest.split_whitespace();
        match words.next() {
            None => {
                if self.settings.notify.is_empty() {
                    self.note_active("The notify list is empty.".into());
                } else {
                    let list = self.settings.notify.join(", ");
                    self.note_active(format!("Notify list: {list}"));
                }
            }
            Some("-r") => {
                let Some(nick) = words.next() else {
                    self.note_active("usage: /notify -r <nick>".into());
                    return;
                };
                self.settings
                    .notify
                    .retain(|item| !item.eq_ignore_ascii_case(nick));
                self.persist();
                self.sync_notify();
                self.note_active(format!("Removed {nick} from the notify list."));
            }
            Some(nick) => {
                if !self
                    .settings
                    .notify
                    .iter()
                    .any(|item| item.eq_ignore_ascii_case(nick))
                {
                    self.settings.notify.push(nick.to_string());
                    self.persist();
                }
                self.sync_notify();
                self.note_active(format!("Added {nick} to the notify list."));
            }
        }
    }

    /// `/alias name command` adds or replaces a quick alias; `/alias name`
    /// removes it; `/alias` lists them.
    fn alias_command(&mut self, rest: &str) {
        let rest = rest.trim();
        if rest.is_empty() {
            let aliases = self.settings.aliases.clone();
            if aliases.is_empty() {
                self.note_active("No quick aliases.".into());
            }
            for alias in aliases {
                self.note_active(format!("/{} = {}", alias.name, alias.body));
            }
            return;
        }
        let (name, body) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let name = name.trim_start_matches('/').to_string();
        self.settings
            .aliases
            .retain(|alias| !alias.name.eq_ignore_ascii_case(&name));
        if body.trim().is_empty() {
            self.note_active(format!("Removed the alias /{name}."));
        } else {
            self.settings.aliases.push(Alias {
                name: name.clone(),
                body: body.trim().to_string(),
            });
            self.note_active(format!("/{name} = {}", body.trim()));
        }
        self.persist();
    }

    fn timers_command(&mut self, rest: &str) {
        if rest.trim().eq_ignore_ascii_case("off") {
            let count = self.timers.len();
            self.timers.clear();
            self.note_active(format!(
                "Stopped {}.",
                super::plural(count, "{0} timer", "{0} timers")
            ));
            return;
        }
        if self.timers.is_empty() {
            self.note_active("No timers are running.".into());
            return;
        }
        let lines: Vec<String> = self
            .timers
            .iter()
            .enumerate()
            .map(|(index, timer)| {
                format!(
                    "Timer {}: every {}s, {} left: {}",
                    index + 1,
                    timer.every.as_secs(),
                    timer.left,
                    timer.command
                )
            })
            .collect();
        for line in lines {
            self.note_active(line);
        }
    }

    /// Import a script file as a new script in the editor's list.
    fn load_script(&mut self, rest: &str) {
        let path = rest.trim().trim_matches('"');
        if path.is_empty() {
            self.note_active("usage: /load <script file>".into());
            return;
        }
        let full = if std::path::Path::new(path).is_absolute() {
            std::path::PathBuf::from(path)
        } else {
            crate::settings::config_dir().join(path)
        };
        let text = match std::fs::read(&full) {
            Ok(bytes) => crate::message::decode_line(&bytes),
            Err(err) => {
                self.note_active(format!("Could not read {}: {err}", full.display()));
                return;
            }
        };
        if let Err(err) = crate::slang::check(&text) {
            self.note_active(format!("Not loaded: {err}"));
            return;
        }
        let name = full
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "script".into());
        if self.settings.scripts.is_empty() && !self.settings.script_text.trim().is_empty() {
            self.settings.scripts.push(crate::settings::ScriptFile {
                name: "main".into(),
                text: self.settings.script_text.clone(),
                enabled: true,
            });
        }
        self.settings.scripts.retain(|file| file.name != name);
        self.settings.scripts.push(crate::settings::ScriptFile {
            name: name.clone(),
            text,
            enabled: true,
        });
        self.settings.rebuild_script();
        self.persist();
        self.note_active(format!("Loaded {name}; Tools, Scripts editor shows it."));
    }

    fn dns_command(&mut self, rest: &str) {
        let Some(name) = rest.split_whitespace().next().map(str::to_string) else {
            self.note_active("usage: /dns <host or nick>".into());
            return;
        };
        // A nick seen in a channel resolves its host.
        let host = self
            .addresses
            .get(&(self.scope(), super::nick_key(&name)))
            .and_then(|address| address.split_once('@').map(|(_, host)| host.to_string()))
            .unwrap_or_else(|| name.clone());
        self.note_active(format!("Looking up {host}..."));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::net::ToSocketAddrs;
            let text = match (host.as_str(), 0).to_socket_addrs() {
                Ok(addrs) => {
                    let mut ips: Vec<String> = addrs.map(|addr| addr.ip().to_string()).collect();
                    ips.dedup();
                    format!("{host} is {}", ips.join(", "))
                }
                Err(err) => format!("{host} could not be looked up: {err}"),
            };
            let _ = tx.send(text);
        });
        self.lookups.push(rx);
    }

    /// Notes from finished `/dns` lookups.
    pub(super) fn poll_lookups(&mut self) {
        let mut done = Vec::new();
        self.lookups.retain(|rx| match rx.try_recv() {
            Ok(text) => {
                done.push(text);
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => false,
        });
        for text in done {
            self.note_active(text);
        }
    }

    /// Say each line of a text file in the active window; the send queue
    /// paces them.
    fn play_file(&mut self, rest: &str) {
        let path = rest.trim().trim_matches('"');
        if path.is_empty() {
            self.note_active("usage: /play <text file>".into());
            return;
        }
        let full = if std::path::Path::new(path).is_absolute() {
            std::path::PathBuf::from(path)
        } else {
            crate::settings::config_dir().join(path)
        };
        let text = match std::fs::read(&full) {
            Ok(bytes) => crate::message::decode_line(&bytes),
            Err(err) => {
                self.note_active(format!("Could not read {}: {err}", full.display()));
                return;
            }
        };
        let lines: Vec<String> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .take(PLAY_LIMIT)
            .map(str::to_string)
            .collect();
        for line in lines {
            // Plain text, never commands, even when a line starts with /.
            self.say(line);
        }
    }

    /// `/ban -u<seconds> [#channel] <nick>`: ban now, lift it later.
    fn timed_ban(&mut self, rest: &str) {
        let (flag, target) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let Some(seconds) = flag[2..].parse::<u64>().ok().filter(|secs| *secs > 0) else {
            self.note_active("usage: /ban -u<seconds> [#channel] <nick>".into());
            return;
        };
        let target = target.trim();
        if target.is_empty() {
            self.note_active("usage: /ban -u<seconds> [#channel] <nick>".into());
            return;
        }
        // Name the channel now, so the ban is lifted where it was set even if
        // another window is in front by then.
        let Some(next) = super::later(Duration::from_secs(seconds)) else {
            self.note_active("A ban can be lifted at most a year later.".into());
            return;
        };
        let target = match self.script_window().map(|index| &self.windows[index]) {
            Some(window)
                if window.kind == Kind::Channel && !target.starts_with(['#', '&', '+', '!']) =>
            {
                format!("{} {target}", window.title)
            }
            _ => target.to_string(),
        };
        self.dispatch(format!("/ban {target}"), Origin::Script);
        self.timers.push(UiTimer {
            next,
            every: Duration::from_secs(seconds),
            left: 1,
            command: format!("/unban {target}"),
            server: self.scope(),
            window: None,
            origin: self.provenance,
            chat: None,
            reply: None,
        });
        self.note_active(format!("The ban on {target} is lifted in {seconds}s."));
    }

    /// Point an existing connection at another server: the old one quits,
    /// its channel and query windows close, and the status window stays.
    fn switch_server(&mut self, id: u64, entry: ServerEntry) {
        let closing: Vec<String> = self
            .windows
            .iter()
            .filter(|window| {
                window.server == Some(id) && matches!(window.kind, Kind::Channel | Kind::Query)
            })
            .map(|window| window.title.clone())
            .collect();
        if let Some(session) = self.sessions.iter_mut().find(|session| session.id == id) {
            if let Some(link) = session.link.take() {
                let _ = link.outgoing.send("/quit".into());
            }
            session.hangup = false;
            session.reconnect_at = None;
            session.attempts = 0;
            session.host = entry.host.clone();
            session.port = entry.port;
            session.tls = entry.tls;
            session.autojoin = entry.channel_list();
            session.network =
                Some(entry.network.clone()).filter(|network| !network.trim().is_empty());
            session.connected_at = None;
            session.away = None;
            session.caps.clear();
            session.away_nicks.clear();
            session.accounts.clear();
            session.umodes.clear();
            session.entry = entry.clone();
        }
        let active_title = self
            .windows
            .get(self.active)
            .map(|window| window.title.clone());
        self.windows.retain(|window| {
            !(window.server == Some(id) && matches!(window.kind, Kind::Channel | Kind::Query))
        });
        let status = self
            .windows
            .iter()
            .position(|window| window.server == Some(id) && window.kind == Kind::Status);
        if let Some(index) = status {
            self.windows[index].title = if entry.network.trim().is_empty() {
                entry.host.clone()
            } else {
                entry.network.clone()
            };
            if !closing.is_empty() {
                let list = closing.join(", ");
                self.windows[index]
                    .lines
                    .push(note(format!("Closed {list} (the old server's windows).")));
            }
        }
        self.active = active_title
            .and_then(|title| self.windows.iter().position(|window| window.title == title))
            .or(status)
            .unwrap_or(0)
            .min(self.windows.len().saturating_sub(1));
        self.find_cache = None;
        self.reconnect(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_arguments() {
        assert_eq!(parse_server("").unwrap(), ServerArgs::default());
        let args = parse_server("-m irc.example.net +6697").unwrap();
        assert!(args.new && args.tls);
        assert_eq!(args.host.as_deref(), Some("irc.example.net"));
        assert_eq!(args.port, Some(6697));
        let args = parse_server("-e -p secret -j #a,#b irc.example:7000").unwrap();
        assert!(!args.new && args.tls);
        assert_eq!(args.port, Some(7000));
        assert_eq!(args.password.as_deref(), Some("secret"));
        assert_eq!(args.channels.as_deref(), Some("#a,#b"));
        let args = parse_server("-me irc.example 6667 pass").unwrap();
        assert!(args.new && args.tls);
        assert_eq!(args.password.as_deref(), Some("pass"));
        let args = parse_server("2001:db8::1 6697").unwrap();
        assert_eq!(args.host.as_deref(), Some("2001:db8::1"));
        assert_eq!(args.port, Some(6697));
        assert!(parse_server("irc.example notaport").is_err());
        assert!(parse_server("-x irc.example").is_err());
    }
}
