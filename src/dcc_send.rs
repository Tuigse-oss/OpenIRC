//! DCC SEND file transfer offers.
//!
//! Handles the CTCP payload `DCC SEND filename ip port [size]` where `ip`
//! is either a dotted IPv4 address or a 32-bit integer in network byte
//! order (e.g. 2130706433 is 127.0.0.1), mirroring DCC chat.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// Shared between a transfer thread and the window: bytes moved so far
/// (counting a resume's starting offset) and a request to stop.
#[derive(Debug, Default)]
pub struct Meter {
    bytes: AtomicU64,
    cancel: AtomicBool,
    /// Where a send starts, and whether the receiver has connected: a
    /// resume is agreed to only while it has not.
    start: std::sync::Mutex<Start>,
}

#[derive(Debug, Default)]
struct Start {
    position: u64,
    connected: bool,
}

impl Meter {
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Send from `position` instead of the start (an accepted resume).
    /// False, changing nothing, once the receiver has connected: the send
    /// has begun where it was.
    pub fn start_at(&self, position: u64) -> bool {
        let mut start = self
            .start
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if start.connected {
            return false;
        }
        start.position = position;
        true
    }

    /// The receiver connected: from now on the start stays where it is.
    fn connected(&self) -> u64 {
        let mut start = self
            .start
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        start.connected = true;
        start.position
    }

    fn set(&self, bytes: u64) -> io::Result<()> {
        self.bytes.store(bytes, Ordering::Relaxed);
        if self.cancel.load(Ordering::Relaxed) {
            Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
        } else {
            Ok(())
        }
    }
}

/// A parsed DCC SEND offer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DccSendOffer {
    pub filename: String,
    pub host: String,
    pub port: u16,
    pub size: Option<u64>,
    pub token: Option<String>,
}

/// Parse a CTCP argument shaped like `SEND filename ip port [size]`.
/// Accepts an optional leading `DCC` token. The ip may be an integer
/// or dotted IPv4.
pub fn parse_dcc_send(argument: &str) -> Option<DccSendOffer> {
    let rest = argument.trim();
    let rest = strip_optional_prefix_word(rest, "DCC")?;
    let rest = strip_prefix_word(rest, "SEND")?;
    let (filename, rest) = next_token(rest)?;
    if filename.is_empty() {
        return None;
    }
    let (ip, rest) = next_token(rest)?;
    let (port, rest) = next_token(rest)?;
    let port: u16 = port.parse().ok()?;
    let (size, token) = match next_token(rest) {
        Some((size_token, rest)) => {
            let size = size_token.parse::<u64>().ok()?;
            let token = next_token(rest).map(|(token, _)| token);
            (Some(size), token)
        }
        None => (None, None),
    };
    if ip.is_empty() {
        return None;
    }
    let host = if ip.chars().all(|ch| ch.is_ascii_digit()) {
        let value: u32 = ip.parse().ok()?;
        Ipv4Addr::from(value).to_string()
    } else {
        ip
    };
    Some(DccSendOffer {
        filename,
        host,
        port,
        size,
        token,
    })
}

pub fn is_passive(offer: &DccSendOffer) -> bool {
    offer.port == 0
}

/// A token for a passive offer that others cannot guess from earlier ones:
/// the standard library's randomly keyed hasher over a counter and the clock.
pub fn fresh_token() -> String {
    use std::hash::BuildHasher;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    token_from(
        std::collections::hash_map::RandomState::new().build_hasher(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        nanos,
    )
}

/// The token `hasher` makes of a counter and a time. Tokens are digits by
/// custom: nine of them, never starting with 0.
fn token_from(mut hasher: impl std::hash::Hasher, counter: u64, nanos: u128) -> String {
    hasher.write_u64(counter);
    hasher.write_u128(nanos);
    token_digits(hasher.finish())
}

fn token_digits(hash: u64) -> String {
    (100_000_000 + hash % 900_000_000).to_string()
}

/// `PRIVMSG nick :\x01DCC SEND file 0 0 size token\x01`
/// The sender does not listen. The other client listens and replies with the same token.
pub fn passive_offer_message(nick: &str, filename: &str, size: u64, token: &str) -> String {
    let filename = wire_filename(filename);
    format!("PRIVMSG {nick} :\u{1}DCC SEND {filename} 0 0 {size} {token}\u{1}")
}

/// Reply that tells the passive sender where to connect.
pub fn passive_accept_message(
    nick: &str,
    filename: &str,
    ip_int: u32,
    port: u16,
    size: u64,
    token: &str,
) -> String {
    let filename = wire_filename(filename);
    format!("PRIVMSG {nick} :\u{1}DCC SEND {filename} {ip_int} {port} {size} {token}\u{1}")
}

/// Encode the CTCP argument `RESUME filename port offset` without the
/// `\x01` wrapper.
pub fn encode_resume_request(filename: &str, port: u16, offset: u64) -> String {
    format!("RESUME {} {port} {offset}", wire_filename(filename))
}

/// The answer to a receiver's RESUME: `ACCEPT filename port position`,
/// without the `\x01` wrapper.
pub fn encode_accept(filename: &str, port: u16, position: u64) -> String {
    format!("ACCEPT {} {port} {position}", wire_filename(filename))
}

/// Parse `RESUME filename port position`, with an optional leading `DCC`.
pub fn parse_dcc_resume(argument: &str) -> Option<(String, u16, u64)> {
    let rest = argument.trim();
    let rest = strip_optional_prefix_word(rest, "DCC")?;
    let rest = strip_prefix_word(rest, "RESUME")?;
    let (filename, rest) = next_token(rest)?;
    let (port, rest) = next_token(rest)?;
    let (position, _) = next_token(rest)?;
    Some((filename, port.parse().ok()?, position.parse().ok()?))
}

/// Parse `ACCEPT filename port position`, with an optional leading `DCC`.
pub fn parse_dcc_accept(argument: &str) -> Option<(String, u16, u64)> {
    let rest = argument.trim();
    let rest = strip_optional_prefix_word(rest, "DCC")?;
    let rest = strip_prefix_word(rest, "ACCEPT")?;
    let (filename, rest) = next_token(rest)?;
    let (port, rest) = next_token(rest)?;
    let (position, _) = next_token(rest)?;
    Some((filename, port.parse().ok()?, position.parse().ok()?))
}

/// How long a transfer may go without any progress before it fails.
const STALL: Duration = Duration::from_secs(60);
/// How often a blocked read or write looks at the Cancel button.
const POLL: Duration = Duration::from_millis(500);
/// How often the end of a send looks at what the receiver confirmed.
const CONFIRM_POLL: Duration = Duration::from_millis(50);

fn waiting(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// Write all of `buf`, giving up when cancelled or when the other side
/// takes nothing for `STALL`.
fn write_all_watched(
    stream: &mut std::net::TcpStream,
    buf: &[u8],
    meter: &Meter,
) -> io::Result<()> {
    let mut sent = 0;
    let mut progress = std::time::Instant::now();
    while sent < buf.len() {
        match stream.write(&buf[sent..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "the connection closed",
                ));
            }
            Ok(count) => {
                sent += count;
                progress = std::time::Instant::now();
            }
            Err(err) if waiting(&err) || err.kind() == io::ErrorKind::Interrupted => {
                if meter.cancelled() {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
                }
                if progress.elapsed() >= STALL {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the other side stopped taking data",
                    ));
                }
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

/// Read some bytes, giving up when cancelled or when nothing comes for
/// `STALL`. Returns 0 at the end of the stream.
fn read_watched(
    stream: &mut std::net::TcpStream,
    buf: &mut [u8],
    meter: &Meter,
) -> io::Result<usize> {
    let mut progress = std::time::Instant::now();
    loop {
        match stream.read(buf) {
            Ok(count) => return Ok(count),
            Err(err) if waiting(&err) || err.kind() == io::ErrorKind::Interrupted => {
                if meter.cancelled() {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
                }
                if progress.elapsed() >= STALL {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the sender stopped sending",
                    ));
                }
                if !waiting(&err) {
                    progress = std::time::Instant::now();
                }
            }
            Err(err) => return Err(err),
        }
    }
}

/// Listen for the receiver and write one file. A resume accepted before
/// the receiver connected (see `Meter::start_at`) starts there.
pub fn serve_file(listener: std::net::TcpListener, path: &Path, meter: &Meter) -> io::Result<u64> {
    let stream =
        crate::dcc::accept_until(&listener, crate::dcc::ACCEPT_WAIT, || meter.cancelled())?;
    let start = meter.connected();
    send_over(stream, path, start, meter)
}

/// Connect to the side that is listening and write the file. Used after a passive reply.
pub fn upload_to(host: &str, port: u16, path: &Path, meter: &Meter) -> io::Result<u64> {
    let stream = crate::net::connect_dcc(host, port)?;
    meter.connected();
    send_over(stream, path, 0, meter)
}

/// What the receiver has confirmed so far, kept up to date by the thread
/// that reads its acknowledgements.
#[derive(Debug, Default)]
struct Confirmed {
    /// The furthest position confirmed, with the 32-bit counts unwrapped.
    position: AtomicU64,
    /// Whether any acknowledgement came at all.
    any: AtomicBool,
    /// The receiver closed the connection, or reading from it failed.
    closed: AtomicBool,
}

/// The full position a 32-bit acknowledgement stands for: the first one
/// at or after `last` that ends the same way. It never goes back, and never
/// past `written`, which no receiver can have seen yet; `None` for a count
/// that fits neither, such as an old acknowledgement arriving late.
fn unwrap_ack(ack: u32, last: u64, written: u64) -> Option<u64> {
    let mut full = (last & !0xFFFF_FFFF) | u64::from(ack);
    if full < last {
        full += 1 << 32;
    }
    (full <= written).then_some(full)
}

/// Write the file from `start` without waiting on each acknowledgement. A
/// helper thread reads the receiver's 4-byte acks as they come, so neither
/// side can stall on a full socket buffer, and keeps only the furthest
/// position confirmed (they are 32-bit and wrap for large files).
///
/// At the end the send has worked once the receiver confirms the last byte.
/// A receiver that never sends a single ack is taken at its word when it
/// closes the connection after the last byte, as some clients do. Anything
/// else fails: a receiver that hangs up short of the end, or one whose
/// confirmations stop moving for `STALL`. Cancel ends the wait at once.
fn send_over(
    mut stream: std::net::TcpStream,
    path: &Path,
    start: u64,
    meter: &Meter,
) -> io::Result<u64> {
    stream.set_nodelay(true)?;
    stream.set_write_timeout(Some(POLL))?;
    let mut file = File::open(path)?;
    if start > 0 {
        file.seek(SeekFrom::Start(start))?;
    }
    let confirmed = std::sync::Arc::new(Confirmed::default());
    confirmed.position.store(start, Ordering::Relaxed);
    // How far the bytes handed to the socket reach, for checking acks.
    let written = std::sync::Arc::new(AtomicU64::new(start));
    let done = std::sync::Arc::new(AtomicBool::new(false));
    let mut acks = stream.try_clone()?;
    acks.set_read_timeout(Some(POLL))?;
    let reader = {
        let (confirmed, written, done) = (confirmed.clone(), written.clone(), done.clone());
        std::thread::spawn(move || read_acks(&mut acks, &confirmed, &written, &done))
    };
    let result = write_and_confirm(&mut stream, &mut file, start, meter, &confirmed, &written);
    done.store(true, Ordering::Relaxed);
    let _ = stream.shutdown(std::net::Shutdown::Both);
    let _ = reader.join();
    result
}

/// Read 4-byte acknowledgements into `confirmed` until the connection
/// closes or the send is over.
fn read_acks(
    acks: &mut std::net::TcpStream,
    confirmed: &Confirmed,
    written: &AtomicU64,
    done: &AtomicBool,
) {
    let mut ack = [0u8; 4];
    let mut have = 0;
    while !done.load(Ordering::Relaxed) {
        match acks.read(&mut ack[have..]) {
            Ok(0) => break,
            Ok(count) => {
                have += count;
                if have < 4 {
                    continue;
                }
                have = 0;
                confirmed.any.store(true, Ordering::Relaxed);
                let last = confirmed.position.load(Ordering::Relaxed);
                let reached = written.load(Ordering::Relaxed);
                if let Some(full) = unwrap_ack(u32::from_be_bytes(ack), last, reached) {
                    confirmed.position.store(full, Ordering::Relaxed);
                }
            }
            Err(err) if waiting(&err) || err.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    confirmed.closed.store(true, Ordering::Relaxed);
}

fn write_and_confirm(
    stream: &mut std::net::TcpStream,
    file: &mut File,
    start: u64,
    meter: &Meter,
    confirmed: &Confirmed,
    written: &AtomicU64,
) -> io::Result<u64> {
    let mut buf = [0u8; 16 * 1024];
    let mut total = start;
    meter.set(total)?;
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        // Counted before writing: the receiver may read the bytes before
        // the write call returns.
        written.store(total + read as u64, Ordering::Relaxed);
        write_all_watched(stream, &buf[..read], meter)?;
        total += read as u64;
        meter.set(total)?;
    }
    let mut seen = confirmed.position.load(Ordering::Relaxed);
    let mut progress = std::time::Instant::now();
    loop {
        let position = confirmed.position.load(Ordering::Relaxed);
        if position == total {
            return Ok(total - start);
        }
        if confirmed.closed.load(Ordering::Relaxed) {
            // Read the position again: the last ack may have come just
            // before the close.
            let position = confirmed.position.load(Ordering::Relaxed);
            if position == total || !confirmed.any.load(Ordering::Relaxed) {
                return Ok(total - start);
            }
            return Err(unconfirmed(position, total));
        }
        if meter.cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        if position != seen {
            seen = position;
            progress = std::time::Instant::now();
        } else if progress.elapsed() >= STALL {
            return Err(unconfirmed(position, total));
        }
        std::thread::sleep(CONFIRM_POLL);
    }
}

fn unconfirmed(position: u64, total: u64) -> io::Error {
    io::Error::new(
        io::ErrorKind::UnexpectedEof,
        format!("the receiver did not confirm every byte ({position} of {total})"),
    )
}

/// A new file for a download: `path` if nobody has it yet, else the first
/// free `name (1)`, `name (2)`, … beside it. Creating it at once keeps the
/// name for this transfer, so two downloads can never share one file.
pub fn create_new_file(path: &Path) -> io::Result<(File, std::path::PathBuf)> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download.bin".into());
    let mut candidate = path.to_path_buf();
    for _ in 0..1_000 {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((file, candidate)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                candidate = unique_path(dir, &name);
            }
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free file name",
    ))
}

/// Replace an existing file: only after the user chose Overwrite.
pub fn create_overwrite(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    File::create(path)
}

/// Open a partial download to continue at `offset`. The file must still be
/// exactly that long; anything else means it changed since the resume was
/// asked for, and writing would corrupt it.
pub fn open_resume(path: &Path, offset: u64) -> io::Result<File> {
    let mut file = OpenOptions::new().write(true).open(path)?;
    let have = file.metadata()?.len();
    if have != offset {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("the file is {have} bytes now, not {offset}; it was not changed"),
        ));
    }
    file.seek(SeekFrom::Start(offset))?;
    Ok(file)
}

/// Accept one connection on `listener` and store the file the sender writes.
pub fn receive_listener(
    listener: std::net::TcpListener,
    file: File,
    size: Option<u64>,
    meter: &Meter,
) -> io::Result<u64> {
    let stream =
        crate::dcc::accept_until(&listener, crate::dcc::ACCEPT_WAIT, || meter.cancelled())?;
    receive_into(stream, file, 0, size, meter)
}

/// Connect to a sender and store the file, which is open at `offset` (0
/// for a new file). Returns the file's length at the end.
pub fn receive_from(
    host: &str,
    port: u16,
    file: File,
    offset: u64,
    size: Option<u64>,
    meter: &Meter,
) -> io::Result<u64> {
    let stream = crate::net::connect_dcc(host, port)?;
    receive_into(stream, file, offset, size, meter)
}

/// Store what the sender writes, sending a 4-byte ack after each chunk.
/// With a known size, exactly that many bytes must arrive: fewer is an
/// error (the partial file stays for a later resume) and more are not
/// written.
fn receive_into(
    mut stream: std::net::TcpStream,
    mut file: File,
    offset: u64,
    size: Option<u64>,
    meter: &Meter,
) -> io::Result<u64> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(POLL))?;
    stream.set_write_timeout(Some(POLL))?;
    let mut buf = [0u8; 16 * 1024];
    let mut total = offset;
    meter.set(total)?;
    loop {
        let room = match size {
            Some(size) if total >= size => break,
            // Compared as u64 first: on a 32-bit system 4 GiB left would
            // wrap to 0 as a usize.
            Some(size) => (size - total).min(buf.len() as u64) as usize,
            None => buf.len(),
        };
        let read = read_watched(&mut stream, &mut buf[..room], meter)?;
        if read == 0 {
            if let Some(size) = size {
                file.flush()?;
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("the sender stopped at {total} of {size} bytes"),
                ));
            }
            break;
        }
        file.write_all(&buf[..read])?;
        total += read as u64;
        write_all_watched(&mut stream, &((total as u32).to_be_bytes()), meter)?;
        meter.set(total)?;
    }
    file.flush()?;
    Ok(total)
}

pub fn safe_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .filter(|ch| !ch.is_control() && !matches!(ch, ':' | '<' | '>' | '"' | '|' | '?' | '*'))
        .collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return "download.bin".to_string();
    }
    let stem = cleaned.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        format!("_{cleaned}")
    } else {
        cleaned
    }
}

/// A path in `dir` for `name` that does not exist yet: `name`, then
/// `name (1)`, `name (2)`, and so on, keeping the extension.
pub fn unique_path(dir: &Path, name: &str) -> std::path::PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|path| !path.exists())
        .unwrap_or(first)
}

/// Quote a filename for a DCC SEND line when it has spaces.
fn wire_filename(filename: &str) -> String {
    if filename.contains(char::is_whitespace) {
        format!("\"{}\"", filename.replace('"', ""))
    } else {
        filename.to_string()
    }
}

/// Build one IRC PRIVMSG line offering a file, including the
/// `\x01DCC SEND ...\x01` payload.
pub fn offer_message(nick: &str, filename: &str, ip_int: u32, port: u16, size: u64) -> String {
    let filename = wire_filename(filename);
    format!("\x01DCC SEND {filename} {ip_int} {port} {size}\x01",)
        .pipe(|payload| format!("PRIVMSG {nick} :{payload}"))
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}

impl Pipe for String {}

/// Strip a required leading whitespace-separated word (case-insensitive),
/// returning the remainder.
fn strip_prefix_word<'a>(input: &'a str, word: &str) -> Option<&'a str> {
    let trimmed = input.trim_start();
    let (first, rest) = split_first_word(trimmed);
    if first.eq_ignore_ascii_case(word) {
        Some(rest.trim_start())
    } else {
        None
    }
}

/// Strip an optional leading word; returns the remainder (or the input
/// unchanged when the word is absent). Returns `None` only when the input
/// is empty.
fn strip_optional_prefix_word<'a>(input: &'a str, word: &str) -> Option<&'a str> {
    let trimmed = input.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    match strip_prefix_word(trimmed, word) {
        Some(rest) => Some(rest),
        None => Some(trimmed),
    }
}

fn split_first_word(input: &str) -> (&str, &str) {
    match input.find(char::is_whitespace) {
        Some(idx) => (&input[..idx], &input[idx..]),
        None => (input, ""),
    }
}

/// Take the next token, supporting a double-quoted token that may contain
/// whitespace. Returns the token and the remainder.
fn next_token(input: &str) -> Option<(String, &str)> {
    let trimmed = input.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(after_quote) = trimmed.strip_prefix('"') {
        let end = after_quote.find('"')?;
        let token = after_quote[..end].to_string();
        let rest = &after_quote[end + 1..];
        return Some((token, rest.trim_start()));
    }
    let (word, rest) = split_first_word(trimmed);
    Some((word.to_string(), rest.trim_start()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_integer_address() {
        let offer = parse_dcc_send("SEND file.txt 2130706433 5000 1234").unwrap();
        assert_eq!(
            offer,
            DccSendOffer {
                filename: "file.txt".to_string(),
                host: "127.0.0.1".to_string(),
                port: 5000,
                size: Some(1234),
                token: None,
            }
        );
    }

    #[test]
    fn parses_dotted_address() {
        let offer = parse_dcc_send("SEND file.txt 192.168.1.2 5000 99").unwrap();
        assert_eq!(offer.filename, "file.txt");
        assert_eq!(offer.host, "192.168.1.2");
        assert_eq!(offer.port, 5000);
        assert_eq!(offer.size, Some(99));
    }

    #[test]
    fn parses_missing_size() {
        let offer = parse_dcc_send("SEND file.txt 2130706433 5000").unwrap();
        assert_eq!(offer.filename, "file.txt");
        assert_eq!(offer.host, "127.0.0.1");
        assert_eq!(offer.port, 5000);
        assert_eq!(offer.size, None);
        assert_eq!(offer.token, None);
    }

    #[test]
    fn parses_a_passive_offer() {
        let offer = parse_dcc_send("SEND file.txt 0 0 100 42").unwrap();
        assert!(is_passive(&offer));
        assert_eq!(offer.host, "0.0.0.0");
        assert_eq!(offer.size, Some(100));
        assert_eq!(offer.token.as_deref(), Some("42"));
        assert_eq!(
            passive_offer_message("bob", "file.txt", 100, "42"),
            "PRIVMSG bob :\u{1}DCC SEND file.txt 0 0 100 42\u{1}"
        );
        assert_eq!(
            passive_accept_message("bob", "file.txt", 2130706433, 5000, 100, "42"),
            "PRIVMSG bob :\u{1}DCC SEND file.txt 2130706433 5000 100 42\u{1}"
        );
    }

    #[test]
    fn uploads_to_the_listening_side() {
        let dir = std::env::temp_dir().join(format!("openirc-passive-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("file.txt");
        std::fs::write(&source, b"passive").unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let dest = dir.join("out.txt");
        let saved = dest.clone();
        let receiver = std::thread::spawn(move || {
            receive_listener(
                listener,
                create_new_file(&saved).unwrap().0,
                Some(7),
                &Meter::default(),
            )
            .unwrap()
        });
        let sent = upload_to("127.0.0.1", port, &source, &Meter::default()).unwrap();
        assert_eq!(sent, 7);
        assert_eq!(receiver.join().unwrap(), 7);
        assert_eq!(std::fs::read(dest).unwrap(), b"passive");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_cancelled_receive_stops_with_an_error() {
        let dir = std::env::temp_dir().join(format!("openirc-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("data.bin");
        std::fs::write(&source, vec![7u8; 300_000]).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let send_path = source.clone();
        let sender =
            std::thread::spawn(move || serve_file(listener, &send_path, &Meter::default()));
        let meter = Meter::default();
        meter.cancel();
        let err = receive_from(
            "127.0.0.1",
            port,
            create_overwrite(&dir.join("out.bin")).unwrap(),
            0,
            Some(300_000),
            &meter,
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
        let _ = sender.join();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn encodes_resume_argument() {
        assert_eq!(
            encode_resume_request("file.txt", 5000, 123),
            "RESUME file.txt 5000 123"
        );
        let resume = encode_resume_request("my file.txt", 5000, 9);
        assert_eq!(resume, "RESUME \"my file.txt\" 5000 9");
        assert_eq!(
            parse_dcc_resume(&resume),
            Some(("my file.txt".to_string(), 5000, 9))
        );
        assert_eq!(
            parse_dcc_accept(&encode_accept("my file.txt", 5000, 9)),
            Some(("my file.txt".to_string(), 5000, 9))
        );
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("openirc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A one-shot sender on localhost that writes `data` and closes.
    fn raw_sender(data: Vec<u8>) -> (u16, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.write_all(&data);
            let _ = stream.shutdown(std::net::Shutdown::Write);
            let mut sink = Vec::new();
            let _ = stream.read_to_end(&mut sink);
        });
        (port, handle)
    }

    #[test]
    fn a_short_or_long_transfer_is_not_reported_as_complete() {
        let dir = scratch("sizes");
        let (port, sender) = raw_sender(b"abc".to_vec());
        let file = create_overwrite(&dir.join("short.bin")).unwrap();
        let err =
            receive_from("127.0.0.1", port, file, 0, Some(100), &Meter::default()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(std::fs::read(dir.join("short.bin")).unwrap(), b"abc");
        sender.join().unwrap();

        let (port, sender) = raw_sender(b"abcdefgh".to_vec());
        let file = create_overwrite(&dir.join("long.bin")).unwrap();
        let got = receive_from("127.0.0.1", port, file, 0, Some(3), &Meter::default()).unwrap();
        assert_eq!(got, 3);
        assert_eq!(std::fs::read(dir.join("long.bin")).unwrap(), b"abc");
        sender.join().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn two_new_downloads_of_one_name_get_their_own_files() {
        let dir = scratch("reserve");
        let (_first, one) = create_new_file(&dir.join("report.bin")).unwrap();
        let (_second, two) = create_new_file(&dir.join("report.bin")).unwrap();
        assert_eq!(one, dir.join("report.bin"));
        assert_eq!(two, dir.join("report (1).bin"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resume_only_continues_a_file_of_the_expected_length() {
        let dir = scratch("resume");
        let path = dir.join("part.bin");
        std::fs::write(&path, b"hello ").unwrap();
        assert_eq!(
            open_resume(&path, 0).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"hello ", "left alone");
        // The sender resumes at 6 and sends the rest.
        let source = dir.join("whole.bin");
        std::fs::write(&source, b"hello world").unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let meter = std::sync::Arc::new(Meter::default());
        meter.start_at(6);
        let shared = meter.clone();
        let sender = std::thread::spawn(move || serve_file(listener, &source, &shared));
        let file = open_resume(&path, 6).unwrap();
        let got = receive_from("127.0.0.1", port, file, 6, Some(11), &Meter::default()).unwrap();
        assert_eq!(got, 11);
        assert_eq!(std::fs::read(&path).unwrap(), b"hello world");
        assert_eq!(sender.join().unwrap().unwrap(), 5);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cancel_ends_a_send_to_a_receiver_that_never_reads() {
        let dir = scratch("stall");
        let source = dir.join("big.bin");
        std::fs::write(&source, vec![1u8; 64 * 1024 * 1024]).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let meter = std::sync::Arc::new(Meter::default());
        let shared = meter.clone();
        let sender = std::thread::spawn(move || serve_file(listener, &source, &shared));
        // Connect and never read.
        let _peer = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let started = std::time::Instant::now();
        while meter.bytes() == 0 && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(300));
        meter.cancel();
        let cancelled = std::time::Instant::now();
        let err = sender.join().unwrap().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
        assert!(cancelled.elapsed() < Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_empty_file_is_sent_and_saved_on_both_sides() {
        let dir = scratch("empty");
        let source = dir.join("empty.bin");
        std::fs::write(&source, b"").unwrap();
        // The receiver listens; the sender connects (a passive offer).
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let file = create_overwrite(&dir.join("got1.bin")).unwrap();
        let receiver = std::thread::spawn(move || {
            receive_listener(listener, file, Some(0), &Meter::default())
        });
        assert_eq!(
            upload_to("127.0.0.1", port, &source, &Meter::default()).unwrap(),
            0
        );
        assert_eq!(receiver.join().unwrap().unwrap(), 0);
        // The sender listens; the receiver connects.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let path = source.clone();
        let sender = std::thread::spawn(move || serve_file(listener, &path, &Meter::default()));
        let file = create_overwrite(&dir.join("got2.bin")).unwrap();
        let got = receive_from("127.0.0.1", port, file, 0, Some(0), &Meter::default()).unwrap();
        assert_eq!(got, 0);
        assert_eq!(sender.join().unwrap().unwrap(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cancel_ends_the_wait_for_the_last_acknowledgement() {
        let dir = scratch("lastack");
        let source = dir.join("small.bin");
        std::fs::write(&source, vec![3u8; 1024]).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let meter = std::sync::Arc::new(Meter::default());
        let shared = meter.clone();
        let sender = std::thread::spawn(move || serve_file(listener, &source, &shared));
        // A receiver that takes every byte but keeps confirming only one.
        let mut peer = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        peer.set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let mut got = 0;
        let mut buf = [0u8; 4096];
        let started = std::time::Instant::now();
        while got < 1024 && started.elapsed() < Duration::from_secs(5) {
            if let Ok(count) = peer.read(&mut buf) {
                got += count;
            }
        }
        assert_eq!(got, 1024);
        let pester = std::thread::spawn(move || {
            for _ in 0..60 {
                if peer.write_all(&1u32.to_be_bytes()).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        std::thread::sleep(Duration::from_millis(400));
        assert!(
            !sender.is_finished(),
            "still waiting for the last byte's ack"
        );
        meter.cancel();
        let cancelled = std::time::Instant::now();
        let err = sender.join().unwrap().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
        assert!(cancelled.elapsed() < Duration::from_secs(2));
        let _ = pester.join();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Feed acknowledgements in order, as the reading thread does, and give
    /// the confirmed position.
    fn confirm(start: u64, acks: &[(u32, u64)]) -> u64 {
        acks.iter().fold(start, |last, (ack, written)| {
            unwrap_ack(*ack, last, *written).unwrap_or(last)
        })
    }

    #[test]
    fn acknowledgements_unwrap_past_four_gibibytes_and_never_go_back() {
        const WRAP: u64 = 1 << 32;
        let total = WRAP + 65_536;
        // Confirmed up to 4 GiB (the count wraps to 0 there); the last
        // 64 KiB sit unread. The early ack of 65,536 was read long ago and
        // stood for 65,536, not for the end.
        let partial = confirm(0, &[(65_536, 65_536), (0, WRAP), (0, total)]);
        assert_eq!(partial, WRAP);
        assert_ne!(partial, total);
        // The real last ack, after the wrap, confirms everything.
        assert_eq!(confirm(partial, &[(65_536, total)]), total);
        // A count beyond what was written, or behind what was confirmed,
        // changes nothing.
        assert_eq!(unwrap_ack(5_000, 0, 4_096), None);
        assert_eq!(unwrap_ack(1, 1_024, 2_048), None);
        assert_eq!(unwrap_ack(1_024, 1_024, 2_048), Some(1_024));
        // A resume that starts past 4 GiB reads its acks from there.
        assert_eq!(unwrap_ack(10, WRAP + 5, WRAP + 100), Some(WRAP + 10));
    }

    #[test]
    fn a_resume_is_agreed_to_only_before_the_receiver_connects() {
        let meter = Meter::default();
        assert!(meter.start_at(10));
        assert_eq!(meter.connected(), 10);
        assert!(!meter.start_at(20), "the send has begun at 10");
        assert_eq!(meter.connected(), 10);
    }

    #[test]
    fn cancel_ends_the_wait_for_a_receiver() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let meter = Meter::default();
        meter.cancel();
        let started = std::time::Instant::now();
        let err = serve_file(listener, Path::new("/nonexistent"), &meter).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn passive_tokens_do_not_follow_each_other() {
        // Always nine digits, never a leading 0, whatever the hash.
        for hash in [0, 1, 899_999_999, 900_000_000, u64::MAX] {
            let token = token_digits(hash);
            assert_eq!(token.len(), 9, "{hash}");
            assert!(!token.starts_with('0'));
            assert!(token.bytes().all(|byte| byte.is_ascii_digit()));
        }
        assert_eq!(token_digits(0), "100000000");
        assert_eq!(token_digits(899_999_999), "999999999");
        // With a fixed key, so the result is the same every run: the next
        // counter, at the same moment, gives a token far from the last.
        let fixed = |counter| token_from(std::hash::DefaultHasher::new(), counter, 1_700_000_000);
        let tokens: Vec<u64> = (1..=50)
            .map(|counter| fixed(counter).parse().unwrap())
            .collect();
        for pair in tokens.windows(2) {
            assert!(pair[0].abs_diff(pair[1]) > 1_000, "{pair:?}");
        }
        assert_eq!(fixed(7), fixed(7));
        // The real ones have the same shape.
        let real = fresh_token();
        assert_eq!(real.len(), 9);
        assert!(!real.starts_with('0'));
    }

    #[test]
    fn transfers_a_small_file_on_localhost() {
        let dir = std::env::temp_dir().join(format!("openirc-dcc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("hello.txt");
        std::fs::write(&source, b"hello dcc").unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let send_path = source.clone();
        let sender = std::thread::spawn(move || {
            serve_file(listener, &send_path, &Meter::default()).unwrap()
        });
        let dest = dir.join("out.txt");
        let file = create_overwrite(&dest).unwrap();
        let received =
            receive_from("127.0.0.1", port, file, 0, Some(9), &Meter::default()).unwrap();
        assert_eq!(received, 9);
        assert_eq!(std::fs::read(dest).unwrap(), b"hello dcc");
        assert_eq!(sender.join().unwrap(), 9);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn builds_offer_privmsg() {
        assert_eq!(
            offer_message("bob", "file.txt", 2130706433, 5000, 1234),
            "PRIVMSG bob :\x01DCC SEND file.txt 2130706433 5000 1234\x01"
        );
    }

    #[test]
    fn safe_filename_keeps_downloads_inside_the_folder() {
        assert_eq!(safe_filename("../../etc/passwd"), "passwd");
        assert_eq!(safe_filename("C:evil.exe"), "Cevil.exe");
        assert_eq!(safe_filename("..\\..\\boot.ini"), "boot.ini");
        assert_eq!(safe_filename("CON.txt"), "_CON.txt");
        assert_eq!(safe_filename("com1"), "_com1");
        assert_eq!(safe_filename("a\u{7}b?.txt. "), "ab.txt");
        assert_eq!(safe_filename(".."), "download.bin");
        assert_eq!(safe_filename(""), "download.bin");
    }

    #[test]
    fn unique_path_does_not_overwrite_an_existing_download() {
        let dir = std::env::temp_dir().join(format!("openirc-unique-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a.txt"));
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (1).txt"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn filenames_with_spaces_are_quoted_and_parse_back() {
        let line = offer_message("bob", "my file.txt", 2130706433, 5000, 3);
        let payload = line.split_once(" :").unwrap().1.trim_matches('\x01');
        let offer = parse_dcc_send(payload).unwrap();
        assert_eq!(offer.filename, "my file.txt");
    }

    #[test]
    fn a_larger_file_does_not_stall_on_acks() {
        let dir = std::env::temp_dir().join(format!("openirc-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("big.bin");
        let data: Vec<u8> = (0..2_000_000u32).map(|n| n as u8).collect();
        std::fs::write(&source, &data).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let send_path = source.clone();
        let started = std::time::Instant::now();
        let sender = std::thread::spawn(move || {
            serve_file(listener, &send_path, &Meter::default()).unwrap()
        });
        let dest = dir.join("out.bin");
        let meter = Meter::default();
        let file = create_overwrite(&dest).unwrap();
        let received =
            receive_from("127.0.0.1", port, file, 0, Some(data.len() as u64), &meter).unwrap();
        assert_eq!(received, data.len() as u64);
        assert_eq!(meter.bytes(), data.len() as u64);
        assert_eq!(sender.join().unwrap(), data.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert!(started.elapsed() < Duration::from_secs(10));
        let _ = std::fs::remove_dir_all(dir);
    }
}
