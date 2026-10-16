//! IRC networks for the connect window.
//!
//! `builtin` is a short list of current networks. `parse_directory` reads a
//! JSON directory of networks, and `fetch_directory` downloads one. A network
//! with a TLS server is preferred over a plain one.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub const DIRECTORY_HOST: &str = "raw.githubusercontent.com";
pub const DIRECTORY_PATH: &str = "/rahatarmanahmed/irc-networks/master/networks.json";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Network {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub tls: bool,
}

pub fn builtin() -> Vec<Network> {
    vec![
        net("DALnet", "irc.dal.net", 6697, true),
        net("EFnet", "irc.efnet.org", 6697, true),
        net("EsperNet", "irc.esper.net", 6697, true),
        net("Hackint", "irc.hackint.org", 6697, true),
        net("IRCNow", "irc.ircnow.org", 6697, true),
        net("Libera.Chat", "irc.libera.chat", 6697, true),
        net("OFTC", "irc.oftc.net", 6697, true),
        net("QuakeNet", "irc.quakenet.org", 6667, false),
        net("Rizon", "irc.rizon.net", 6697, true),
        net("Snoonet", "irc.snoonet.org", 6697, true),
        net("Undernet", "irc.undernet.org", 6667, false),
        net("tilde.chat", "irc.tilde.chat", 6697, true),
    ]
}

pub fn merge(base: Vec<Network>, extra: Vec<Network>) -> Vec<Network> {
    let mut out = base;
    for net in extra {
        if out
            .iter()
            .any(|item| item.name.eq_ignore_ascii_case(&net.name))
        {
            continue;
        }
        out.push(net);
    }
    out.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });
    out
}

pub fn parse_directory(json: &str) -> Vec<Network> {
    let mut networks = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    let mut index = 0;
    let chars: Vec<char> = json.chars().collect();
    while index < chars.len() {
        match chars[index] {
            '"' => index = skip_string(&chars, index),
            '{' => {
                if depth == 0 {
                    start = index;
                }
                depth += 1;
                index += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0
                    && let Some(network) =
                        parse_network(&chars[start..=index].iter().collect::<String>())
                {
                    networks.push(network);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    networks
}

pub fn fetch_directory() -> io::Result<Vec<Network>> {
    let body = https_get(DIRECTORY_HOST, DIRECTORY_PATH)?;
    let networks = parse_directory(&body);
    if networks.is_empty() {
        return Err(io::Error::other("the network directory was empty"));
    }
    Ok(networks)
}

fn net(name: &str, host: &str, port: u16, tls: bool) -> Network {
    Network {
        name: name.to_string(),
        host: host.to_string(),
        port,
        tls,
    }
}

fn parse_network(object: &str) -> Option<Network> {
    let name = json_string(object, "name")?;
    let mut best: Option<Server> = None;
    let mut cursor = 0;
    while let Some(host_at) = object[cursor..].find("\"hostname\"") {
        let at = cursor + host_at;
        let host = json_string(&object[at..], "hostname")?;
        let window = &object[at..];
        let port = json_number(window, "port").unwrap_or(6667);
        let tls = json_bool(window, "ssl").unwrap_or(false);
        let main = json_bool(window, "isMainServer").unwrap_or(false);
        let server = Server {
            host,
            port,
            tls,
            main,
        };
        best = Some(match best {
            None => server,
            Some(current) => prefer(current, server),
        });
        cursor = at + "\"hostname\"".len();
    }
    let server = best?;
    if name.is_empty() || server.host.is_empty() {
        return None;
    }
    Some(Network {
        name,
        host: server.host,
        port: server.port,
        tls: server.tls,
    })
}

struct Server {
    host: String,
    port: u16,
    tls: bool,
    main: bool,
}

fn prefer(current: Server, next: Server) -> Server {
    fn rank(server: &Server) -> u8 {
        u8::from(server.tls) * 2 + u8::from(server.main)
    }
    if rank(&next) > rank(&current) {
        next
    } else {
        current
    }
}

fn json_string(text: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{key}\"");
    let at = text.find(&pattern)? + pattern.len();
    let rest = text[at..].trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(escaped) = chars.next() {
                out.push(escaped);
            }
            continue;
        }
        if ch == '"' {
            break;
        }
        out.push(ch);
    }
    Some(out)
}

fn json_number(text: &str, key: &str) -> Option<u16> {
    let pattern = format!("\"{key}\"");
    let at = text.find(&pattern)? + pattern.len();
    let rest = text[at..].trim_start().strip_prefix(':')?.trim_start();
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn json_bool(text: &str, key: &str) -> Option<bool> {
    let pattern = format!("\"{key}\"");
    let at = text.find(&pattern)? + pattern.len();
    let rest = text[at..].trim_start().strip_prefix(':')?.trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn skip_string(chars: &[char], start: usize) -> usize {
    let mut index = start + 1;
    while index < chars.len() {
        if chars[index] == '\\' {
            index += 2;
            continue;
        }
        if chars[index] == '"' {
            return index + 1;
        }
        index += 1;
    }
    chars.len()
}

fn https_get(host: &str, path: &str) -> io::Result<String> {
    let tcp = TcpStream::connect((host, 443))?;
    tcp.set_nodelay(true)?;
    tcp.set_read_timeout(Some(Duration::from_secs(20)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(20)))?;
    let (mut stream, _) = crate::tls::wrap(host, tcp, &crate::tls::TlsOptions::default())?;
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock).map_err(|err| {
            io::Error::new(
                err.kind(),
                format!("TLS handshake with {host} failed: {err}"),
            )
        })?;
    }
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nUser-Agent: OpenIRC\r\n\r\n"
    );
    stream.write_all(request.as_bytes())?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| io::Error::other("network directory response had no body"))?;
    let status = head.lines().next().unwrap_or("");
    if !status.contains(" 200 ") {
        return Err(io::Error::other(format!(
            "network directory returned {status}"
        )));
    }
    Ok(body.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_prefers_the_tls_server_and_keeps_builtin_names() {
        let json = r#"
        [
          {"name":"Example","avgUsers":3,"servers":[
            {"hostname":"irc.example","port":6667,"ssl":false,"isMainServer":true},
            {"hostname":"irc.example","port":6697,"ssl":true,"isMainServer":false}
          ]},
          {"name":"Libera.Chat","servers":[
            {"hostname":"old.example","port":6667,"ssl":false,"isMainServer":true}
          ]}
        ]
        "#;
        let parsed = parse_directory(json);
        assert_eq!(parsed[0].host, "irc.example");
        assert_eq!(parsed[0].port, 6697);
        assert!(parsed[0].tls);
        let merged = merge(builtin(), parsed);
        let libera = merged.iter().find(|net| net.name == "Libera.Chat").unwrap();
        assert_eq!(libera.host, "irc.libera.chat");
        assert!(merged.iter().any(|net| net.name == "Example"));
    }
}
