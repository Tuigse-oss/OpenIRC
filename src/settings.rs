//! Saved OpenIRC settings: connection, favorites, notify, aliases, and logging.
//!
//! The file is a small line format under the user's config directory so the
//! window can reopen with the same servers and lists.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Favorite {
    pub name: String,
    pub server: String,
    pub port: u16,
    pub tls: bool,
    pub channels: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    pub name: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Perform {
    pub server: String,
    pub command: String,
}

pub const SOUND_EVENTS: [&str; 9] = [
    "message",
    "action",
    "notice",
    "join",
    "part",
    "quit",
    "kick",
    "highlight",
    "dcc",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventSounds {
    paths: Vec<(String, String)>,
}

impl EventSounds {
    pub fn new() -> Self {
        Self { paths: Vec::new() }
    }

    pub fn path(&self, event: &str) -> &str {
        self.paths
            .iter()
            .find(|(name, _)| name == event)
            .map(|(_, path)| path.as_str())
            .unwrap_or("")
    }

    pub fn set(&mut self, event: &str, path: String) {
        if !SOUND_EVENTS.contains(&event) {
            return;
        }
        if let Some(slot) = self.paths.iter_mut().find(|(name, _)| name == event) {
            slot.1 = path;
        } else {
            self.paths.push((event.to_string(), path));
        }
    }
}

impl Default for EventSounds {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Popup {
    pub label: String,
    pub body: String,
    /// Which right-click menu it is in: `line` (a transcript line),
    /// `nicklist`, `channel`, `query`, or `status`.
    pub menu: String,
}

/// The right-click menus a popup can be in, with names for people.
use crate::i18n::tr_key;

pub const POPUP_MENUS: [(&str, &str); 5] = [
    ("line", tr_key("Transcript line")),
    ("nicklist", tr_key("Nick list")),
    ("channel", tr_key("Channel tab")),
    ("query", tr_key("Query tab")),
    ("status", tr_key("Status tab")),
];

/// One script file in the script editor. Enabled files run together, in
/// list order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptFile {
    pub name: String,
    pub text: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub server: String,
    pub port: u16,
    pub tls: bool,
    pub nick: String,
    pub user: String,
    pub realname: String,
    pub autojoin: String,
    pub show_joins: bool,
    pub timestamps: bool,
    pub reconnect: bool,
    pub logging: bool,
    pub log_dir: String,
    /// Keep color and style codes in logs.
    pub log_keep_codes: bool,
    /// Start a new log file every `day` or `month`; `none` keeps one file.
    pub log_dated: String,
    /// Keep each network's logs in its own folder.
    pub log_by_network: bool,
    /// Trim a log that grows past this many KiB; 0 never trims.
    pub log_max_kb: u32,
    /// Lines of the log to show when a channel or query window opens.
    pub log_reload: u32,
    pub notify: Vec<String>,
    /// Ignore rules: who, what kinds, where, and until when.
    pub ignores: Vec<crate::rules::IgnoreRule>,
    pub beep: bool,
    pub sounds: bool,
    pub theme: crate::theme::Theme,
    /// The 16 message colors.
    pub palette: crate::theme::Palette,
    pub ident: bool,
    pub ident_user: String,
    pub event_sounds: EventSounds,
    /// Highlight rules.
    pub highlights: Vec<crate::rules::HighlightRule>,
    /// Highlight lines that mention your own nick.
    pub highlight_nick: bool,
    /// Nickname color rules, checked before the automatic colors.
    pub nick_colors: Vec<crate::rules::NickColorRule>,
    /// Color nicks automatically when no rule matches.
    pub auto_nick_colors: bool,
    /// Give these people a channel status when they join.
    pub auto_modes: Vec<crate::rules::AutoMode>,
    /// Wait a few random seconds before an automatic mode, so several
    /// operators do not all set it at once.
    pub auto_mode_delay: bool,
    pub perform: Vec<Perform>,
    pub flood_messages: u32,
    pub flood_joins: u32,
    pub flood_window_secs: u64,
    /// CTCP requests answered per flood window, from everyone together.
    pub flood_ctcp: u32,
    /// New query windows other people may open per flood window; later
    /// ones go to the Messages window. 0 has no limit.
    pub query_limit: u32,
    /// Show these in the active window instead of the status window.
    pub active_notices: bool,
    pub active_ctcp: bool,
    pub active_invites: bool,
    pub active_whois: bool,
    pub active_away: bool,
    /// Put private messages from people without an open query in one
    /// Messages window instead of opening a query for each.
    pub message_window: bool,
    /// Grow the input into several lines; Enter sends, Shift+Enter starts
    /// a new line.
    pub multiline_input: bool,
    /// Character that starts a typed command, besides `/`.
    pub command_prefix: String,
    /// Separates commands in aliases, popups, scripts, and dialog buttons.
    pub separator: String,
    /// Commands run on a double-click: on a nick in the nick list, and in
    /// a channel, query, or status window. `$nick` and `$chan` are filled.
    pub dclick_nick: String,
    pub dclick_channel: String,
    pub dclick_query: String,
    pub dclick_status: String,
    /// Command run for each file dropped on a window; `$file` is the path.
    pub drop_command: String,
    /// User levels for script events: levels (separated by commas) and the
    /// nick or mask they belong to.
    pub users: Vec<(String, String)>,
    /// The level of everyone not in `users`.
    pub default_level: String,
    /// Where received files go.
    pub dcc_folder: String,
    /// Folders by file type: extensions separated by spaces, and a folder
    /// (relative ones are inside `dcc_folder`).
    pub dcc_folder_rules: Vec<(String, String)>,
    /// Accept files from these nicks or masks without asking.
    pub dcc_auto_accept: Vec<String>,
    /// File extensions never accepted, separated by spaces.
    pub dcc_ignore_types: String,
    /// When an accepted file already exists: ask, resume, overwrite, or
    /// rename.
    pub dcc_exists: String,
    /// Reopen the channels and queries of the last session on connect.
    pub restore_windows: bool,
    /// The channels and queries open at the end of the last session, by
    /// server address.
    pub last_windows: Vec<(String, Vec<String>, Vec<String>)>,
    /// Set away after this many idle minutes; 0 never does.
    pub auto_away_minutes: u32,
    /// The reason used for the automatic away.
    pub auto_away_reason: String,
    /// Come back from the automatic away when you type again.
    pub auto_back: bool,
    /// Color the whole message in the speaker's nick color.
    pub nick_color_line: bool,
    /// Title bar text; `$me`, `$network`, and `$window` are filled in.
    /// Blank uses the built-in title.
    pub title_text: String,
    /// Leave out the `@` and `+` badges in the nick list.
    pub nicklist_hide_badges: bool,
    /// Leave out the `#` before channel names in the treebar.
    pub tree_hide_hash: bool,
    /// Keep the computer awake while a file transfer runs.
    pub dcc_keep_awake: bool,
    /// Toolbar buttons left out, by icon name.
    pub toolbar_hidden: Vec<String>,
    /// Keep typed lines between sessions (never ones that may hold a
    /// password).
    pub keep_history: bool,
    /// Typed lines saved from the last session, oldest first.
    pub history: Vec<String>,
    /// Open the channels list when a server finishes connecting.
    pub channels_on_connect: bool,
    /// Main window position and size (x, y, width, height) when it closed.
    pub window_place: Option<[i32; 4]>,
    /// The main window was maximized when it closed.
    pub window_maximized: bool,
    /// Mark unknown words in the input line as they are typed.
    pub spell_input: bool,
    /// Show the speaker's channel rank (`@`, `+`) before the nick.
    pub nick_rank: bool,
    /// Text around the nick on chat lines, before and after.
    pub nick_open: String,
    pub nick_close: String,
    /// Language of the menus and dialogs: a code such as `de`, `en` for
    /// English, or empty to follow the system.
    pub language: String,
    /// Web search address; `%s` is replaced by the words (or they are added
    /// at the end).
    pub search_url: String,
    /// Show the monitor panel along the bottom.
    pub monitor_panel: bool,
    /// Windows shown in a fixed-width font, by title.
    pub mono_windows: Vec<String>,
    /// Order of windows in the switchbar and tree: `opened` or `name`.
    pub window_order: String,
    /// Hide the window when minimized; the tray icon shows it again.
    pub minimize_to_tray: bool,
    /// Blink the tray icon while a highlight or private message waits.
    pub tray_blink: bool,
    /// Negotiate IRCv3 capabilities when connecting.
    pub ircv3: bool,
    /// Show who is typing (IRCv3 typing notifications).
    pub show_typing: bool,
    /// Tell others when you are typing.
    pub send_typing: bool,
    /// Servers that asked for TLS only: host, TLS port, and until when
    /// (seconds since 1970).
    pub sts_policies: Vec<(String, u16, u64)>,
    pub proxy: bool,
    pub proxy_host: String,
    pub proxy_port: u16,
    pub proxy_user: String,
    pub proxy_pass: String,
    pub proxy_kind: String,
    pub sasl_password: String,
    /// SASL account name. Blank uses the nick from these settings.
    pub sasl_user: String,
    pub font_size: u8,
    pub dictionary: String,
    /// Alternative nick tried when the nick is taken.
    pub alt_nick: String,
    /// Connect to the saved server when OpenIRC starts.
    pub connect_on_start: bool,
    /// Where the nick list sits: right, left, or hidden.
    pub nicklist: String,
    /// Short join/part/quit lines.
    pub short_joins: bool,
    /// Hide PING/PONG lines.
    pub hide_ping: bool,
    /// Hide the message of the day on connect.
    pub skip_motd: bool,
    /// Timestamp format: HH, MM, SS are replaced.
    pub timestamp_format: String,
    /// Mark the last line read when leaving a window.
    pub line_marker: bool,
    /// Join a channel again after being kicked.
    pub rejoin_on_kick: bool,
    /// Join a channel when invited.
    pub join_on_invite: bool,
    /// Send WHOIS when someone opens a query.
    pub whois_on_query: bool,
    /// Quit message; blank uses the OpenIRC version.
    pub quit_message: String,
    /// CTCP FINGER reply; blank uses the real name.
    pub finger_reply: String,
    /// Remove colors and text styles from incoming messages.
    pub strip_codes: bool,
    /// Pace own messages so the server does not disconnect for flooding.
    pub send_queue: bool,
    /// Ask before pasting this many lines or more; 0 never asks.
    pub paste_confirm: u32,
    /// Use a monospace font for the transcript.
    pub transcript_mono: bool,
    /// Shade every other transcript line.
    pub line_shading: bool,
    /// Show small notices for highlights, private messages, and invites.
    pub tips: bool,
    /// Show the network name in the switchbar and title.
    pub show_network: bool,
    /// Saved servers, grouped by network.
    pub servers: Vec<crate::servers::ServerEntry>,
    /// PEM file holding the client certificate and its private key.
    pub client_cert: String,
    /// Server certificates the user chose to trust: host and SHA-256.
    pub trusted_certs: Vec<(String, String)>,
    /// Seconds to wait for a connection before giving up.
    pub connect_timeout: u32,
    /// Times to retry a dropped or failed connection; 0 retries forever.
    pub retry_attempts: u32,
    /// Seconds between retries.
    pub retry_delay: u32,
    /// On a retry, try the next saved server of the same network.
    pub retry_next: bool,
    /// Address order: system, ipv4, or ipv6 first.
    pub address_order: String,
    /// Local address to connect from; blank lets the system choose.
    pub bind_address: String,
    /// When binding fails, connect without binding.
    pub bind_fallback: bool,
    /// First DCC port; 0 uses any free port.
    pub dcc_port_first: u16,
    /// Last DCC port.
    pub dcc_port_last: u16,
    /// DCC address: local, server, or manual.
    pub dcc_ip_method: String,
    /// DCC address set by hand.
    pub dcc_ip: String,
    /// Proxy use: servers, dcc, or both.
    pub proxy_scope: String,
    /// Hosts that never use the proxy.
    pub proxy_except: String,
    /// Operating system name in ident replies.
    pub ident_system: String,
    /// Port the ident server listens on.
    pub ident_port: u16,
    /// Run the ident server only while connecting.
    pub ident_connecting_only: bool,
    pub remotes: Vec<crate::script::Remote>,
    pub popups: Vec<Popup>,
    pub vars: Vec<(String, String)>,
    pub dialogs: Vec<crate::dialog::Dialog>,
    /// The script that runs: the enabled script files joined, or the
    /// single script of older settings files.
    pub script_text: String,
    /// The script files.
    pub scripts: Vec<ScriptFile>,
    pub favorites: Vec<Favorite>,
    pub aliases: Vec<Alias>,
    /// Set when the file exists but could not be read. Saving is refused so
    /// the unread file is never replaced with defaults.
    pub locked: bool,
}

impl Settings {
    pub fn new() -> Self {
        let nick = format!("oi{}", std::process::id() % 100_000);
        Self {
            server: "irc.libera.chat".into(),
            port: 6697,
            tls: true,
            nick: nick.clone(),
            user: nick.to_ascii_lowercase(),
            realname: "OpenIRC user".into(),
            autojoin: String::new(),
            show_joins: true,
            timestamps: true,
            reconnect: true,
            logging: false,
            log_dir: config_dir().join("logs").display().to_string(),
            log_keep_codes: false,
            log_dated: "none".into(),
            log_by_network: false,
            log_max_kb: 0,
            log_reload: 0,
            notify: Vec::new(),
            ignores: Vec::new(),
            beep: false,
            sounds: false,
            theme: crate::theme::Theme::classic(),
            palette: crate::colors::PALETTE,
            ident: false,
            ident_user: String::new(),
            event_sounds: EventSounds::new(),
            highlights: Vec::new(),
            highlight_nick: true,
            nick_colors: Vec::new(),
            auto_nick_colors: true,
            auto_modes: Vec::new(),
            auto_mode_delay: true,
            perform: Vec::new(),
            flood_messages: 10,
            flood_joins: 8,
            flood_window_secs: 5,
            flood_ctcp: 3,
            query_limit: 5,
            active_notices: false,
            active_ctcp: false,
            active_invites: false,
            active_whois: true,
            active_away: true,
            message_window: false,
            multiline_input: false,
            command_prefix: "/".into(),
            separator: "|".into(),
            dclick_nick: "/query $nick".into(),
            dclick_channel: "/channel".into(),
            dclick_query: "/whois $nick".into(),
            dclick_status: String::new(),
            drop_command: "/dcc send $nick $file".into(),
            users: Vec::new(),
            default_level: "1".into(),
            dcc_folder: config_dir().join("downloads").display().to_string(),
            dcc_folder_rules: Vec::new(),
            dcc_auto_accept: Vec::new(),
            dcc_ignore_types:
                "exe com bat cmd scr pif vbs vbe js jse wsf wsh msi msp ps1 lnk hta cpl".into(),
            dcc_exists: "ask".into(),
            restore_windows: true,
            last_windows: Vec::new(),
            auto_away_minutes: 0,
            auto_away_reason: "Away from the keyboard".into(),
            auto_back: true,
            nick_color_line: false,
            title_text: String::new(),
            nicklist_hide_badges: false,
            tree_hide_hash: false,
            dcc_keep_awake: true,
            toolbar_hidden: Vec::new(),
            keep_history: true,
            history: Vec::new(),
            channels_on_connect: true,
            window_place: None,
            window_maximized: false,
            spell_input: true,
            nick_rank: false,
            nick_open: "<".into(),
            nick_close: ">".into(),
            search_url: "https://duckduckgo.com/?q=%s".into(),
            language: String::new(),
            monitor_panel: false,
            mono_windows: Vec::new(),
            window_order: "opened".into(),
            minimize_to_tray: false,
            tray_blink: true,
            ircv3: true,
            show_typing: true,
            send_typing: false,
            sts_policies: Vec::new(),
            proxy: false,
            proxy_host: String::new(),
            proxy_port: 1080,
            proxy_user: String::new(),
            proxy_pass: String::new(),
            proxy_kind: "socks5".into(),
            sasl_password: String::new(),
            sasl_user: String::new(),
            font_size: 13,
            dictionary: String::new(),
            alt_nick: String::new(),
            connect_on_start: false,
            nicklist: "right".into(),
            short_joins: false,
            hide_ping: true,
            skip_motd: false,
            timestamp_format: "HH:MM".into(),
            line_marker: true,
            rejoin_on_kick: false,
            join_on_invite: false,
            whois_on_query: false,
            quit_message: String::new(),
            finger_reply: String::new(),
            strip_codes: false,
            send_queue: true,
            paste_confirm: 4,
            transcript_mono: false,
            line_shading: false,
            tips: true,
            show_network: true,
            servers: Vec::new(),
            client_cert: String::new(),
            trusted_certs: Vec::new(),
            connect_timeout: 30,
            retry_attempts: 10,
            retry_delay: 5,
            retry_next: true,
            address_order: "system".into(),
            bind_address: String::new(),
            bind_fallback: true,
            dcc_port_first: 0,
            dcc_port_last: 0,
            dcc_ip_method: "local".into(),
            dcc_ip: String::new(),
            proxy_scope: "servers".into(),
            proxy_except: String::new(),
            ident_system: "UNIX".into(),
            ident_port: 113,
            ident_connecting_only: false,
            remotes: Vec::new(),
            popups: Vec::new(),
            vars: Vec::new(),
            dialogs: Vec::new(),
            script_text: String::new(),
            scripts: Vec::new(),
            favorites: Vec::new(),
            aliases: Vec::new(),
            locked: false,
        }
    }

    /// Read the settings file. Lines that cannot be read are skipped and
    /// described in the returned warnings; when there are any, the original
    /// file is copied to `settings.txt.bak` before anything can overwrite it.
    pub fn load() -> (Self, Vec<String>) {
        Self::load_from(&settings_path())
    }

    fn load_from(path: &std::path::Path) -> (Self, Vec<String>) {
        let path = path.to_path_buf();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return (Self::new(), Vec::new()),
            Err(err) => {
                let mut settings = Self::new();
                settings.locked = true;
                return (
                    settings,
                    vec![format!(
                        "Could not read {}: {err}. Defaults are in use and will not be saved over it.",
                        path.display()
                    )],
                );
            }
        };
        let text = crate::message::decode_line(&bytes);
        let (mut settings, mut warnings) = parse_settings(&text);
        if !warnings.is_empty() {
            let backup = path.with_extension("txt.bak");
            match fs::copy(&path, &backup) {
                Ok(_) => warnings.push(format!(
                    "The original file was copied to {}.",
                    backup.display()
                )),
                Err(err) => {
                    // Without a copy, saving would lose the damaged lines for
                    // good, so the file is left as it is.
                    settings.locked = true;
                    warnings.push(format!(
                        "Could not back up {}: {err}. Changes will not be saved over it.",
                        path.display()
                    ));
                }
            }
        }
        (settings, warnings)
    }

    /// Write the settings through a temporary file and a rename, so a crash
    /// mid-write never leaves a half file. On Unix the file is private to the
    /// user because it can hold the SASL and proxy passwords.
    pub fn save(&self) -> io::Result<()> {
        self.save_to(&settings_path())
    }

    fn save_to(&self, path: &std::path::Path) -> io::Result<()> {
        if self.locked {
            return Err(io::Error::other(
                "settings were not loaded from disk, so they are not saved over the file",
            ));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // A temporary file of this save's own, created fresh: two saves at
        // once (two copies of the program) never write into one file.
        static SAVES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let temp = path.with_extension(format!(
            "txt.{}.{}.tmp",
            std::process::id(),
            SAVES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_file(&temp);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        let written = file
            .write_all(encode_settings(self).as_bytes())
            .and_then(|()| file.sync_all());
        drop(file);
        if let Err(err) = written.and_then(|()| fs::rename(&temp, path)) {
            let _ = fs::remove_file(&temp);
            return Err(err);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    /// Join the enabled script files into the script that runs.
    pub fn rebuild_script(&mut self) {
        self.script_text = self
            .scripts
            .iter()
            .filter(|file| file.enabled)
            .map(|file| file.text.trim_end())
            .collect::<Vec<_>>()
            .join("\n\n");
    }

    /// The command separator with the spaces around it, such as ` | `.
    pub fn split_token(&self) -> String {
        let separator = self.separator.trim();
        format!(" {} ", if separator.is_empty() { "|" } else { separator })
    }

    /// The commands an alias line stands for, or `None` when `line` does
    /// not call an alias. The body is split into commands before its `$1`
    /// fields are filled in, so a separator inside the arguments (which may
    /// be another user's text) stays text and never starts a new command.
    pub fn expand_alias(&self, line: &str) -> Option<Vec<String>> {
        let name = line.strip_prefix('/')?;
        let (name, rest) = name
            .split_once(char::is_whitespace)
            .map(|(name, rest)| (name, rest.trim()))
            .unwrap_or((name, ""));
        let alias = self
            .aliases
            .iter()
            .find(|alias| alias.name.eq_ignore_ascii_case(name))?;
        let args: Vec<&str> = rest.split_whitespace().collect();
        let separator = self.split_token();
        let mut parts: Vec<String> = alias
            .body
            .split(separator.as_str())
            .map(|part| {
                if alias.body.contains('$') {
                    fill_params(part, name, &args)
                } else {
                    part.to_string()
                }
            })
            .collect();
        if !alias.body.contains('$')
            && !rest.is_empty()
            && let Some(last) = parts.last_mut()
        {
            last.push(' ');
            last.push_str(rest);
        }
        Some(parts)
    }
}

fn fill_params(body: &str, name: &str, args: &[&str]) -> String {
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '$' && chars.get(index + 1) == Some(&'$') {
            out.push('$');
            index += 2;
            continue;
        }
        if chars[index] == '$'
            && let Some(digit) = chars.get(index + 1).and_then(|ch| ch.to_digit(10))
        {
            let number = digit as usize;
            index += 2;
            let ranged = number > 0 && chars.get(index) == Some(&'-');
            if ranged {
                index += 1;
            }
            if number == 0 {
                if !ranged {
                    out.push_str(name);
                }
            } else if ranged {
                if number <= args.len() {
                    out.push_str(&args[number - 1..].join(" "));
                }
            } else if number <= args.len() {
                out.push_str(args[number - 1]);
            }
            continue;
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

/// A saved `true` or `false`, and nothing else: a damaged value such as
/// `tru` must never be read as `false` and quietly turn TLS or any other
/// option off.
pub fn parse_flag(text: &str) -> Option<bool> {
    match text {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

pub fn config_dir() -> PathBuf {
    if let Ok(path) = std::env::var("XDG_CONFIG_HOME")
        && !path.is_empty()
    {
        return PathBuf::from(path).join("openirc");
    }
    let home = std::env::var("HOME")
        .ok()
        .filter(|home| !home.is_empty())
        .map(|home| PathBuf::from(home).join(".config").join("openirc"));
    // Windows has no HOME unless a Unix-style shell sets one: settings go
    // in the roaming application data folder, unless a HOME folder is
    // already in use.
    #[cfg(windows)]
    {
        let in_use = home
            .as_ref()
            .filter(|folder| folder.join("settings.txt").is_file());
        if let Some(folder) = in_use {
            return folder.clone();
        }
        if let Some(appdata) = std::env::var_os("APPDATA").filter(|path| !path.is_empty()) {
            return PathBuf::from(appdata).join("openirc");
        }
    }
    home.unwrap_or_else(|| PathBuf::from("openirc-config"))
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.txt")
}

/// Current file format. Format 2 escapes `\`, tab, CR, and LF in every value
/// so any text survives a round trip. Files without a `format` line are
/// format 1, where only the script and dialogs used `\n`.
const FORMAT: u32 = 2;

fn esc(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

fn unesc(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.peek() {
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            _ => {
                out.push('\\');
                continue;
            }
        }
        chars.next();
    }
    out
}

fn encode_settings(settings: &Settings) -> String {
    let mut out = String::new();
    let mut line = |key: &str, fields: &[&str]| {
        let joined: Vec<String> = fields.iter().map(|field| esc(field)).collect();
        out.push_str(key);
        out.push(' ');
        out.push_str(&joined.join("\t"));
        out.push('\n');
    };
    line("format", &[&FORMAT.to_string()]);
    line("server", &[&settings.server]);
    line("port", &[&settings.port.to_string()]);
    line("tls", &[&settings.tls.to_string()]);
    line("nick", &[&settings.nick]);
    line("user", &[&settings.user]);
    line("realname", &[&settings.realname]);
    line("autojoin", &[&settings.autojoin]);
    line("show_joins", &[&settings.show_joins.to_string()]);
    line("timestamps", &[&settings.timestamps.to_string()]);
    line("reconnect", &[&settings.reconnect.to_string()]);
    line("logging", &[&settings.logging.to_string()]);
    line("log_dir", &[&settings.log_dir]);
    line("log_keep_codes", &[&settings.log_keep_codes.to_string()]);
    line("log_dated", &[&settings.log_dated]);
    line("log_by_network", &[&settings.log_by_network.to_string()]);
    line("log_max_kb", &[&settings.log_max_kb.to_string()]);
    line("log_reload", &[&settings.log_reload.to_string()]);
    for nick in &settings.notify {
        line("notify", &[nick]);
    }
    for rule in &settings.ignores {
        let fields = rule.fields();
        let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
        line("ignore_rule", &fields);
    }
    line("beep", &[&settings.beep.to_string()]);
    line("sounds", &[&settings.sounds.to_string()]);
    line("ident", &[&settings.ident.to_string()]);
    line("ident_user", &[&settings.ident_user]);
    for (name, color) in [
        ("face", settings.theme.face),
        ("paper", settings.theme.paper),
        ("ink", settings.theme.ink),
        ("shadow", settings.theme.shadow),
        ("hilite", settings.theme.hilite),
        ("attention", settings.theme.attention),
    ] {
        line("color", &[&format!("{name} {}", color.encode())]);
    }
    if settings.palette != crate::colors::PALETTE {
        let colors: Vec<String> = settings
            .palette
            .iter()
            .map(|rgb| crate::theme::Rgb(rgb[0], rgb[1], rgb[2]).encode())
            .collect();
        line("palette", &[&colors.join(" ")]);
    }
    for event in SOUND_EVENTS {
        let path = settings.event_sounds.path(event);
        if !path.is_empty() {
            line("sound", &[event, path]);
        }
    }
    for rule in &settings.highlights {
        let fields = rule.fields();
        let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
        line("highlight_rule", &fields);
    }
    line("highlight_nick", &[&settings.highlight_nick.to_string()]);
    for rule in &settings.nick_colors {
        let fields = rule.fields();
        let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
        line("nick_color", &fields);
    }
    line(
        "auto_nick_colors",
        &[&settings.auto_nick_colors.to_string()],
    );
    for rule in &settings.auto_modes {
        let fields = rule.fields();
        let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
        line("auto_mode", &fields);
    }
    line("auto_mode_delay", &[&settings.auto_mode_delay.to_string()]);
    for item in &settings.perform {
        line("perform", &[&item.server, &item.command]);
    }
    line("flood_messages", &[&settings.flood_messages.to_string()]);
    line("flood_joins", &[&settings.flood_joins.to_string()]);
    line("flood_window", &[&settings.flood_window_secs.to_string()]);
    line("flood_ctcp", &[&settings.flood_ctcp.to_string()]);
    line("query_limit", &[&settings.query_limit.to_string()]);
    line("active_notices", &[&settings.active_notices.to_string()]);
    line("active_ctcp", &[&settings.active_ctcp.to_string()]);
    line("active_invites", &[&settings.active_invites.to_string()]);
    line("active_whois", &[&settings.active_whois.to_string()]);
    line("active_away", &[&settings.active_away.to_string()]);
    line("message_window", &[&settings.message_window.to_string()]);
    line("multiline_input", &[&settings.multiline_input.to_string()]);
    line("command_prefix", &[&settings.command_prefix]);
    line("separator", &[&settings.separator]);
    line("dclick_nick", &[&settings.dclick_nick]);
    line("dclick_channel", &[&settings.dclick_channel]);
    line("dclick_query", &[&settings.dclick_query]);
    line("dclick_status", &[&settings.dclick_status]);
    line("drop_command", &[&settings.drop_command]);
    for (levels, mask) in &settings.users {
        line("user_level", &[levels, mask]);
    }
    line("default_level", &[&settings.default_level]);
    line("dcc_folder", &[&settings.dcc_folder]);
    for (types, folder) in &settings.dcc_folder_rules {
        line("dcc_folder_rule", &[types, folder]);
    }
    for mask in &settings.dcc_auto_accept {
        line("dcc_auto_accept", &[mask]);
    }
    line("dcc_ignore_types", &[&settings.dcc_ignore_types]);
    line("dcc_exists", &[&settings.dcc_exists]);
    line("restore_windows", &[&settings.restore_windows.to_string()]);
    for (host, channels, queries) in &settings.last_windows {
        line(
            "last_windows",
            &[host, &channels.join(" "), &queries.join(" ")],
        );
    }
    line(
        "auto_away_minutes",
        &[&settings.auto_away_minutes.to_string()],
    );
    line("auto_away_reason", &[&settings.auto_away_reason]);
    line("auto_back", &[&settings.auto_back.to_string()]);
    line("nick_color_line", &[&settings.nick_color_line.to_string()]);
    line("title_text", &[&settings.title_text]);
    line(
        "nicklist_hide_badges",
        &[&settings.nicklist_hide_badges.to_string()],
    );
    line("tree_hide_hash", &[&settings.tree_hide_hash.to_string()]);
    line("dcc_keep_awake", &[&settings.dcc_keep_awake.to_string()]);
    for name in &settings.toolbar_hidden {
        line("toolbar_hidden", &[name]);
    }
    line("keep_history", &[&settings.keep_history.to_string()]);
    if settings.keep_history {
        for typed in &settings.history {
            line("history", &[typed]);
        }
    }
    line(
        "channels_on_connect",
        &[&settings.channels_on_connect.to_string()],
    );
    if let Some([x, y, width, height]) = settings.window_place {
        line(
            "window_place",
            &[
                &x.to_string(),
                &y.to_string(),
                &width.to_string(),
                &height.to_string(),
            ],
        );
    }
    line(
        "window_maximized",
        &[&settings.window_maximized.to_string()],
    );
    line("spell_input", &[&settings.spell_input.to_string()]);
    line("nick_rank", &[&settings.nick_rank.to_string()]);
    line(
        "nick_brackets",
        &[&settings.nick_open, &settings.nick_close],
    );
    line("search_url", &[&settings.search_url]);
    line("language", &[&settings.language]);
    line("monitor_panel", &[&settings.monitor_panel.to_string()]);
    for title in &settings.mono_windows {
        line("mono_window", &[title]);
    }
    line("window_order", &[&settings.window_order]);
    line(
        "minimize_to_tray",
        &[&settings.minimize_to_tray.to_string()],
    );
    line("tray_blink", &[&settings.tray_blink.to_string()]);
    line("ircv3", &[&settings.ircv3.to_string()]);
    line("show_typing", &[&settings.show_typing.to_string()]);
    line("send_typing", &[&settings.send_typing.to_string()]);
    for (host, port, until) in &settings.sts_policies {
        line("sts", &[host, &port.to_string(), &until.to_string()]);
    }
    line("proxy", &[&settings.proxy.to_string()]);
    line("proxy_host", &[&settings.proxy_host]);
    line("proxy_port", &[&settings.proxy_port.to_string()]);
    line("proxy_user", &[&settings.proxy_user]);
    line("proxy_pass", &[&settings.proxy_pass]);
    line("proxy_kind", &[&settings.proxy_kind]);
    line("sasl_password", &[&settings.sasl_password]);
    line("sasl_user", &[&settings.sasl_user]);
    line("font_size", &[&settings.font_size.to_string()]);
    line("dictionary", &[&settings.dictionary]);
    line("alt_nick", &[&settings.alt_nick]);
    line(
        "connect_on_start",
        &[&settings.connect_on_start.to_string()],
    );
    line("nicklist", &[&settings.nicklist]);
    line("short_joins", &[&settings.short_joins.to_string()]);
    line("hide_ping", &[&settings.hide_ping.to_string()]);
    line("skip_motd", &[&settings.skip_motd.to_string()]);
    line("timestamp_format", &[&settings.timestamp_format]);
    line("line_marker", &[&settings.line_marker.to_string()]);
    line("rejoin_on_kick", &[&settings.rejoin_on_kick.to_string()]);
    line("join_on_invite", &[&settings.join_on_invite.to_string()]);
    line("whois_on_query", &[&settings.whois_on_query.to_string()]);
    line("quit_message", &[&settings.quit_message]);
    line("finger_reply", &[&settings.finger_reply]);
    line("strip_codes", &[&settings.strip_codes.to_string()]);
    line("send_queue", &[&settings.send_queue.to_string()]);
    line("paste_confirm", &[&settings.paste_confirm.to_string()]);
    line("transcript_mono", &[&settings.transcript_mono.to_string()]);
    line("line_shading", &[&settings.line_shading.to_string()]);
    line("tips", &[&settings.tips.to_string()]);
    line("show_network", &[&settings.show_network.to_string()]);
    for server in &settings.servers {
        let fields = server.fields();
        let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
        line("saved_server", &fields);
    }
    line("client_cert", &[&settings.client_cert]);
    for (host, fingerprint) in &settings.trusted_certs {
        line("trusted_cert", &[host, fingerprint]);
    }
    line("connect_timeout", &[&settings.connect_timeout.to_string()]);
    line("retry_attempts", &[&settings.retry_attempts.to_string()]);
    line("retry_delay", &[&settings.retry_delay.to_string()]);
    line("retry_next", &[&settings.retry_next.to_string()]);
    line("address_order", &[&settings.address_order]);
    line("bind_address", &[&settings.bind_address]);
    line("bind_fallback", &[&settings.bind_fallback.to_string()]);
    line("dcc_port_first", &[&settings.dcc_port_first.to_string()]);
    line("dcc_port_last", &[&settings.dcc_port_last.to_string()]);
    line("dcc_ip_method", &[&settings.dcc_ip_method]);
    line("dcc_ip", &[&settings.dcc_ip]);
    line("proxy_scope", &[&settings.proxy_scope]);
    line("proxy_except", &[&settings.proxy_except]);
    line("ident_system", &[&settings.ident_system]);
    line("ident_port", &[&settings.ident_port.to_string()]);
    line(
        "ident_connecting_only",
        &[&settings.ident_connecting_only.to_string()],
    );
    for remote in &settings.remotes {
        line("remote", &[&remote.event, &remote.mask, &remote.body]);
    }
    for popup in &settings.popups {
        if popup.menu.is_empty() || popup.menu == "line" {
            line("popup", &[&popup.label, &popup.body]);
        } else {
            line("popup", &[&popup.label, &popup.body, &popup.menu]);
        }
    }
    for (name, value) in &settings.vars {
        line("var", &[name, value]);
    }
    for dialog in &settings.dialogs {
        line("dialog", &[&crate::dialog::emit(dialog)]);
    }
    if settings.scripts.is_empty() {
        if !settings.script_text.is_empty() {
            line("script_text", &[&settings.script_text]);
        }
    } else {
        for file in &settings.scripts {
            line(
                "script_file",
                &[&file.name, &file.enabled.to_string(), &file.text],
            );
        }
    }
    for favorite in &settings.favorites {
        line(
            "favorite",
            &[
                &favorite.name,
                &favorite.server,
                &favorite.port.to_string(),
                &favorite.tls.to_string(),
                &favorite.channels,
            ],
        );
    }
    for alias in &settings.aliases {
        line("alias", &[&alias.name, &alias.body]);
    }
    out
}

/// Parse the settings text. Each line stands alone: a line that cannot be
/// read is skipped and reported, and every other line still applies.
fn parse_settings(text: &str) -> (Settings, Vec<String>) {
    let mut settings = Settings::new();
    let mut warnings = Vec::new();
    let mut format = 1;
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let line = raw.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        let fields: Vec<String> = value
            .split('\t')
            .map(|field| {
                if format >= 2 {
                    unesc(field)
                } else {
                    field.to_string()
                }
            })
            .collect();
        let one = fields.join("\t");
        let mut bad = |what: &str| warnings.push(format!("settings line {number} ({key}): {what}"));
        // `true` or `false`; anything else is reported and the default kept,
        // so a typo never quietly turns an option such as TLS off.
        macro_rules! flag {
            ($slot:expr) => {
                match parse_flag(one.trim()) {
                    Some(value) => $slot = value,
                    None => bad(&format!("`{}` is not true or false", one.trim())),
                }
            };
        }
        macro_rules! number {
            ($slot:expr) => {
                match one.trim().parse() {
                    Ok(parsed) => $slot = parsed,
                    Err(_) => bad(&format!("`{}` is not a valid number", one.trim())),
                }
            };
        }
        match key {
            "format" => match one.trim().parse::<u32>() {
                Ok(found) if found <= FORMAT => format = found,
                _ => bad("this file was written by a newer OpenIRC"),
            },
            "server" => settings.server = one,
            "port" => number!(settings.port),
            "tls" => flag!(settings.tls),
            "nick" => settings.nick = one,
            "user" => settings.user = one,
            "realname" => settings.realname = one,
            "autojoin" => settings.autojoin = one,
            "show_joins" => flag!(settings.show_joins),
            "timestamps" => flag!(settings.timestamps),
            "reconnect" => flag!(settings.reconnect),
            "logging" => flag!(settings.logging),
            "log_dir" => settings.log_dir = one,
            "log_keep_codes" => flag!(settings.log_keep_codes),
            "log_dated" => settings.log_dated = one,
            "log_by_network" => flag!(settings.log_by_network),
            "log_max_kb" => number!(settings.log_max_kb),
            "log_reload" => number!(settings.log_reload),
            "notify" => settings.notify.push(one),
            // Format 1 kept nicks only; they ignore everything.
            "ignore" => settings.ignores.push(crate::rules::IgnoreRule::all(&one)),
            "ignore_rule" => match crate::rules::IgnoreRule::from_fields(&fields) {
                Some(rule) => settings.ignores.push(rule),
                None => bad("expected mask, types, network, and expiry"),
            },
            "highlight_rule" => match crate::rules::HighlightRule::from_fields(&fields) {
                Some(rule) => settings.highlights.push(rule),
                None => bad("expected 9 highlight fields, each yes/no one true or false"),
            },
            "highlight_nick" => flag!(settings.highlight_nick),
            "nick_color" => match crate::rules::NickColorRule::from_fields(&fields) {
                Some(rule) => settings.nick_colors.push(rule),
                None => bad("expected mask, modes, and a color 0-15"),
            },
            "auto_nick_colors" => flag!(settings.auto_nick_colors),
            "auto_mode" => match crate::rules::AutoMode::from_fields(&fields) {
                Some(rule) => settings.auto_modes.push(rule),
                None => bad("expected mask, channels, network, mode, and protect true or false"),
            },
            "auto_mode_delay" => flag!(settings.auto_mode_delay),
            "beep" => flag!(settings.beep),
            "sounds" => flag!(settings.sounds),
            "ident" => flag!(settings.ident),
            "ident_user" => settings.ident_user = one,
            "color" => match one.split_once(' ') {
                Some((name, hex)) => match crate::theme::Rgb::parse(hex) {
                    Some(rgb) => settings.theme.set(name, rgb),
                    None => bad(&format!("`{hex}` is not a RRGGBB color")),
                },
                None => bad("expected a name and a color"),
            },
            "palette" => {
                let colors: Vec<Option<crate::theme::Rgb>> = one
                    .split_whitespace()
                    .map(crate::theme::Rgb::parse)
                    .collect();
                if colors.len() == 16 && colors.iter().all(Option::is_some) {
                    for (slot, color) in settings
                        .palette
                        .iter_mut()
                        .zip(colors.into_iter().flatten())
                    {
                        *slot = [color.0, color.1, color.2];
                    }
                } else {
                    bad("expected 16 RRGGBB colors");
                }
            }
            "sound" => match fields.as_slice() {
                [event, path] => settings.event_sounds.set(event, path.clone()),
                _ => bad("expected an event and a path"),
            },
            "highlight" => {
                if !one.trim().is_empty() {
                    settings
                        .highlights
                        .push(crate::rules::HighlightRule::words(&one));
                }
            }
            "perform" => {
                let (server, command) = match fields.as_slice() {
                    [server, command] => (server.clone(), command.clone()),
                    _ => (String::new(), one.clone()),
                };
                if !command.trim().is_empty() {
                    settings.perform.push(Perform {
                        server,
                        command: command.trim().to_string(),
                    });
                }
            }
            "flood_messages" => number!(settings.flood_messages),
            "flood_joins" => number!(settings.flood_joins),
            "flood_window" => number!(settings.flood_window_secs),
            "flood_ctcp" => number!(settings.flood_ctcp),
            "query_limit" => number!(settings.query_limit),
            "active_notices" => flag!(settings.active_notices),
            "active_ctcp" => flag!(settings.active_ctcp),
            "active_invites" => flag!(settings.active_invites),
            "active_whois" => flag!(settings.active_whois),
            "active_away" => flag!(settings.active_away),
            "message_window" => flag!(settings.message_window),
            "multiline_input" => flag!(settings.multiline_input),
            "command_prefix" => settings.command_prefix = one,
            "separator" => settings.separator = one,
            "dclick_nick" => settings.dclick_nick = one,
            "dclick_channel" => settings.dclick_channel = one,
            "dclick_query" => settings.dclick_query = one,
            "dclick_status" => settings.dclick_status = one,
            "drop_command" => settings.drop_command = one,
            "user_level" => match fields.as_slice() {
                [levels, mask] if !mask.trim().is_empty() => {
                    settings.users.push((levels.clone(), mask.clone()))
                }
                _ => bad("expected levels and a mask"),
            },
            "default_level" => settings.default_level = one,
            "dcc_folder" => settings.dcc_folder = one,
            "dcc_folder_rule" => match fields.as_slice() {
                [types, folder] if !types.trim().is_empty() => settings
                    .dcc_folder_rules
                    .push((types.clone(), folder.clone())),
                _ => bad("expected file types and a folder"),
            },
            "dcc_auto_accept" => {
                if !one.trim().is_empty() {
                    settings.dcc_auto_accept.push(one.trim().to_string());
                }
            }
            "dcc_ignore_types" => settings.dcc_ignore_types = one,
            "dcc_exists" => settings.dcc_exists = one,
            "restore_windows" => flag!(settings.restore_windows),
            "last_windows" => match fields.as_slice() {
                [host, channels, queries] if !host.is_empty() => settings.last_windows.push((
                    host.clone(),
                    channels.split_whitespace().map(str::to_string).collect(),
                    queries.split_whitespace().map(str::to_string).collect(),
                )),
                _ => bad("expected a server, channels, and queries"),
            },
            "auto_away_minutes" => number!(settings.auto_away_minutes),
            "auto_away_reason" => settings.auto_away_reason = one,
            "auto_back" => flag!(settings.auto_back),
            "nick_color_line" => flag!(settings.nick_color_line),
            "title_text" => settings.title_text = one,
            "nicklist_hide_badges" => flag!(settings.nicklist_hide_badges),
            "tree_hide_hash" => flag!(settings.tree_hide_hash),
            "dcc_keep_awake" => flag!(settings.dcc_keep_awake),
            "toolbar_hidden" => settings.toolbar_hidden.push(one),
            "keep_history" => flag!(settings.keep_history),
            "history" => settings.history.push(one),
            "channels_on_connect" => flag!(settings.channels_on_connect),
            "window_place" => {
                let numbers: Vec<i32> = fields
                    .iter()
                    .filter_map(|field| field.trim().parse().ok())
                    .collect();
                match numbers.as_slice() {
                    [x, y, width, height] if *width >= 200 && *height >= 150 => {
                        settings.window_place = Some([*x, *y, *width, *height]);
                    }
                    _ => bad("expected x, y, width, and height"),
                }
            }
            "window_maximized" => flag!(settings.window_maximized),
            "spell_input" => flag!(settings.spell_input),
            "nick_rank" => flag!(settings.nick_rank),
            "nick_brackets" => match fields.as_slice() {
                [open, close] => {
                    settings.nick_open = open.clone();
                    settings.nick_close = close.clone();
                }
                _ => bad("expected the text before and after the nick"),
            },
            "search_url" => settings.search_url = one,
            "language" => settings.language = one.trim().to_string(),
            "monitor_panel" => flag!(settings.monitor_panel),
            "mono_window" => {
                if !one.trim().is_empty() {
                    settings.mono_windows.push(one.trim().to_string());
                }
            }
            "window_order" => settings.window_order = one,
            "minimize_to_tray" => flag!(settings.minimize_to_tray),
            "tray_blink" => flag!(settings.tray_blink),
            "ircv3" => flag!(settings.ircv3),
            "show_typing" => flag!(settings.show_typing),
            "send_typing" => flag!(settings.send_typing),
            "sts" => match fields.as_slice() {
                [host, port, until] if !host.is_empty() => {
                    match (port.parse::<u16>(), until.parse::<u64>()) {
                        (Ok(port), Ok(until)) => {
                            settings.sts_policies.push((host.clone(), port, until))
                        }
                        _ => bad("expected a host, a port, and a time"),
                    }
                }
                _ => bad("expected a host, a port, and a time"),
            },
            "proxy" => flag!(settings.proxy),
            "proxy_host" => settings.proxy_host = one,
            "proxy_port" => number!(settings.proxy_port),
            "proxy_user" => settings.proxy_user = one,
            "proxy_pass" => settings.proxy_pass = one,
            "proxy_kind" => settings.proxy_kind = one,
            "sasl_password" => settings.sasl_password = one,
            "sasl_user" => settings.sasl_user = one,
            "font_size" => match one.trim().parse::<u8>() {
                Ok(size) => settings.font_size = size.clamp(9, 28),
                Err(_) => bad(&format!("`{}` is not a font size", one.trim())),
            },
            "dictionary" => settings.dictionary = one,
            "alt_nick" => settings.alt_nick = one,
            "connect_on_start" => flag!(settings.connect_on_start),
            "nicklist" => settings.nicklist = one,
            "short_joins" => flag!(settings.short_joins),
            "hide_ping" => flag!(settings.hide_ping),
            "skip_motd" => flag!(settings.skip_motd),
            "timestamp_format" => settings.timestamp_format = one,
            "line_marker" => flag!(settings.line_marker),
            "rejoin_on_kick" => flag!(settings.rejoin_on_kick),
            "join_on_invite" => flag!(settings.join_on_invite),
            "whois_on_query" => flag!(settings.whois_on_query),
            "quit_message" => settings.quit_message = one,
            "finger_reply" => settings.finger_reply = one,
            "strip_codes" => flag!(settings.strip_codes),
            "send_queue" => flag!(settings.send_queue),
            "paste_confirm" => number!(settings.paste_confirm),
            "transcript_mono" => flag!(settings.transcript_mono),
            "line_shading" => flag!(settings.line_shading),
            "tips" => flag!(settings.tips),
            "show_network" => flag!(settings.show_network),
            "saved_server" => match crate::servers::ServerEntry::from_fields(&fields) {
                Some(entry) => settings.servers.push(entry),
                None => bad("expected 13 server fields, with a port and TLS true or false"),
            },
            "client_cert" => settings.client_cert = one,
            "trusted_cert" => match fields.as_slice() {
                [host, fingerprint] if !host.is_empty() => settings
                    .trusted_certs
                    .push((host.clone(), fingerprint.clone())),
                _ => bad("expected a host and a fingerprint"),
            },
            "connect_timeout" => number!(settings.connect_timeout),
            "retry_attempts" => number!(settings.retry_attempts),
            "retry_delay" => number!(settings.retry_delay),
            "retry_next" => flag!(settings.retry_next),
            "address_order" => settings.address_order = one,
            "bind_address" => settings.bind_address = one,
            "bind_fallback" => flag!(settings.bind_fallback),
            "dcc_port_first" => number!(settings.dcc_port_first),
            "dcc_port_last" => number!(settings.dcc_port_last),
            "dcc_ip_method" => settings.dcc_ip_method = one,
            "dcc_ip" => settings.dcc_ip = one,
            "proxy_scope" => settings.proxy_scope = one,
            "proxy_except" => settings.proxy_except = one,
            "ident_system" => settings.ident_system = one,
            "ident_port" => number!(settings.ident_port),
            "ident_connecting_only" => flag!(settings.ident_connecting_only),
            "remote" => match fields.as_slice() {
                [event, mask, body] if !body.is_empty() => {
                    settings.remotes.push(crate::script::Remote {
                        event: event.clone(),
                        mask: mask.clone(),
                        body: body.clone(),
                    })
                }
                _ => bad("expected an event, a mask, and a body"),
            },
            "popup" => match fields.as_slice() {
                [label, body] if !label.is_empty() => settings.popups.push(Popup {
                    label: label.clone(),
                    body: body.clone(),
                    menu: "line".into(),
                }),
                [label, body, menu] if !label.is_empty() => settings.popups.push(Popup {
                    label: label.clone(),
                    body: body.clone(),
                    menu: menu.clone(),
                }),
                _ => bad("expected a label and a body"),
            },
            "var" => match fields.as_slice() {
                [name, value] if !name.is_empty() => {
                    settings.vars.push((name.clone(), value.clone()))
                }
                [name] if !name.is_empty() => settings.vars.push((name.clone(), String::new())),
                _ => bad("expected a name and a value"),
            },
            "dialog" => {
                let script = if format >= 2 {
                    one
                } else {
                    one.replace("\\n", "\n")
                };
                match crate::dialog::parse(&script) {
                    Some(dialog) => settings.dialogs.push(dialog),
                    None => bad("the dialog text could not be read"),
                }
            }
            "script_file" => match fields.as_slice() {
                [name, enabled, text] if !name.trim().is_empty() => match parse_flag(enabled) {
                    Some(enabled) => settings.scripts.push(ScriptFile {
                        name: name.clone(),
                        text: text.clone(),
                        enabled,
                    }),
                    None => bad(&format!("`{enabled}` is not true or false")),
                },
                _ => bad("expected a name, on or off, and the script"),
            },
            "script_text" => {
                settings.script_text = if format >= 2 {
                    one
                } else {
                    one.replace("\\n", "\n")
                }
            }
            "favorite" => {
                let mut parts = fields.clone();
                if parts.len() == 4 {
                    parts.push(String::new());
                }
                match parts.as_slice() {
                    [name, server, port, tls, channels] => {
                        match (port.trim().parse(), parse_flag(tls)) {
                            (Ok(port), Some(tls)) => settings.favorites.push(Favorite {
                                name: name.clone(),
                                server: server.clone(),
                                port,
                                tls,
                                channels: channels.clone(),
                            }),
                            (Err(_), _) => bad(&format!("`{port}` is not a valid port")),
                            (_, None) => bad(&format!("TLS `{tls}` is not true or false")),
                        }
                    }
                    _ => bad("expected name, server, port, tls, and channels"),
                }
            }
            "alias" => match fields.as_slice() {
                [name, body] if !name.is_empty() => settings.aliases.push(Alias {
                    name: name.clone(),
                    body: body.clone(),
                }),
                _ => bad("expected a name and a body"),
            },
            _ => bad("unknown setting"),
        }
    }
    if !settings.scripts.is_empty() {
        settings.rebuild_script();
    }
    (settings, warnings)
}

/// A file name for a window or network that no other name shares: letters,
/// digits and `# & + ! - . _` stay, every other character becomes `%XX`
/// for each of its bytes (and so does `%` itself). A leading or trailing
/// dot, and names Windows keeps for devices such as `CON`, are escaped too.
fn safe_name(window: &str) -> String {
    let mut safe = String::with_capacity(window.len());
    let last = window.chars().count().saturating_sub(1);
    for (index, ch) in window.chars().enumerate() {
        let edge_dot = ch == '.' && (index == 0 || index == last);
        if !edge_dot
            && (ch.is_alphanumeric() || matches!(ch, '#' | '&' | '+' | '!' | '-' | '.' | '_'))
        {
            safe.push(ch);
        } else {
            let mut bytes = [0u8; 4];
            for byte in ch.encode_utf8(&mut bytes).bytes() {
                safe.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    if safe.is_empty() {
        return "window".into();
    }
    let stem = safe.split('.').next().unwrap_or("").to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if device {
        let first = safe.remove(0);
        safe.insert_str(0, &format!("%{:02X}", first as u32));
    }
    safe
}

/// A name as `safe_name` wrote it, with each `%XX` turned back into its
/// character, for showing log files.
pub fn shown_log_name(name: &str) -> String {
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(byte) = name
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The log file for a window: in the network's folder when `network` is
/// given, and with the date in the name for `day` or `month` files.
pub fn log_file(
    dir: &str,
    network: Option<&str>,
    window: &str,
    dated: &str,
    now: &crate::clock::LocalTime,
) -> PathBuf {
    let mut folder = PathBuf::from(dir);
    if let Some(network) = network.filter(|network| !network.trim().is_empty()) {
        folder = folder.join(safe_name(network));
    }
    let name = safe_name(window);
    let file = match dated {
        "day" => format!("{name}.{:04}-{:02}-{:02}.log", now.year, now.month, now.day),
        "month" => format!("{name}.{:04}-{:02}.log", now.year, now.month),
        _ => format!("{name}.log"),
    };
    folder.join(file)
}

/// Options that create a file only its owner can read (on Unix; Windows
/// keeps files in the user's own folder private already).
fn private_options() -> fs::OpenOptions {
    let mut options = fs::OpenOptions::new();
    options.create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

/// Append one line to a log, then trim the file to three quarters of
/// `max_bytes` when it has grown past that (0 never trims).
pub fn append_log_file(path: &std::path::Path, line: &str, max_bytes: u64) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = private_options().append(true).open(path)?;
    // A log made by an older version, or by hand, may be readable by
    // others; it is made private too.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = file.metadata()?.permissions().mode();
        if mode & 0o077 != 0 {
            file.set_permissions(fs::Permissions::from_mode(mode & 0o700))?;
        }
    }
    writeln!(file, "{line}")?;
    let size = file.metadata()?.len();
    drop(file);
    if max_bytes > 0 && size > max_bytes {
        let kept = read_log_tail(path, max_bytes / 4 * 3)?;
        let partial = path.with_extension("log.trim");
        private_options()
            .write(true)
            .truncate(true)
            .open(&partial)?
            .write_all(kept.as_bytes())?;
        fs::rename(&partial, path)?;
    }
    Ok(())
}

/// Every log under `dir`, including the network folders, sorted by path.
pub fn list_logs(dir: &str) -> Vec<PathBuf> {
    fn walk(folder: &std::path::Path, depth: u8, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(folder) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && depth > 0 {
                walk(&path, depth - 1, out);
            } else if path.extension().is_some_and(|ext| ext == "log") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(std::path::Path::new(dir), 1, &mut out);
    out.sort();
    out
}

/// The last `count` lines of a log, oldest first.
pub fn last_log_lines(path: &std::path::Path, count: usize) -> Vec<String> {
    let Ok(text) = read_log_tail(path, 64 * 1024) else {
        return Vec::new();
    };
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(count)..]
        .iter()
        .map(|line| line.to_string())
        .collect()
}

/// The last `limit` bytes of a log, starting at a line boundary.
pub fn read_log_tail(path: &std::path::Path, limit: u64) -> io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = fs::File::open(path)?;
    let size = file.metadata()?.len();
    let start = size.saturating_sub(limit);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    // Drop the cut first line as bytes, before decoding: the cut may fall
    // inside a character. Each line is then decoded on its own, so one odd
    // line never changes how the others read.
    let whole = if start > 0 {
        match bytes.iter().position(|byte| *byte == b'\n') {
            Some(newline) => &bytes[newline + 1..],
            None => &bytes[..],
        }
    } else {
        &bytes[..]
    };
    let mut text = String::with_capacity(whole.len());
    for (index, line) in whole.split(|byte| *byte == b'\n').enumerate() {
        if index > 0 {
            text.push('\n');
        }
        text.push_str(&crate::message::decode_line(line));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_names_never_share_a_log_file() {
        let pairs = [
            ("#a/b", "#a_b"),
            ("alice[", "alice_"),
            ("a%5B", "a["),
            (".x", "x"),
            ("x.", "x"),
            ("#é", "#_"),
        ];
        for (one, two) in pairs {
            assert_ne!(safe_name(one), safe_name(two), "{one} {two}");
        }
        assert_eq!(safe_name("#rust"), "#rust");
        assert_eq!(safe_name("alice_"), "alice_");
        assert_eq!(safe_name("#a/b"), "#a%2Fb");
        assert_eq!(safe_name("con"), "%63on");
        assert_eq!(safe_name("COM1.chat"), "%43OM1.chat");
        assert_eq!(safe_name("../up"), "%2E.%2Fup");
        assert_eq!(safe_name(""), "window");
        for name in ["#a/b", "alice[", "#é ü", "con", ".x."] {
            assert_eq!(shown_log_name(&safe_name(name)), name);
        }
    }

    #[test]
    fn a_log_tail_cut_inside_a_letter_stays_utf8() {
        let dir = std::env::temp_dir().join(format!("openirc-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cut.log");
        std::fs::write(&path, "abc\néx\ncafé\n").unwrap();
        for limit in 1..14 {
            let tail = read_log_tail(&path, limit).unwrap();
            assert!(!tail.contains('Ã'), "{limit}: {tail:?}");
        }
        assert_eq!(read_log_tail(&path, 9).unwrap(), "café\n");
        // Trimming rewrites the file in UTF-8 too.
        for _ in 0..40 {
            append_log_file(&path, "naïve ünïcode line", 256).unwrap();
        }
        let kept = std::fs::read(&path).unwrap();
        assert!(
            String::from_utf8(kept)
                .unwrap()
                .ends_with("naïve ünïcode line\n")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let fresh = dir.join("fresh.log");
            append_log_file(&fresh, "x", 0).unwrap();
            let mode = std::fs::metadata(&fresh).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "{mode:o}");
            // An older log anyone could read is made private on its next line.
            let old = dir.join("old.log");
            std::fs::write(&old, "before\n").unwrap();
            std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o644)).unwrap();
            append_log_file(&old, "after", 0).unwrap();
            let mode = std::fs::metadata(&old).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{mode:o}");
            assert_eq!(std::fs::read_to_string(&old).unwrap(), "before\nafter\n");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn window_place_brackets_and_search_round_trip() {
        let mut settings = Settings::new();
        settings.window_place = Some([-8, 40, 1200, 800]);
        settings.window_maximized = true;
        settings.nick_open = String::new();
        settings.nick_close = ":".into();
        settings.nick_rank = true;
        settings.spell_input = false;
        settings.search_url = "https://example.com/?q=%s&x=1".into();
        settings.language = "de".into();
        let (parsed, warnings) = parse_settings(&encode_settings(&settings));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(parsed, settings);
        let (parsed, warnings) = parse_settings("window_place 0\t0\t10\t10\n");
        assert_eq!(parsed.window_place, None, "a tiny window is not restored");
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn settings_round_trip_and_alias_expansion() {
        let mut settings = Settings::new();
        settings.server = "irc.oftc.net".into();
        settings.port = 6667;
        settings.tls = false;
        settings.notify = vec!["alice".into()];
        settings.ignores = vec![crate::rules::IgnoreRule::all("spammer")];
        settings.beep = true;
        settings.sounds = true;
        settings.ident = true;
        settings.ident_user = "ada".into();
        settings.theme.attention = crate::theme::Rgb(1, 2, 3);
        settings
            .event_sounds
            .set("message", "/tmp/message.wav".into());
        settings.highlights = vec![
            crate::rules::HighlightRule::words("rust"),
            crate::rules::HighlightRule::words("good night"),
        ];
        settings.nick_colors.push(crate::rules::NickColorRule {
            mask: String::new(),
            modes: "@".into(),
            color: 4,
        });
        settings.auto_modes.push(crate::rules::AutoMode {
            mask: "friend".into(),
            channels: "#home".into(),
            network: String::new(),
            mode: 'v',
            protect: false,
        });
        settings.perform.push(Perform {
            server: "irc.libera.chat".into(),
            command: "/msg NickServ IDENTIFY secret".into(),
        });
        settings.flood_messages = 4;
        settings.flood_joins = 3;
        settings.flood_window_secs = 2;
        settings.proxy = true;
        settings.proxy_host = "127.0.0.1".into();
        settings.proxy_port = 9050;
        settings.favorites.push(Favorite {
            name: "Libera".into(),
            server: "irc.libera.chat".into(),
            port: 6697,
            tls: true,
            channels: "#openirc".into(),
        });
        settings.aliases.push(Alias {
            name: "hello".into(),
            body: "PRIVMSG #openirc :hello".into(),
        });
        let (mut parsed, warnings) = parse_settings(&encode_settings(&settings));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(parsed.server, "irc.oftc.net");
        assert_eq!(
            parsed.ignores,
            vec![crate::rules::IgnoreRule::all("spammer")]
        );
        assert_eq!(parsed.nick_colors, settings.nick_colors);
        assert_eq!(parsed.auto_modes, settings.auto_modes);
        assert!(parsed.beep);
        assert!(parsed.sounds);
        assert!(parsed.ident);
        assert_eq!(parsed.ident_user, "ada");
        assert_eq!(parsed.theme.attention, crate::theme::Rgb(1, 2, 3));
        assert_eq!(parsed.event_sounds.path("message"), "/tmp/message.wav");
        assert_eq!(parsed.highlights, settings.highlights);
        assert_eq!(parsed.perform[0].command, "/msg NickServ IDENTIFY secret");
        assert_eq!(parsed.flood_messages, 4);
        assert_eq!(parsed.flood_joins, 3);
        assert_eq!(parsed.flood_window_secs, 2);
        assert!(parsed.proxy);
        assert_eq!(parsed.proxy_host, "127.0.0.1");
        assert_eq!(parsed.proxy_port, 9050);
        assert_eq!(parsed.favorites[0].channels, "#openirc");
        let one = |line: &str| -> Vec<String> { vec![line.to_string()] };
        assert_eq!(
            parsed.expand_alias("/hello there"),
            Some(one("PRIVMSG #openirc :hello there"))
        );
        assert_eq!(parsed.expand_alias("plain"), None);
        parsed.aliases.push(Alias {
            name: "greet".into(),
            body: "say $1 hello $2-".into(),
        });
        assert_eq!(
            parsed.expand_alias("/greet Ann good night"),
            Some(one("say Ann hello good night"))
        );
        assert_eq!(parsed.expand_alias("/greet"), Some(one("say  hello ")));
        parsed.aliases.push(Alias {
            name: "price".into(),
            body: "cost $$1".into(),
        });
        assert_eq!(parsed.expand_alias("/price 9"), Some(one("cost $1")));
        parsed.aliases.push(Alias {
            name: "j".into(),
            body: "say $0-done".into(),
        });
        assert_eq!(parsed.expand_alias("/j hi"), Some(one("say j-done")));
        parsed.aliases.push(Alias {
            name: "z".into(),
            body: "say $0-".into(),
        });
        assert_eq!(parsed.expand_alias("/z a b"), Some(one("say z-")));
    }

    #[test]
    fn a_separator_in_alias_arguments_stays_text() {
        let mut settings = Settings::new();
        settings.aliases.push(Alias {
            name: "relay".into(),
            body: "/echo $1- | /echo done".into(),
        });
        settings.aliases.push(Alias {
            name: "plain".into(),
            body: "/echo a | /echo b".into(),
        });
        assert_eq!(
            settings.expand_alias("/relay hello | /echo second-command"),
            Some(vec![
                "/echo hello | /echo second-command".to_string(),
                "/echo done".to_string()
            ])
        );
        assert_eq!(
            settings.expand_alias("/plain x | /quit"),
            Some(vec!["/echo a".to_string(), "/echo b x | /quit".to_string()])
        );
    }

    #[test]
    fn every_text_field_survives_a_round_trip() {
        let mut settings = Settings::new();
        settings.realname = "tab\there \\n not a newline".into();
        settings.log_dir = r"C:\new\logs".into();
        settings.script_text = "on *:TEXT:*:#:{\n  echo a\\b\tc\n}".into();
        settings.sasl_user = "account".into();
        settings.favorites.push(Favorite {
            name: "No channel".into(),
            server: "irc.example".into(),
            port: 6697,
            tls: true,
            channels: String::new(),
        });
        settings.vars.push(("empty".into(), String::new()));
        settings.vars.push(("spaced".into(), "  two  ".into()));
        settings.remotes.push(crate::script::Remote {
            event: "text".into(),
            mask: "*hi*".into(),
            body: "/notice $nick hi".into(),
        });
        settings.popups.push(Popup {
            label: "Slap".into(),
            body: "/me slaps $nick".into(),
            menu: "line".into(),
        });
        settings.popups.push(Popup {
            label: "Op".into(),
            body: "/op $chan $nick".into(),
            menu: "nicklist".into(),
        });
        settings.dialogs.push(crate::dialog::Dialog::empty("demo"));
        let mut server = crate::servers::ServerEntry::new("Net", "irc.example", 6697, true);
        server.login = crate::servers::Login::NickServ;
        server.login_pass = "pa\tss".into();
        settings.servers.push(server);
        settings
            .trusted_certs
            .push(("self.example".into(), "ab12".into()));
        settings.client_cert = "/home/me/cert.pem".into();
        settings.palette[4] = [1, 2, 3];
        settings
            .sts_policies
            .push(("irc.example".into(), 6697, 1_900_000_000));
        settings.command_prefix = ".".into();
        settings.separator = ";".into();
        settings.drop_command = "/dcc send $nick $file".into();
        settings.message_window = true;
        settings.query_limit = 9;
        settings.flood_ctcp = 7;
        settings
            .dcc_folder_rules
            .push(("jpg png".into(), "Pictures".into()));
        settings.dcc_auto_accept.push("friend!*@*".into());
        settings
            .users
            .push(("10,friend".into(), "*!*@home.example".into()));
        settings.mono_windows.push("#xdcc".into());
        settings.last_windows.push((
            "irc.example".into(),
            vec!["#a".into(), "#b".into()],
            vec!["ann".into()],
        ));
        let mut with_files = settings.clone();
        with_files.scripts = vec![
            ScriptFile {
                name: "greet".into(),
                text: "alias hi {\n  echo hi\n}".into(),
                enabled: true,
            },
            ScriptFile {
                name: "off".into(),
                text: "alias no {\n  echo no\n}".into(),
                enabled: false,
            },
        ];
        with_files.rebuild_script();
        let (parsed_files, warnings) = parse_settings(&encode_settings(&with_files));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(parsed_files, with_files);
        assert_eq!(parsed_files.script_text, "alias hi {\n  echo hi\n}");
        let (parsed, warnings) = parse_settings(&encode_settings(&settings));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(parsed, settings);
    }

    #[test]
    fn one_bad_line_is_skipped_and_reported_and_the_rest_still_loads() {
        let text = "format 2\nserver irc.example\nport 66670\nnick keepme\n\
                    favorite Home\tirc.example\tnotaport\tfalse\t#a\n\
                    favorite Work\tirc.work\t6697\ttrue\t\n\
                    alias hi\t/me says hi\nmystery 1\n";
        let (parsed, warnings) = parse_settings(text);
        assert_eq!(parsed.server, "irc.example");
        assert_eq!(parsed.port, Settings::new().port);
        assert_eq!(parsed.nick, "keepme");
        assert_eq!(parsed.favorites.len(), 1);
        assert_eq!(parsed.favorites[0].name, "Work");
        assert_eq!(parsed.favorites[0].channels, "");
        assert_eq!(parsed.aliases.len(), 1);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings[0].contains("line 3"));
    }

    #[test]
    fn format_one_files_still_load() {
        let text = "server irc.example\nlog_dir C:\\new\\logs\n\
                    script_text alias a {\\n  echo hi\\n}\n\
                    favorite Old\tirc.example\t6667\tfalse\t\n";
        let (parsed, warnings) = parse_settings(text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(parsed.log_dir, "C:\\new\\logs");
        assert_eq!(parsed.script_text, "alias a {\n  echo hi\n}");
        assert_eq!(parsed.favorites.len(), 1);
    }

    #[test]
    fn damaged_settings_are_never_saved_over_without_a_backup() {
        let dir = std::env::temp_dir().join(format!("openirc-bak-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.txt");
        let original = b"format 2\nport invalid\nnick alice\n";
        fs::write(&path, original).unwrap();
        // The backup cannot be written: a folder has its name.
        fs::create_dir(dir.join("settings.txt.bak")).unwrap();
        let (settings, warnings) = Settings::load_from(&path);
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("not be saved"))
        );
        assert!(settings.save_to(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        // With a backup possible, saving goes ahead and the copy is kept.
        fs::remove_dir(dir.join("settings.txt.bak")).unwrap();
        let (settings, _) = Settings::load_from(&path);
        settings.save_to(&path).unwrap();
        assert_eq!(fs::read(dir.join("settings.txt.bak")).unwrap(), original);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn saves_at_the_same_time_each_leave_a_whole_file() {
        let dir = std::env::temp_dir().join(format!("openirc-race-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.txt");
        let snapshots: Vec<String> = (0..8)
            .map(|n| {
                let mut settings = Settings::new();
                settings.nick = format!("nick{n}{}", "x".repeat(n * 50));
                encode_settings(&settings)
            })
            .collect();
        let workers: Vec<_> = (0..8)
            .map(|n| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let mut settings = Settings::new();
                    settings.nick = format!("nick{n}{}", "x".repeat(n * 50));
                    for _ in 0..20 {
                        // Every save works, however they overlap.
                        settings.save_to(&path).unwrap();
                        // What is there at any moment is one whole save.
                        let now = fs::read_to_string(&path).unwrap();
                        assert!(
                            (0..8).any(|other| now == expected(other)),
                            "a mixed or partial file"
                        );
                    }
                })
            })
            .collect();
        fn expected(n: usize) -> String {
            let mut settings = Settings::new();
            settings.nick = format!("nick{n}{}", "x".repeat(n * 50));
            encode_settings(&settings)
        }
        for worker in workers {
            worker.join().unwrap();
        }
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            snapshots.contains(&text),
            "the file is exactly one of the saves"
        );
        let leftovers = fs::read_dir(&dir)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .is_ok_and(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            })
            .count();
        assert_eq!(leftovers, 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_mistyped_switch_is_reported_and_keeps_its_default() {
        let default = Settings::new().reconnect;
        let (parsed, warnings) = parse_settings("format 2\nreconnect tru\ntls true \n");
        assert_eq!(parsed.reconnect, default);
        assert!(parsed.tls, "spaces around the value are fine");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("not true or false"));
    }

    #[test]
    fn a_damaged_switch_inside_a_record_is_reported_not_read_as_false() {
        let mut settings = Settings::new();
        settings.servers.push(crate::servers::ServerEntry::new(
            "Example",
            "irc.example",
            6697,
            true,
        ));
        settings.favorites.push(Favorite {
            name: "Fav".into(),
            server: "irc.example".into(),
            port: 6697,
            tls: true,
            channels: "#room".into(),
        });
        settings.highlights.push(crate::rules::HighlightRule {
            words: "me".into(),
            sources: String::new(),
            on: crate::rules::MatchOn::Message,
            case: true,
            regex: false,
            color: None,
            sound: String::new(),
            flash: true,
            notice: false,
        });
        settings.auto_modes.push(crate::rules::AutoMode {
            mask: "friend".into(),
            channels: "#ops".into(),
            network: String::new(),
            mode: 'o',
            protect: true,
        });
        settings.scripts.push(ScriptFile {
            name: "one.oirc".into(),
            text: "alias x { echo x }".into(),
            enabled: true,
        });
        let text = encode_settings(&settings);
        let (clean, warnings) = parse_settings(&text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(clean.servers, settings.servers);
        for key in [
            "saved_server",
            "favorite",
            "highlight_rule",
            "auto_mode",
            "script_file",
        ] {
            for damage in ["tru", "true ", "TRUE", ""] {
                let damaged: String = text
                    .lines()
                    .map(|line| {
                        if line.starts_with(&format!("{key} ")) {
                            line.replacen("\ttrue", &format!("\t{damage}"), 1).replacen(
                                " true\t",
                                &format!(" {damage}\t"),
                                1,
                            )
                        } else {
                            line.to_string()
                        }
                    })
                    .map(|line| line + "\n")
                    .collect();
                assert_ne!(damaged, text, "{key}: nothing was damaged");
                let (parsed, warnings) = parse_settings(&damaged);
                assert_eq!(warnings.len(), 1, "{key} `{damage}`: {warnings:?}");
                assert!(warnings[0].contains(key), "{warnings:?}");
                let kept = match key {
                    "saved_server" => parsed.servers.len(),
                    "favorite" => parsed.favorites.len(),
                    "highlight_rule" => parsed.highlights.len(),
                    "auto_mode" => parsed.auto_modes.len(),
                    _ => parsed.scripts.len(),
                };
                assert_eq!(kept, 0, "{key} `{damage}` was read anyway");
            }
        }
    }

    #[test]
    fn a_locked_settings_value_refuses_to_save() {
        let settings = Settings {
            locked: true,
            ..Settings::new()
        };
        assert!(settings.save().is_err());
    }

    #[test]
    fn log_files_by_network_date_and_size() {
        let dir = std::env::temp_dir().join(format!("openirc-logs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let dir_text = dir.display().to_string();
        let now = crate::clock::LocalTime {
            year: 2026,
            month: 3,
            day: 7,
            hour: 1,
            minute: 2,
            second: 3,
        };
        let path = log_file(&dir_text, Some("Libera/Chat"), "#rust", "day", &now);
        assert!(
            path.ends_with("Libera%2FChat/#rust.2026-03-07.log"),
            "{path:?}"
        );
        let monthly = log_file(&dir_text, None, "alice", "month", &now);
        assert!(monthly.ends_with("alice.2026-03.log"));
        for n in 0..200 {
            append_log_file(&path, &format!("line {n:03} with some padding"), 2_000).unwrap();
        }
        let size = fs::metadata(&path).unwrap().len();
        assert!(size <= 2_000, "trimmed to {size}");
        let last = last_log_lines(&path, 2);
        assert_eq!(
            last,
            vec!["line 198 with some padding", "line 199 with some padding"]
        );
        assert_eq!(list_logs(&dir_text), vec![path.clone()]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn log_paths_keep_channel_prefixes_and_stay_in_the_folder() {
        let now = crate::clock::now();
        let log_path = |dir: &str, window: &str| log_file(dir, None, window, "none", &now);
        assert!(log_path("/logs", "#rust").ends_with("#rust.log"));
        assert_ne!(log_path("/logs", "#rust"), log_path("/logs", "_rust"));
        let up = log_path("/logs", "../../etc/passwd");
        assert_eq!(up.parent().unwrap(), std::path::Path::new("/logs"));
        assert_eq!(up.file_name().unwrap(), "%2E.%2F..%2Fetc%2Fpasswd.log");
        assert_eq!(log_path("/logs", "..").file_name().unwrap(), "%2E%2E.log");
        let network = log_file("/logs", Some(".."), "#rust", "none", &now);
        assert_eq!(network, std::path::Path::new("/logs/%2E%2E/#rust.log"));
    }
}
