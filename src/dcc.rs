//! Direct Client-to-Client chat.
//!
//! An offer is the CTCP payload `DCC CHAT chat <ipv4-integer> <port>`.
//! OpenIRC can place that offer and accept one.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DccOffer {
    pub nick: String,
    pub host: String,
    pub port: u16,
    /// The IRC session the offer came on (the window's own number for it),
    /// which the chat belongs to once it connects.
    pub server: Option<u64>,
}

pub fn parse_dcc_chat(argument: &str) -> Option<(String, u16)> {
    let mut parts = argument.split_whitespace();
    let kind = parts.next()?;
    if !kind.eq_ignore_ascii_case("CHAT") {
        return None;
    }
    let _protocol = parts.next();
    let host = parts.next()?;
    let port = parts.next()?.parse().ok()?;
    let host = if host.chars().all(|ch| ch.is_ascii_digit()) {
        decode_dcc_ip(host.parse().ok()?)
    } else {
        host.to_string()
    };
    Some((host, port))
}

pub fn encode_dcc_ip(ip: Ipv4Addr) -> u32 {
    u32::from(ip)
}

pub fn decode_dcc_ip(value: u32) -> String {
    Ipv4Addr::from(value).to_string()
}

pub struct DccChat {
    /// Lines to the peer. The queue holds `OUTGOING_LIMIT` lines; when the
    /// peer does not keep up, more are refused rather than kept.
    pub outgoing: SyncSender<String>,
    pub incoming: Receiver<String>,
    /// Hangs up when dropped, wherever the chat's thread is waiting.
    pub hangup: Hangup,
}

/// Lines waiting to be written to a DCC chat peer.
pub const OUTGOING_LIMIT: usize = 256;

/// The longest a write to a DCC chat peer may wait.
const WRITE_WAIT: Duration = Duration::from_secs(30);

/// A handle on a chat's socket that shuts it down when dropped. Closing the
/// socket from here ends a read or write the chat's thread is blocked in, so
/// letting go of a chat really ends it, even with a peer that stopped
/// reading.
pub struct Hangup(Option<TcpStream>);

impl Hangup {
    /// For tests: a chat without a socket.
    #[cfg(test)]
    pub fn none() -> Self {
        Self(None)
    }
}

impl Drop for Hangup {
    fn drop(&mut self) {
        if let Some(socket) = &self.0 {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    }
}

pub fn connect_chat(host: &str, port: u16) -> std::io::Result<DccChat> {
    let stream = crate::net::connect_dcc(host, port)?;
    stream.set_nodelay(true)?;
    split_chat(stream)
}

pub struct PendingListen {
    pub port: u16,
    pub chat: Receiver<DccChat>,
    /// Set when this is dropped: the listener stops waiting and frees its
    /// port at once, instead of when its wait runs out.
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for PendingListen {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// How long an offered DCC chat or file waits for the other side to connect.
pub const ACCEPT_WAIT: Duration = Duration::from_secs(120);

/// Accept one connection on `listener`, giving up after `wait`.
#[cfg(test)]
pub fn accept_within(listener: &TcpListener, wait: Duration) -> std::io::Result<TcpStream> {
    accept_until(listener, wait, || false)
}

/// Like `accept_within`, but gives up as soon as `cancelled` says so.
pub fn accept_until(
    listener: &TcpListener,
    wait: Duration,
    cancelled: impl Fn() -> bool,
) -> std::io::Result<TcpStream> {
    listener.set_nonblocking(true)?;
    let deadline = std::time::Instant::now() + wait;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false)?;
                return Ok(stream);
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if cancelled() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "cancelled",
                    ));
                }
                if std::time::Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "nobody connected in time",
                    ));
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(err) => return Err(err),
        }
    }
}

pub fn listen_chat() -> std::io::Result<PendingListen> {
    let listener = crate::net::listen_dcc()?;
    let port = listener.local_addr()?.port();
    let (ready_tx, ready_rx) = mpsc::channel();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop = cancel.clone();
    thread::spawn(move || {
        let cancelled = || stop.load(std::sync::atomic::Ordering::Relaxed);
        let Ok(stream) = accept_until(&listener, ACCEPT_WAIT, cancelled) else {
            return;
        };
        let _ = stream.set_nodelay(true);
        if let Ok(chat) = split_chat(stream) {
            let _ = ready_tx.send(chat);
        }
    });
    Ok(PendingListen {
        port,
        chat: ready_rx,
        cancel,
    })
}

fn split_chat(stream: TcpStream) -> std::io::Result<DccChat> {
    stream.set_read_timeout(Some(Duration::from_millis(200)))?;
    stream.set_write_timeout(Some(WRITE_WAIT))?;
    let hangup = Hangup(Some(stream.try_clone()?));
    let mut socket = stream;
    let (out_tx, out_rx) = mpsc::sync_channel::<String>(OUTGOING_LIMIT);
    // A bounded queue: when the window falls behind, the reader waits and
    // the peer is slowed down by TCP instead of filling memory.
    let (in_tx, in_rx) = mpsc::sync_channel::<String>(1_000);
    thread::spawn(move || {
        let mut pending = crate::message::LineBuffer::default();
        let mut buf = [0u8; 4096];
        loop {
            match socket.read(&mut buf) {
                Ok(0) => break,
                Ok(read) => {
                    pending.extend(&buf[..read]);
                    while let Some(raw) = pending.next_line() {
                        let line = crate::message::decode_line(raw);
                        if in_tx.send(line).is_err() {
                            return;
                        }
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::TimedOut => {}
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
            loop {
                match out_rx.try_recv() {
                    Ok(text) => {
                        let text = text.replace(['\r', '\n'], " ");
                        if write!(socket, "{text}\r\n").is_err() || socket.flush().is_err() {
                            return;
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    // The chat window was closed: hang up, even if the
                    // other side is still connected and quiet.
                    Err(mpsc::TryRecvError::Disconnected) => {
                        let _ = socket.shutdown(std::net::Shutdown::Both);
                        return;
                    }
                }
            }
        }
    });
    Ok(DccChat {
        outgoing: out_tx,
        incoming: in_rx,
        hangup,
    })
}

pub fn local_ipv4() -> Ipv4Addr {
    if let Ok(socket) = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        && socket.connect("8.8.8.8:80").is_ok()
        && let Ok(address) = socket.local_addr()
        && let std::net::IpAddr::V4(ip) = address.ip()
    {
        return ip;
    }
    Ipv4Addr::LOCALHOST
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_integer_address_form() {
        let (host, port) = parse_dcc_chat("CHAT chat 2130706433 5500").unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 5500);
        assert_eq!(encode_dcc_ip(Ipv4Addr::LOCALHOST), 2130706433);
    }

    #[test]
    fn closing_a_chat_hangs_up_on_a_quiet_peer() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut seen = Vec::new();
            // Never writes; reads until our side hangs up.
            let _ = std::io::Read::read_to_end(&mut sock, &mut seen);
            seen
        });
        let chat = connect_chat("127.0.0.1", port).unwrap();
        chat.outgoing.send("bye".into()).unwrap();
        thread::sleep(Duration::from_millis(300));
        let started = std::time::Instant::now();
        drop(chat);
        let seen = peer.join().unwrap();
        assert_eq!(seen, b"bye\r\n");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "hung up at once"
        );
    }

    #[test]
    fn hanging_up_ends_a_chat_stuck_writing_to_a_peer_that_never_reads() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let peer = thread::spawn(move || {
            // Connected, and never reads, until the test is over (or fails,
            // which drops `done_tx`): the test does not race this side.
            let socket = accept_within(&listener, Duration::from_secs(5)).unwrap();
            let _ = done_rx.recv();
            drop(socket);
        });
        let DccChat {
            outgoing,
            incoming,
            hangup,
        } = connect_chat("127.0.0.1", port).unwrap();
        // Write until the socket buffers are full, the thread is stuck in a
        // write, and the queue behind it fills up too.
        let line = "x".repeat(64 * 1024);
        let started = std::time::Instant::now();
        let mut full = false;
        // Only against a hang: how many lines fit depends on the machine's
        // socket buffers, and the peer waits as long as it takes.
        while started.elapsed() < Duration::from_secs(60) {
            match outgoing.try_send(line.clone()) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(_)) => {
                    full = true;
                    break;
                }
                Err(err) => panic!("{err}"),
            }
        }
        assert!(full, "the queue to a peer that never reads is bounded");
        thread::sleep(Duration::from_millis(300));
        let hung_up = std::time::Instant::now();
        drop(hangup);
        drop(outgoing);
        // The chat's thread ends at once, not when its write gives up
        // (`WRITE_WAIT`).
        assert!(matches!(
            incoming.recv_timeout(Duration::from_secs(10)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
        assert!(hung_up.elapsed() < Duration::from_secs(10));
        let _ = done_tx.send(());
        peer.join().unwrap();
    }

    #[test]
    fn dropping_an_unanswered_chat_offer_frees_its_port() {
        let pending = listen_chat().unwrap();
        let port = pending.port;
        drop(pending);
        let started = std::time::Instant::now();
        while TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).is_err() {
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "the listener let go of {port} when cancelled, not when its wait ran out"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}
