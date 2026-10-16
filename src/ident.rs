//! RFC 1413 ident server.
//!
//! `listen` binds the address and answers each query on its own thread.
//! The reply is `port, port : USERID : UNIX : name`.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// A query is two port numbers; anything longer is not one.
const QUERY_LIMIT: usize = 512;
/// The whole exchange must finish within this time, however slowly the
/// other side sends.
const DEADLINE: Duration = Duration::from_secs(10);
/// Queries answered at once; more connections are closed unanswered.
const WORKERS: usize = 8;

pub struct IdentServer {
    stop: Arc<AtomicBool>,
}

impl Drop for IdentServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn userid(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != ':')
        .take(32)
        .collect();
    if cleaned.is_empty() {
        "openirc".to_string()
    } else {
        cleaned
    }
}

pub fn parse_query(line: &str) -> Option<(u16, u16)> {
    let (left, right) = line.trim().split_once(',')?;
    Some((left.trim().parse().ok()?, right.trim().parse().ok()?))
}

/// The reply with the operating system name the user chose.
pub fn format_reply_as(server_port: u16, client_port: u16, system: &str, user: &str) -> String {
    let system: String = system
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .take(16)
        .collect();
    let system = if system.is_empty() {
        "UNIX".to_string()
    } else {
        system
    };
    format!(
        "{server_port}, {client_port} : USERID : {system} : {}\r\n",
        userid(user)
    )
}

pub fn listen_as(
    addr: impl ToSocketAddrs,
    user: String,
    system: String,
) -> std::io::Result<(IdentServer, SocketAddr)> {
    listen_within(addr, user, system, DEADLINE)
}

/// `listen_as` with its own time limit for each query.
fn listen_within(
    addr: impl ToSocketAddrs,
    user: String,
    system: String,
    deadline: Duration,
) -> std::io::Result<(IdentServer, SocketAddr)> {
    let listener = TcpListener::bind(addr)?;
    listener.set_nonblocking(true)?;
    let local = listener.local_addr()?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let user = userid(&user);
    thread::spawn(move || accept_loop(listener, flag, user, system, deadline));
    Ok((IdentServer { stop }, local))
}

fn accept_loop(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    user: String,
    system: String,
    deadline: Duration,
) {
    let busy = Arc::new(AtomicUsize::new(0));
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                if busy.fetch_add(1, Ordering::SeqCst) >= WORKERS {
                    busy.fetch_sub(1, Ordering::SeqCst);
                    drop(stream);
                    continue;
                }
                let user = user.clone();
                let system = system.clone();
                let busy = Arc::clone(&busy);
                let stop = Arc::clone(&stop);
                thread::spawn(move || {
                    let _ = answer(stream, &user, &system, &stop, deadline);
                    busy.fetch_sub(1, Ordering::SeqCst);
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
}

/// Read one query line of at most `QUERY_LIMIT` bytes before `deadline`.
/// Never more than that is read, line end included. Returns `None` when
/// the line is too long, too slow, or the server stopped.
fn read_query(
    stream: &mut TcpStream,
    stop: &AtomicBool,
    deadline: Duration,
) -> std::io::Result<Option<String>> {
    let started = Instant::now();
    let mut line = Vec::with_capacity(QUERY_LIMIT + 1);
    let mut chunk = [0u8; 128];
    loop {
        let left = deadline.saturating_sub(started.elapsed());
        if left.is_zero() || stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        stream.set_read_timeout(Some(left.min(Duration::from_secs(1))))?;
        // Room for the longest query and its line end, and no more.
        let room = (QUERY_LIMIT + 1 - line.len()).min(chunk.len());
        match stream.read(&mut chunk[..room]) {
            Ok(0) => break,
            Ok(count) => {
                line.extend_from_slice(&chunk[..count]);
                if let Some(end) = line.iter().position(|byte| *byte == b'\n') {
                    line.truncate(end);
                    break;
                }
                if line.len() > QUERY_LIMIT {
                    return Ok(None);
                }
            }
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(err) => return Err(err),
        }
    }
    Ok(Some(String::from_utf8_lossy(&line).into_owned()))
}

fn answer(
    mut stream: TcpStream,
    user: &str,
    system: &str,
    stop: &AtomicBool,
    deadline: Duration,
) -> std::io::Result<()> {
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let Some(line) = read_query(&mut stream, stop, deadline)? else {
        return Ok(());
    };
    if let Some((server_port, client_port)) = parse_query(&line) {
        stream.write_all(format_reply_as(server_port, client_port, system, user).as_bytes())?;
    } else {
        stream.write_all(b"0, 0 : ERROR : INVALID-PORT\r\n")?;
    }
    stream.flush()
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::net::TcpStream;

    use super::*;

    #[test]
    fn query_and_reply_follow_rfc1413() {
        assert_eq!(parse_query(" 1234 , 5678 \r\n"), Some((1234, 5678)));
        assert_eq!(
            format_reply_as(1234, 5678, "UNIX", "ada lovelace"),
            "1234, 5678 : USERID : UNIX : adalovelace\r\n"
        );
        assert!(parse_query("nope").is_none());
    }

    #[test]
    fn server_answers_a_query() {
        let (server, addr) = listen_as("127.0.0.1:0", "ada".into(), "UNIX".into()).unwrap();
        let mut client = TcpStream::connect(addr).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client.write_all(b"6667, 40000\r\n").unwrap();
        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "6667, 40000 : USERID : UNIX : ada\r\n");
        drop(server);
    }

    /// Read what the server sends until it hangs up: an end of stream, or a
    /// reset when it closed with our bytes unread. A wait longer than
    /// `within`, or any other error, fails the test.
    fn until_closed(client: &mut TcpStream, within: Duration) -> Vec<u8> {
        client.set_read_timeout(Some(within)).unwrap();
        let started = Instant::now();
        let mut reply = Vec::new();
        let mut chunk = [0u8; 256];
        loop {
            match client.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => reply.extend_from_slice(&chunk[..count]),
                Err(err) if err.kind() == std::io::ErrorKind::ConnectionReset => break,
                Err(err) => panic!("the server did not hang up in time: {err}"),
            }
        }
        assert!(started.elapsed() < within);
        reply
    }

    #[test]
    fn an_endless_query_is_cut_off() {
        let (server, addr) = listen_as("127.0.0.1:0", "ada".into(), "UNIX".into()).unwrap();
        // The longest query is still answered (it is not two ports).
        let mut client = TcpStream::connect(addr).unwrap();
        let mut longest = vec![b'1'; QUERY_LIMIT];
        longest.push(b'\n');
        client.write_all(&longest).unwrap();
        let reply = until_closed(&mut client, Duration::from_secs(3));
        assert_eq!(reply, b"0, 0 : ERROR : INVALID-PORT\r\n");
        // One byte more and the server hangs up unanswered, at once.
        let mut client = TcpStream::connect(addr).unwrap();
        let mut long = vec![b'1'; QUERY_LIMIT + 1];
        long.push(b'\n');
        let _ = client.write_all(&long);
        assert!(until_closed(&mut client, Duration::from_secs(3)).is_empty());
        drop(server);
    }

    #[test]
    fn a_slow_query_ends_at_the_deadline() {
        let deadline = Duration::from_millis(400);
        let (server, addr) =
            listen_within("127.0.0.1:0", "ada".into(), "UNIX".into(), deadline).unwrap();
        let mut client = TcpStream::connect(addr).unwrap();
        let mut trickle = client.try_clone().unwrap();
        // One byte now and then keeps every read short, but not the whole.
        let sender = thread::spawn(move || {
            for _ in 0..30 {
                if trickle.write_all(b"1").is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
        assert!(until_closed(&mut client, Duration::from_secs(2)).is_empty());
        let _ = sender.join();
        drop(server);
    }

    #[test]
    fn busy_workers_turn_away_more_and_stopping_ends_them() {
        let (server, addr) = listen_as("127.0.0.1:0", "ada".into(), "UNIX".into()).unwrap();
        let silent: Vec<TcpStream> = (0..WORKERS)
            .map(|_| TcpStream::connect(addr).unwrap())
            .collect();
        thread::sleep(Duration::from_millis(200));
        // Every worker waits on a silent client: one more is closed unread.
        let mut extra = TcpStream::connect(addr).unwrap();
        assert!(until_closed(&mut extra, Duration::from_secs(2)).is_empty());
        // Stopping the server ends the waiting workers too.
        drop(server);
        for mut client in silent {
            assert!(until_closed(&mut client, Duration::from_secs(3)).is_empty());
        }
    }
}
