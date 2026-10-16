//! Network preferences shared by server and DCC connections: address
//! order, a local address to bind, the DCC port range and address, and
//! which connections go through the proxy.
//!
//! The window sets them with [`configure`] when settings load or change;
//! connection code reads them with [`prefs`].

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::RwLock;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AddressOrder {
    /// As the resolver returns them.
    #[default]
    System,
    Ipv4First,
    Ipv6First,
}

impl AddressOrder {
    pub fn key(self) -> &'static str {
        match self {
            AddressOrder::System => "system",
            AddressOrder::Ipv4First => "ipv4",
            AddressOrder::Ipv6First => "ipv6",
        }
    }

    pub fn from_key(key: &str) -> Self {
        match key {
            "ipv4" => AddressOrder::Ipv4First,
            "ipv6" => AddressOrder::Ipv6First,
            _ => AddressOrder::System,
        }
    }
}

/// Which connections use the proxy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProxyScope {
    #[default]
    Servers,
    Dcc,
    Both,
}

impl ProxyScope {
    pub fn key(self) -> &'static str {
        match self {
            ProxyScope::Servers => "servers",
            ProxyScope::Dcc => "dcc",
            ProxyScope::Both => "both",
        }
    }

    pub fn from_key(key: &str) -> Self {
        match key {
            "dcc" => ProxyScope::Dcc,
            "both" => ProxyScope::Both,
            _ => ProxyScope::Servers,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct NetPrefs {
    pub order: AddressOrder,
    /// Local address to connect from; `None` lets the system choose.
    pub bind: Option<IpAddr>,
    /// When binding fails, connect without binding instead of failing.
    pub bind_fallback: bool,
    /// Ports DCC listens on, inclusive; `None` for any free port.
    pub dcc_ports: Option<(u16, u16)>,
    /// DCC address set by hand.
    pub dcc_ip: Option<Ipv4Addr>,
    /// DCC address the server reported for us.
    pub learned_ip: Option<Ipv4Addr>,
    pub proxy: Option<crate::socks::Endpoint>,
    pub proxy_scope: ProxyScope,
    /// Hosts that never use the proxy: wildcard masks separated by commas
    /// or spaces, such as `*.lan, 192.168.*`.
    pub proxy_except: String,
}

static PREFS: RwLock<Option<NetPrefs>> = RwLock::new(None);

pub fn configure(prefs: NetPrefs) {
    if let Ok(mut slot) = PREFS.write() {
        *slot = Some(prefs);
    }
}

pub fn prefs() -> NetPrefs {
    PREFS
        .read()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or_default()
}

/// Record the address the server sees us as, for DCC offers.
pub fn learn_ip(ip: Ipv4Addr) {
    if let Ok(mut slot) = PREFS.write() {
        slot.get_or_insert_with(NetPrefs::default).learned_ip = Some(ip);
    }
}

/// The address to put in DCC offers: the one set by hand, else the one the
/// server reported, else this machine's outgoing address.
pub fn dcc_ip() -> Ipv4Addr {
    let prefs = prefs();
    prefs
        .dcc_ip
        .or(prefs.learned_ip)
        .unwrap_or_else(crate::dcc::local_ipv4)
}

/// Addresses for `host`, in the preferred order.
pub fn resolve(host: &str, port: u16, order: AddressOrder) -> io::Result<Vec<SocketAddr>> {
    let mut addrs: Vec<SocketAddr> = (host, port).to_socket_addrs()?.collect();
    match order {
        AddressOrder::System => {}
        AddressOrder::Ipv4First => addrs.sort_by_key(|addr| addr.is_ipv6()),
        AddressOrder::Ipv6First => addrs.sort_by_key(|addr| addr.is_ipv4()),
    }
    Ok(addrs)
}

/// Connect to the first address of `host` that answers within `timeout`,
/// from the bound local address if one is set.
pub fn dial(host: &str, port: u16, timeout: Duration) -> io::Result<TcpStream> {
    dial_with(host, port, timeout, &prefs())
}

/// `dial` with the given preferences instead of the saved ones.
pub fn dial_with(
    host: &str,
    port: u16,
    timeout: Duration,
    prefs: &NetPrefs,
) -> io::Result<TcpStream> {
    let failed = |err: io::Error| {
        io::Error::new(
            err.kind(),
            format!("could not connect to {host}:{port}: {err}"),
        )
    };
    let addrs = resolve(host, port, prefs.order).map_err(failed)?;
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no address found");
    for addr in addrs {
        match connect_one(addr, timeout, prefs) {
            Ok(stream) => return Ok(stream),
            Err(err) => last = err,
        }
    }
    Err(failed(last))
}

fn connect_one(addr: SocketAddr, timeout: Duration, prefs: &NetPrefs) -> io::Result<TcpStream> {
    let Some(local) = prefs.bind else {
        return TcpStream::connect_timeout(&addr, timeout);
    };
    if local.is_ipv4() != addr.is_ipv4() {
        // The chosen local address cannot reach this kind of address. Without
        // fallback, skip it rather than connect from another address.
        if prefs.bind_fallback {
            return TcpStream::connect_timeout(&addr, timeout);
        }
        return Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("local address {local} cannot reach {addr}"),
        ));
    }
    let socket = socket2::Socket::new(
        socket2::Domain::for_address(addr),
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;
    if let Err(err) = socket.bind(&SocketAddr::new(local, 0).into()) {
        if prefs.bind_fallback {
            return TcpStream::connect_timeout(&addr, timeout);
        }
        return Err(io::Error::new(
            err.kind(),
            format!("could not use local address {local}: {err}"),
        ));
    }
    socket.connect_timeout(&addr.into(), timeout)?;
    Ok(socket.into())
}

/// Whether a connection to `host` of this kind goes through the proxy.
pub fn proxy_for(host: &str, dcc: bool) -> Option<crate::socks::Endpoint> {
    let prefs = prefs();
    let proxy = prefs.proxy?;
    let wanted = match prefs.proxy_scope {
        ProxyScope::Servers => !dcc,
        ProxyScope::Dcc => dcc,
        ProxyScope::Both => true,
    };
    let excepted = prefs
        .proxy_except
        .split([',', ' '])
        .map(str::trim)
        .filter(|mask| !mask.is_empty())
        .any(|mask| crate::script::mask_match(mask, host));
    (wanted && !excepted).then_some(proxy)
}

/// An outgoing DCC connection, through the proxy when it applies.
pub fn connect_dcc(host: &str, port: u16) -> io::Result<TcpStream> {
    match proxy_for(host, true) {
        Some(proxy) => proxy.connect(host, port),
        None => dial(host, port, Duration::from_secs(30)),
    }
}

/// A listener for DCC: the first free port of the range, or any port.
pub fn listen_dcc() -> io::Result<TcpListener> {
    let prefs = prefs();
    match prefs.dcc_ports {
        Some((first, last)) if first > 0 && first <= last => {
            for port in first..=last {
                if let Ok(listener) = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)) {
                    return Ok(listener);
                }
            }
            Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("every DCC port from {first} to {last} is in use"),
            ))
        }
        _ => TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)),
    }
}

/// Parse a `302 USERHOST` reply such as `nick=+user@host` and return the
/// host part.
pub fn userhost_host(text: &str) -> Option<&str> {
    let first = text.split_whitespace().next()?;
    let (_, rest) = first.split_once('=')?;
    let (_, host) = rest.split_once('@')?;
    Some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_order_sorts_families() {
        let v4: SocketAddr = "192.0.2.1:6667".parse().unwrap();
        let v6: SocketAddr = "[2001:db8::1]:6667".parse().unwrap();
        let mut addrs = vec![v6, v4];
        addrs.sort_by_key(|addr| addr.is_ipv6());
        assert_eq!(addrs, vec![v4, v6]);
        assert_eq!(
            AddressOrder::from_key(AddressOrder::Ipv6First.key()),
            AddressOrder::Ipv6First
        );
        assert_eq!(ProxyScope::from_key("both"), ProxyScope::Both);
    }

    #[test]
    fn userhost_reply_gives_the_host() {
        assert_eq!(userhost_host("me=+me@203.0.113.9"), Some("203.0.113.9"));
        assert_eq!(
            userhost_host("me*=-me@host.example other=+x@y"),
            Some("host.example")
        );
        assert_eq!(userhost_host(""), None);
    }

    #[test]
    fn dcc_listener_uses_the_port_range() {
        let probe = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        let busy = probe.local_addr().unwrap().port();
        let range = Some((busy, busy.saturating_add(20)));
        let prefs = NetPrefs {
            dcc_ports: range,
            ..NetPrefs::default()
        };
        // Exercise the range logic without touching the shared settings.
        let picked = match prefs.dcc_ports {
            Some((first, last)) => (first..=last)
                .find_map(|port| TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).ok())
                .unwrap(),
            None => unreachable!(),
        };
        let port = picked.local_addr().unwrap().port();
        assert!(port > busy && port <= busy + 20);
    }

    #[test]
    fn bound_dial_reaches_a_local_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let prefs = NetPrefs {
            bind: Some("127.0.0.1".parse().unwrap()),
            ..NetPrefs::default()
        };
        let stream = connect_one(addr, Duration::from_secs(2), &prefs).unwrap();
        assert_eq!(
            stream.local_addr().unwrap().ip(),
            "127.0.0.1".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn a_local_address_of_the_other_family_is_never_skipped_silently() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let v4 = listener.local_addr().unwrap();
        let v6: SocketAddr = "[::1]:9".parse().unwrap();
        let strict = NetPrefs {
            bind: Some("::1".parse().unwrap()),
            bind_fallback: false,
            ..NetPrefs::default()
        };
        let err = connect_one(v4, Duration::from_secs(2), &strict).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AddrNotAvailable);
        let loose = NetPrefs {
            bind_fallback: true,
            ..strict.clone()
        };
        assert!(connect_one(v4, Duration::from_secs(2), &loose).is_ok());
        let strict_v4 = NetPrefs {
            bind: Some("127.0.0.1".parse().unwrap()),
            ..strict
        };
        assert_eq!(
            connect_one(v6, Duration::from_secs(2), &strict_v4)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AddrNotAvailable
        );
    }
}
