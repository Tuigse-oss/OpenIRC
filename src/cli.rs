use crate::pump::valid_nick;

pub const BIN: &str = "openirc";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub nick: String,
    pub user: String,
    pub realname: String,
    pub channels: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ArgsError {
    Help(String),
    Version(String),
    Invalid(String),
}

pub fn parse_args(args: impl IntoIterator<Item = impl Into<String>>) -> Result<Args, ArgsError> {
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    let mut host: Option<String> = None;
    let mut port: Option<u16> = None;
    let mut tls = false;
    let mut nick: Option<String> = None;
    let mut user: Option<String> = None;
    let mut realname: Option<String> = None;
    let mut channels = Vec::new();
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--help" | "-h" => return Err(ArgsError::Help(help_text())),
            "--version" | "-V" => {
                return Err(ArgsError::Version(format!(
                    "{BIN} {}",
                    env!("CARGO_PKG_VERSION")
                )));
            }
            "--tls" => tls = true,
            "--server" | "--port" | "--nick" | "--user" | "--realname" | "--channel" | "--join" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| ArgsError::Invalid(format!("missing value for {arg}")))?;
                match arg.as_str() {
                    "--server" => host = Some(split_assign(value, &mut port)?),
                    "--port" => {
                        port = Some(parse_port(value)?);
                    }
                    "--nick" => nick = Some(value.clone()),
                    "--user" => user = Some(value.clone()),
                    "--realname" => realname = Some(value.clone()),
                    "--channel" | "--join" => channels.push(value.clone()),
                    _ => unreachable!(),
                }
                index += 2;
                continue;
            }
            other if other.starts_with('-') => {
                return Err(ArgsError::Invalid(format!("unknown option {other}")));
            }
            other => {
                if host.is_some() {
                    return Err(ArgsError::Invalid(format!("unexpected argument {other}")));
                }
                host = Some(split_assign(other, &mut port)?);
            }
        }
        index += 1;
    }

    let host = host.ok_or_else(|| {
        ArgsError::Invalid("missing server. Example: openirc --tls irc.libera.chat".into())
    })?;
    if host.is_empty() {
        return Err(ArgsError::Invalid("server name is empty".into()));
    }
    let nick = nick.unwrap_or_else(default_nick);
    if !valid_nick(&nick) {
        return Err(ArgsError::Invalid(format!(
            "nick `{nick}` must be 1-16 characters and start with a letter"
        )));
    }
    let user = user.unwrap_or_else(|| nick.to_ascii_lowercase());
    if user.is_empty() || user.contains(char::is_whitespace) {
        return Err(ArgsError::Invalid("user name must be one word".into()));
    }
    let realname = realname.unwrap_or_else(|| format!("{} user", crate::NAME));
    let port = port.unwrap_or(if tls { 6697 } else { 6667 });

    Ok(Args {
        host,
        port,
        tls,
        nick,
        user,
        realname,
        channels,
    })
}

fn split_assign(value: &str, port: &mut Option<u16>) -> Result<String, ArgsError> {
    let (host, parsed) = split_host_port(value);
    if let Some(parsed) = parsed {
        if port.is_some() {
            return Err(ArgsError::Invalid("port was given more than once".into()));
        }
        *port = Some(parsed);
    }
    if host.is_empty() {
        return Err(ArgsError::Invalid("server name is empty".into()));
    }
    Ok(host)
}

fn split_host_port(value: &str) -> (String, Option<u16>) {
    if let Some(rest) = value.strip_prefix('[') {
        if let Some((host, port)) = rest.split_once("]:")
            && let Ok(port) = port.parse::<u16>()
        {
            return (host.to_string(), Some(port));
        }
        if let Some(host) = rest.strip_suffix(']') {
            return (host.to_string(), None);
        }
    }
    if value.chars().filter(|ch| *ch == ':').count() == 1
        && let Some((host, port)) = value.rsplit_once(':')
        && let Ok(port) = port.parse::<u16>()
    {
        return (host.to_string(), Some(port));
    }
    (value.to_string(), None)
}

fn parse_port(value: &str) -> Result<u16, ArgsError> {
    value
        .parse::<u16>()
        .map_err(|_| ArgsError::Invalid(format!("invalid port `{value}`")))
}

fn default_nick() -> String {
    format!("oi{}", std::process::id() % 100_000)
}

fn help_text() -> String {
    format!(
        "\
{bin} {version}
Open a window, or pass --cli for a line console.

Usage:
  {bin}
  {bin} [options] <server[:port]>
  {bin} --cli [options] <server[:port]>

Options:
  --cli                 Stay in the terminal instead of opening a window.
  --tls                 Connect with TLS. Default port becomes 6697.
  --nick <name>         Nickname. Default is oi followed by a process number.
  --user <name>         Username sent in USER. Defaults to the nickname.
  --realname <text>     Real name sent in USER.
  --channel <channel>   Channel to join after registration. Repeatable.
  --join <channel>      Same as --channel.
  --help                Show this help.
  --version             Show the version.

Examples:
  {bin} --tls --channel '#openirc' irc.libera.chat
  {bin} --nick bob --port 6667 irc.oftc.net
",
        bin = BIN,
        version = env!("CARGO_PKG_VERSION"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tls_host_and_channel() {
        let args = parse_args([
            "--tls",
            "--channel",
            "#openirc",
            "--nick",
            "Bob",
            "irc.libera.chat",
        ])
        .unwrap();
        assert_eq!(
            args,
            Args {
                host: "irc.libera.chat".into(),
                port: 6697,
                tls: true,
                nick: "Bob".into(),
                user: "bob".into(),
                realname: "OpenIRC user".into(),
                channels: vec!["#openirc".into()],
            }
        );
    }

    #[test]
    fn splits_a_plain_port_and_bracketed_ipv6() {
        let plain = parse_args(["irc.oftc.net:6668", "--nick", "bob"]).unwrap();
        assert_eq!(plain.host, "irc.oftc.net");
        assert_eq!(plain.port, 6668);
        assert!(!plain.tls);

        let ipv6 = parse_args(["[2001:db8::1]:6667", "--nick", "bob"]).unwrap();
        assert_eq!(ipv6.host, "2001:db8::1");
        assert_eq!(ipv6.port, 6667);
    }

    #[test]
    fn help_and_missing_server_are_distinct() {
        assert!(matches!(parse_args(["--help"]), Err(ArgsError::Help(_))));
        assert!(matches!(
            parse_args(Vec::<String>::new()),
            Err(ArgsError::Invalid(_))
        ));
    }
}
