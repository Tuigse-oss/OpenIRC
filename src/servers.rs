//! Saved servers, grouped by network, with per-server login, encoding, and
//! auto-join channels.

/// How OpenIRC logs in to services after connecting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Login {
    #[default]
    None,
    /// SASL with a user name and password.
    SaslPlain,
    /// SASL with the client TLS certificate (CertFP).
    SaslExternal,
    /// `PRIVMSG NickServ :IDENTIFY` after registration.
    NickServ,
}

impl Login {
    pub const ALL: [Login; 4] = [
        Login::None,
        Login::SaslPlain,
        Login::SaslExternal,
        Login::NickServ,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Login::None => "None",
            Login::SaslPlain => "SASL (user name and password)",
            Login::SaslExternal => "SASL with my certificate",
            Login::NickServ => "NickServ IDENTIFY",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Login::None => "none",
            Login::SaslPlain => "sasl",
            Login::SaslExternal => "external",
            Login::NickServ => "nickserv",
        }
    }

    pub fn from_key(key: &str) -> Login {
        match key {
            "sasl" => Login::SaslPlain,
            "external" => Login::SaslExternal,
            "nickserv" => Login::NickServ,
            _ => Login::None,
        }
    }
}

/// Text encodings offered for a server. `auto` reads UTF-8 and falls back
/// to Windows-1252 for lines that are not valid UTF-8, and sends UTF-8.
pub const ENCODINGS: [&str; 8] = [
    "auto",
    "utf-8",
    "windows-1252",
    "iso-8859-1",
    "iso-8859-15",
    "windows-1251",
    "koi8-r",
    "iso-8859-2",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerEntry {
    /// The group the server belongs to, such as `Libera.Chat`.
    pub network: String,
    pub description: String,
    pub host: String,
    pub port: u16,
    pub tls: bool,
    /// Server password, sent as `PASS` before registration.
    pub password: String,
    pub login: Login,
    pub login_user: String,
    pub login_pass: String,
    /// Nick for this server; blank uses the global nick.
    pub nick: String,
    pub alt_nick: String,
    pub encoding: String,
    /// Channels to join, separated by commas or spaces.
    pub channels: String,
}

impl ServerEntry {
    pub fn new(network: &str, host: &str, port: u16, tls: bool) -> Self {
        Self {
            network: network.to_string(),
            description: String::new(),
            host: host.to_string(),
            port,
            tls,
            password: String::new(),
            login: Login::None,
            login_user: String::new(),
            login_pass: String::new(),
            nick: String::new(),
            alt_nick: String::new(),
            encoding: "auto".into(),
            channels: String::new(),
        }
    }

    /// What the list shows for this server.
    pub fn label(&self) -> String {
        if self.description.trim().is_empty() {
            format!(
                "{}:{}{}",
                self.host,
                self.port,
                if self.tls { " (TLS)" } else { "" }
            )
        } else {
            self.description.clone()
        }
    }

    pub fn channel_list(&self) -> Vec<String> {
        split_channels(&self.channels)
    }

    /// The settings-file fields, in order.
    pub fn fields(&self) -> Vec<String> {
        vec![
            self.network.clone(),
            self.description.clone(),
            self.host.clone(),
            self.port.to_string(),
            self.tls.to_string(),
            self.password.clone(),
            self.login.key().into(),
            self.login_user.clone(),
            self.login_pass.clone(),
            self.nick.clone(),
            self.alt_nick.clone(),
            self.encoding.clone(),
            self.channels.clone(),
        ]
    }

    pub fn from_fields(fields: &[String]) -> Option<Self> {
        if fields.len() != 13 {
            return None;
        }
        Some(Self {
            network: fields[0].clone(),
            description: fields[1].clone(),
            host: fields[2].clone(),
            port: fields[3].trim().parse().ok()?,
            tls: crate::settings::parse_flag(&fields[4])?,
            password: fields[5].clone(),
            login: Login::from_key(&fields[6]),
            login_user: fields[7].clone(),
            login_pass: fields[8].clone(),
            nick: fields[9].clone(),
            alt_nick: fields[10].clone(),
            encoding: if fields[11].is_empty() {
                "auto".into()
            } else {
                fields[11].clone()
            },
            channels: fields[12].clone(),
        })
    }
}

/// Channels typed as `#a, #b #c` (a key may follow a channel after a space
/// only when commas separate the channels: `#a key, #b`).
pub fn split_channels(text: &str) -> Vec<String> {
    if text.contains(',') {
        text.split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        text.split_whitespace().map(str::to_string).collect()
    }
}

/// The next server to try after `current` fails: the next entry of the same
/// network, wrapping around; `None` when it is the only one.
pub fn next_in_group<'a>(
    servers: &'a [ServerEntry],
    current: &ServerEntry,
) -> Option<&'a ServerEntry> {
    let group: Vec<&ServerEntry> = servers
        .iter()
        .filter(|entry| {
            !current.network.is_empty() && entry.network.eq_ignore_ascii_case(&current.network)
        })
        .collect();
    if group.len() < 2 {
        return None;
    }
    let at = group
        .iter()
        .position(|entry| entry.host == current.host && entry.port == current.port)
        .unwrap_or(group.len() - 1);
    Some(group[(at + 1) % group.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_round_trip() {
        let mut entry = ServerEntry::new("Libera.Chat", "irc.libera.chat", 6697, true);
        entry.login = Login::SaslExternal;
        entry.channels = "#rust, #openirc".into();
        entry.encoding = "windows-1251".into();
        assert_eq!(
            ServerEntry::from_fields(&entry.fields()),
            Some(entry.clone())
        );
        assert_eq!(
            entry.channel_list(),
            vec!["#rust".to_string(), "#openirc".to_string()]
        );
        assert!(ServerEntry::from_fields(&["x".into()]).is_none());
    }

    #[test]
    fn channels_with_keys_and_spaces() {
        assert_eq!(
            split_channels("#a key, #b"),
            vec!["#a key".to_string(), "#b".to_string()]
        );
        assert_eq!(
            split_channels("#a #b"),
            vec!["#a".to_string(), "#b".to_string()]
        );
    }

    #[test]
    fn next_server_wraps_inside_the_group() {
        let servers = vec![
            ServerEntry::new("Net", "a.net", 6697, true),
            ServerEntry::new("Other", "x.org", 6667, false),
            ServerEntry::new("Net", "b.net", 6697, true),
        ];
        assert_eq!(next_in_group(&servers, &servers[0]).unwrap().host, "b.net");
        assert_eq!(next_in_group(&servers, &servers[2]).unwrap().host, "a.net");
        assert!(next_in_group(&servers, &servers[1]).is_none());
    }
}
