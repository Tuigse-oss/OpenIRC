//! SOCKS5 connect, with no authentication.
//!
//! `dial` opens a TCP connection to the proxy, sends the greeting from
//! `greeting`, then the domain connect request from `connect_request`.
//! After a successful reply the returned stream speaks to the destination.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub fn greeting() -> [u8; 3] {
    [0x05, 0x01, 0x00]
}

pub fn auth_greeting() -> [u8; 3] {
    [0x05, 0x01, 0x02]
}

pub fn socks4_request(host: &str, port: u16, user: &str) -> Vec<u8> {
    let mut out = vec![0x04, 0x01];
    out.extend(port.to_be_bytes());
    out.extend([0, 0, 0, 1]);
    out.extend(user.as_bytes());
    out.push(0);
    out.extend(host.as_bytes());
    out.push(0);
    out
}

pub fn http_connect(host: &str, port: u16, user: &str, pass: &str) -> String {
    let mut request = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
    if !user.is_empty() {
        request.push_str(&format!(
            "Proxy-Authorization: Basic {}\r\n",
            base64(format!("{user}:{pass}").as_bytes())
        ));
    }
    request.push_str("\r\n");
    request
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub kind: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
}

impl Endpoint {
    pub fn connect(&self, host: &str, port: u16) -> io::Result<TcpStream> {
        self.connect_with(host, port, &crate::net::prefs())
    }

    /// Reach `host` through the proxy. The proxy itself is dialed like any
    /// server: from the chosen local address, failing rather than using
    /// another one unless fallback is allowed. `host` goes to the proxy as
    /// a name, so the proxy looks it up.
    pub fn connect_with(
        &self,
        host: &str,
        port: u16,
        prefs: &crate::net::NetPrefs,
    ) -> io::Result<TcpStream> {
        self.connect_within(host, port, prefs, HANDSHAKE_LIMIT)
    }

    fn connect_within(
        &self,
        host: &str,
        port: u16,
        prefs: &crate::net::NetPrefs,
        limit: Duration,
    ) -> io::Result<TcpStream> {
        let timeout = Duration::from_secs(30);
        let mut stream = crate::net::dial_with(&self.host, self.port, timeout, prefs)
            .map_err(|err| io::Error::new(err.kind(), format!("{} proxy: {err}", self.kind)))?;
        stream.set_nodelay(true)?;
        {
            // However slowly the proxy answers, the whole exchange ends in
            // time.
            let mut timed = Timed {
                stream: &mut stream,
                until: Instant::now() + limit,
            };
            match self.kind.as_str() {
                "socks4" => socks4_handshake(&mut timed, host, port, &self.user)?,
                "http" => http_handshake(&mut timed, host, port, &self.user, &self.pass)?,
                _ => handshake(&mut timed, host, port, &self.user, &self.pass)?,
            }
        }
        stream.set_read_timeout(Some(Duration::from_secs(20)))?;
        stream.set_write_timeout(Some(Duration::from_secs(20)))?;
        Ok(stream)
    }
}

/// The most a proxy handshake may take in all.
const HANDSHAKE_LIMIT: Duration = Duration::from_secs(20);

/// A stream whose reads and writes all end by `until`, not each within a
/// time of its own.
struct Timed<'a> {
    stream: &'a mut TcpStream,
    until: Instant,
}

fn too_slow() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "the proxy took too long to answer")
}

/// A read or write that ran out of time: Unix says "would block", Windows
/// "timed out"; either way it is the deadline.
fn timed<T>(result: io::Result<T>) -> io::Result<T> {
    result.map_err(|err| match err.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => too_slow(),
        _ => err,
    })
}

impl Timed<'_> {
    fn left(&self) -> io::Result<Duration> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(too_slow());
        }
        Ok(left)
    }
}

impl Read for Timed<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.left()?;
        self.stream.set_read_timeout(Some(left))?;
        timed(self.stream.read(buf))
    }
}

impl Write for Timed<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let left = self.left()?;
        self.stream.set_write_timeout(Some(left))?;
        timed(self.stream.write(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

pub fn base64(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut index = 0;
    while index + 3 <= data.len() {
        let chunk = u32::from(data[index]) << 16
            | u32::from(data[index + 1]) << 8
            | u32::from(data[index + 2]);
        out.push(TABLE[((chunk >> 18) & 63) as usize] as char);
        out.push(TABLE[((chunk >> 12) & 63) as usize] as char);
        out.push(TABLE[((chunk >> 6) & 63) as usize] as char);
        out.push(TABLE[(chunk & 63) as usize] as char);
        index += 3;
    }
    let rest = data.len() - index;
    if rest == 1 {
        let chunk = u32::from(data[index]) << 16;
        out.push(TABLE[((chunk >> 18) & 63) as usize] as char);
        out.push(TABLE[((chunk >> 12) & 63) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rest == 2 {
        let chunk = u32::from(data[index]) << 16 | u32::from(data[index + 1]) << 8;
        out.push(TABLE[((chunk >> 18) & 63) as usize] as char);
        out.push(TABLE[((chunk >> 12) & 63) as usize] as char);
        out.push(TABLE[((chunk >> 6) & 63) as usize] as char);
        out.push('=');
    }
    out
}

pub fn connect_request(host: &str, port: u16) -> io::Result<Vec<u8>> {
    let bytes = host.as_bytes();
    if bytes.is_empty() || bytes.len() > 255 {
        return Err(io::Error::other(
            "SOCKS5 destination host must be 1 to 255 bytes",
        ));
    }
    let mut out = Vec::with_capacity(7 + bytes.len());
    out.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, bytes.len() as u8]);
    out.extend_from_slice(bytes);
    out.extend_from_slice(&port.to_be_bytes());
    Ok(out)
}

fn handshake(
    stream: &mut (impl Read + Write),
    host: &str,
    port: u16,
    user: &str,
    pass: &str,
) -> io::Result<()> {
    if user.is_empty() {
        stream.write_all(&greeting())?;
    } else {
        stream.write_all(&auth_greeting())?;
    }
    stream.flush()?;
    let mut method = [0u8; 2];
    stream.read_exact(&mut method)?;
    let expected = if user.is_empty() { 0x00 } else { 0x02 };
    if method != [0x05, expected] {
        return Err(io::Error::other(format!(
            "SOCKS5 proxy rejected authentication ({:02x} {:02x})",
            method[0], method[1]
        )));
    }
    if !user.is_empty() {
        let mut auth = vec![0x01, user.len() as u8];
        auth.extend(user.as_bytes());
        auth.push(pass.len() as u8);
        auth.extend(pass.as_bytes());
        stream.write_all(&auth)?;
        stream.flush()?;
        let mut status = [0u8; 2];
        stream.read_exact(&mut status)?;
        if status != [0x01, 0x00] {
            return Err(io::Error::other("SOCKS5 username was rejected"));
        }
    }
    stream.write_all(&connect_request(host, port)?)?;
    stream.flush()?;
    read_reply(stream)
}

fn socks4_handshake(
    stream: &mut (impl Read + Write),
    host: &str,
    port: u16,
    user: &str,
) -> io::Result<()> {
    stream.write_all(&socks4_request(host, port, user))?;
    stream.flush()?;
    let mut reply = [0u8; 8];
    stream.read_exact(&mut reply)?;
    if reply[1] != 0x5a {
        return Err(io::Error::other(format!(
            "SOCKS4 connect failed with code {}",
            reply[1]
        )));
    }
    Ok(())
}

fn http_handshake(
    stream: &mut (impl Read + Write),
    host: &str,
    port: u16,
    user: &str,
    pass: &str,
) -> io::Result<()> {
    stream.write_all(http_connect(host, port, user, pass).as_bytes())?;
    stream.flush()?;
    let head = read_http_head(stream)?;
    let status = head.lines().next().unwrap_or("").trim().to_string();
    if http_status(&status) != Some(200) {
        return Err(io::Error::other(format!(
            "HTTP proxy rejected CONNECT: {status}"
        )));
    }
    Ok(())
}

/// Longest proxy reply header accepted.
const HTTP_HEAD_LIMIT: usize = 8 * 1024;

/// Read the proxy's reply up to and including the blank line, one byte at
/// a time, so bytes the server sends right after it stay in the stream for
/// the IRC or DCC connection.
fn read_http_head(stream: &mut impl Read) -> io::Result<String> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if stream.read(&mut byte)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "HTTP proxy closed the connection",
            ));
        }
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") || head.ends_with(b"\n\n") {
            return Ok(String::from_utf8_lossy(&head).into_owned());
        }
        if head.len() > HTTP_HEAD_LIMIT {
            return Err(io::Error::other("HTTP proxy reply is too long"));
        }
    }
}

/// The status code of a line like `HTTP/1.1 200 Connection established`.
fn http_status(line: &str) -> Option<u16> {
    let mut words = line.split_whitespace();
    words
        .next()
        .filter(|version| version.starts_with("HTTP/"))?;
    words.next()?.parse().ok()
}

fn read_reply(stream: &mut impl Read) -> io::Result<()> {
    let mut head = [0u8; 4];
    stream.read_exact(&mut head)?;
    if head[0] != 0x05 {
        return Err(io::Error::other("SOCKS5 reply had an unexpected version"));
    }
    if head[1] != 0x00 {
        return Err(io::Error::other(format!(
            "SOCKS5 connect failed with code {}",
            head[1]
        )));
    }
    let rest = match head[3] {
        0x01 => 4 + 2,
        0x04 => 16 + 2,
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len)?;
            len[0] as usize + 2
        }
        other => {
            return Err(io::Error::other(format!(
                "SOCKS5 reply had address type {other}"
            )));
        }
    };
    let mut bound = vec![0u8; rest];
    stream.read_exact(&mut bound)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    use super::*;

    #[test]
    fn greeting_and_domain_request() {
        assert_eq!(greeting(), [5, 1, 0]);
        assert_eq!(auth_greeting(), [5, 1, 2]);
        assert_eq!(base64(b"ada:secret"), "YWRhOnNlY3JldA==");
        assert!(
            http_connect("irc.example", 6697, "ada", "secret").contains("Basic YWRhOnNlY3JldA==")
        );
        assert_eq!(socks4_request("irc.example", 6667, "ada")[0], 4);
        assert_eq!(
            connect_request("irc.example", 6667).unwrap(),
            [
                5, 1, 0, 3, 11, b'i', b'r', b'c', b'.', b'e', b'x', b'a', b'm', b'p', b'l', b'e',
                0x1a, 0x0b
            ]
        );
    }

    #[test]
    fn dial_completes_a_no_auth_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            sock.set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut hello = [0u8; 3];
            reader.read_exact(&mut hello).unwrap();
            assert_eq!(hello, greeting());
            sock.write_all(&[0x05, 0x00]).unwrap();
            sock.flush().unwrap();
            let mut request = vec![0u8; connect_request("irc.example", 6667).unwrap().len()];
            reader.read_exact(&mut request).unwrap();
            assert_eq!(request, connect_request("irc.example", 6667).unwrap());
            sock.write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .unwrap();
            sock.flush().unwrap();
            let mut ping = Vec::new();
            reader.read_until(b'\n', &mut ping).unwrap();
            ping
        });

        let mut stream = Endpoint {
            kind: "socks5".into(),
            host: proxy_addr.ip().to_string(),
            port: proxy_addr.port(),
            user: String::new(),
            pass: String::new(),
        }
        .connect("irc.example", 6667)
        .unwrap();
        stream.write_all(b"PING :openirc\r\n").unwrap();
        stream.flush().unwrap();
        let seen = server.join().unwrap();
        assert_eq!(seen, b"PING :openirc\r\n");
    }

    #[test]
    fn the_proxy_is_reached_only_from_the_chosen_local_address() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        let proxy = Endpoint {
            kind: "socks5".into(),
            host: proxy_addr.ip().to_string(),
            port: proxy_addr.port(),
            user: String::new(),
            pass: String::new(),
        };
        // An address this machine does not have, and one of the other
        // family; neither may fall back to another address.
        for bind in ["192.0.2.1", "::1"] {
            let strict = crate::net::NetPrefs {
                bind: Some(bind.parse().unwrap()),
                bind_fallback: false,
                ..crate::net::NetPrefs::default()
            };
            let err = proxy
                .connect_with("irc.example", 6667, &strict)
                .unwrap_err();
            assert!(err.to_string().contains("socks5 proxy"), "{err}");
            thread::sleep(Duration::from_millis(50));
            assert!(
                listener.accept().is_err(),
                "{bind}: the proxy saw a connection"
            );
        }
        // With fallback allowed the proxy is reached.
        let loose = crate::net::NetPrefs {
            bind: Some("192.0.2.1".parse().unwrap()),
            bind_fallback: true,
            ..crate::net::NetPrefs::default()
        };
        let reached = thread::spawn(move || {
            let _ = proxy.connect_with("irc.example", 6667, &loose);
        });
        let started = std::time::Instant::now();
        let mut accepted = false;
        while started.elapsed() < Duration::from_secs(5) {
            if let Ok((sock, _)) = listener.accept() {
                accepted = true;
                drop(sock);
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(accepted);
        let _ = reached.join();
    }

    #[test]
    fn a_proxy_that_answers_a_byte_at_a_time_still_ends_in_time() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            // A reply that never ends, each byte well inside any one read's
            // time limit.
            for _ in 0..40 {
                if sock.write_all(b"X").is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
        });
        let proxy = Endpoint {
            kind: "http".into(),
            host: proxy_addr.ip().to_string(),
            port: proxy_addr.port(),
            user: String::new(),
            pass: String::new(),
        };
        let started = std::time::Instant::now();
        let err = proxy
            .connect_within(
                "irc.example",
                6667,
                &crate::net::NetPrefs::default(),
                Duration::from_millis(400),
            )
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut, "{err}");
        assert!(err.to_string().contains("took too long"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(1));
        let _ = server.join();
    }

    #[test]
    fn bytes_right_after_the_proxy_reply_are_kept() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let proxy = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            // The reply and the first IRC line arrive together.
            stream
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\nPING :early\r\n")
                .unwrap();
            thread::sleep(Duration::from_millis(200));
        });
        let endpoint = Endpoint {
            kind: "http".into(),
            host: "127.0.0.1".into(),
            port,
            user: String::new(),
            pass: String::new(),
        };
        let stream = endpoint.connect("irc.example", 6667).unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        assert_eq!(line, "PING :early\r\n");
        proxy.join().unwrap();
    }

    #[test]
    fn only_a_200_status_opens_the_tunnel() {
        assert_eq!(
            http_status("HTTP/1.1 200 Connection established"),
            Some(200)
        );
        assert_eq!(http_status("HTTP/1.0 403 Forbidden 200"), Some(403));
        assert_eq!(http_status("garbage 200"), None);
        let mut long = std::io::Cursor::new(vec![b'a'; HTTP_HEAD_LIMIT + 10]);
        assert!(read_http_head(&mut long).is_err());
    }
}
