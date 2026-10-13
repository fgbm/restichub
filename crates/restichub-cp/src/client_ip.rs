//! The client IP address of a request (ADR-0023, section Audit log).
//!
//! The control plane uses the address of the socket by default. It uses the `X-Forwarded-For`
//! header only when the socket peer is a trusted proxy. It then takes the right-most address that
//! is not a trusted proxy.

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;

use crate::error::ApiError;
use crate::server::AppState;

/// The networks whose `X-Forwarded-For` header the control plane trusts.
#[derive(Debug, Clone, Default)]
pub struct TrustedProxies {
    nets: Vec<(IpAddr, u8)>,
}

impl TrustedProxies {
    /// Parses a list of networks.
    ///
    /// Each entry is an address or a network in CIDR notation, for example `10.0.0.1` or
    /// `10.0.0.0/8`. An empty entry is ignored. The function returns a message on a bad entry.
    pub fn parse<'a, I>(entries: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let mut nets = Vec::new();
        for entry in entries {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            nets.push(parse_net(entry)?);
        }
        Ok(Self { nets })
    }

    /// Returns true if the list holds no network.
    pub fn is_empty(&self) -> bool {
        self.nets.is_empty()
    }

    /// Returns true if the address is in a trusted network.
    pub fn contains(&self, address: IpAddr) -> bool {
        let address = address.to_canonical();
        self.nets
            .iter()
            .any(|&(net, prefix)| in_net(address, net, prefix))
    }
}

/// Parses one network entry. The function canonicalizes the address.
fn parse_net(entry: &str) -> Result<(IpAddr, u8), String> {
    let (address, prefix) = match entry.split_once('/') {
        Some((address, prefix)) => (address, Some(prefix)),
        None => (entry, None),
    };
    let address: IpAddr = address
        .parse()
        .map_err(|_| format!("`{entry}` is not an address or a network"))?;
    let address = address.to_canonical();
    let width = if address.is_ipv4() { 32 } else { 128 };
    let prefix: u8 = match prefix {
        Some(value) => value
            .parse()
            .map_err(|_| format!("`{entry}` has no valid prefix"))?,
        None => width,
    };
    if prefix > width {
        return Err(format!("`{entry}` has a prefix larger than {width}"));
    }
    Ok((address, prefix))
}

/// Returns true if the address is in the network.
fn in_net(address: IpAddr, net: IpAddr, prefix: u8) -> bool {
    match (address, net) {
        (IpAddr::V4(address), IpAddr::V4(net)) => {
            mask_matches(u32::from(address), u32::from(net), prefix, 32)
        }
        (IpAddr::V6(address), IpAddr::V6(net)) => {
            mask_matches(u128::from(address), u128::from(net), prefix, 128)
        }
        _ => false,
    }
}

/// Compares the top `prefix` bits of two values.
fn mask_matches<T>(address: T, net: T, prefix: u8, width: u8) -> bool
where
    T: BitAndMask,
{
    let shift = width - prefix;
    address.masked(shift) == net.masked(shift)
}

/// A value with a bit mask.
trait BitAndMask: Copy + PartialEq {
    /// Returns the value with the low `shift` bits cleared.
    ///
    /// The function returns zero when `shift` is not less than the width of the type. A zero mask
    /// matches every address, also for the prefix `/0`.
    fn masked(self, shift: u8) -> Self;
}

impl BitAndMask for u32 {
    fn masked(self, shift: u8) -> Self {
        let mask = u32::MAX.checked_shl(u32::from(shift)).unwrap_or(0);
        self & mask
    }
}

impl BitAndMask for u128 {
    fn masked(self, shift: u8) -> Self {
        let mask = u128::MAX.checked_shl(u32::from(shift)).unwrap_or(0);
        self & mask
    }
}

/// The client IP address of a request.
///
/// The extractor reads the socket peer. If the peer is a trusted proxy, it reads the
/// `X-Forwarded-For` header and takes the right-most address that is not a trusted proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(address)| address.ip())
            .ok_or_else(ApiError::internal_error)?;
        let forwarded = forwarded_for(parts);
        Ok(Self(resolve_client_ip(
            peer,
            forwarded.as_deref(),
            state.trusted_proxies(),
        )))
    }
}

/// Returns the joined `X-Forwarded-For` header lines in order.
///
/// The function joins the header lines with a comma. A client cannot reorder the lines. If any
/// line is not readable text, the function returns `None`. The caller then uses the socket peer.
fn forwarded_for(parts: &Parts) -> Option<String> {
    let mut lines = Vec::new();
    for value in parts.headers.get_all("x-forwarded-for") {
        match value.to_str() {
            Ok(line) => lines.push(line.to_owned()),
            Err(_) => return None,
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join(","))
    }
}

/// Returns the client IP address from the peer and the forwarded chain.
///
/// If the peer is not a trusted proxy, the function returns the peer. If the peer is a trusted
/// proxy, the function walks the header from the right. It returns the first address that is not a
/// trusted proxy. The function stops at the first entry that it cannot parse and returns the last
/// trusted hop or the peer. It never continues to the left of that entry.
pub fn resolve_client_ip(
    peer: IpAddr,
    forwarded: Option<&str>,
    trusted: &TrustedProxies,
) -> IpAddr {
    let peer = peer.to_canonical();
    if !trusted.contains(peer) {
        return peer;
    }
    let Some(forwarded) = forwarded else {
        return peer;
    };
    let mut candidate = peer;
    for entry in forwarded.split(',').rev() {
        let Some(address) = parse_address(entry) else {
            break;
        };
        if trusted.contains(address) {
            candidate = address;
        } else {
            return address;
        }
    }
    candidate
}

/// Parses one header entry. The function removes a port and canonicalizes the address.
fn parse_address(entry: &str) -> Option<IpAddr> {
    let entry = entry.trim();
    if entry.is_empty() {
        return None;
    }
    // The bracketed IPv6 form `[2001:db8::1]:443`.
    if let Some(rest) = entry.strip_prefix('[') {
        let (address, _) = rest.split_once(']')?;
        return address.parse::<IpAddr>().ok().map(|ip| ip.to_canonical());
    }
    // The IPv4 form `203.0.113.7:443`. A bare IPv6 address holds more than one colon, so the
    // function splits on a single colon only.
    let host = match entry.split_once(':') {
        Some((host, port)) if !host.contains(':') && port.parse::<u16>().is_ok() => host,
        _ => entry,
    };
    host.parse::<IpAddr>().ok().map(|ip| ip.to_canonical())
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::{TrustedProxies, resolve_client_ip};

    fn ip(value: &str) -> IpAddr {
        value
            .parse()
            .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
    }

    fn trusted(entries: &[&str]) -> TrustedProxies {
        match TrustedProxies::parse(entries.iter().copied()) {
            Ok(trusted) => trusted,
            Err(error) => panic!("cannot parse the networks: {error}"),
        }
    }

    #[test]
    fn a_trusted_proxy_list_matches_networks() {
        let list = trusted(&["10.0.0.0/8", "2001:db8::/32"]);
        assert!(list.contains(ip("10.1.2.3")));
        assert!(!list.contains(ip("11.0.0.1")));
        assert!(list.contains(ip("2001:db8::1")));
        assert!(!list.contains(ip("2001:db9::1")));
    }

    #[test]
    fn a_bad_network_is_refused() {
        assert!(TrustedProxies::parse(["10.0.0.0/33"]).is_err());
        assert!(TrustedProxies::parse(["not-an-address"]).is_err());
        assert!(TrustedProxies::parse(["10.0.0.0/8", ""]).is_ok());
    }

    #[test]
    fn a_zero_prefix_matches_every_address() {
        let v4 = trusted(&["0.0.0.0/0"]);
        assert!(v4.contains(ip("203.0.113.7")));
        assert!(v4.contains(ip("10.1.2.3")));
        let v6 = trusted(&["::/0"]);
        assert!(v6.contains(ip("2001:db8::1")));
        assert!(v6.contains(ip("2001:db8::2")));
    }

    #[test]
    fn a_full_prefix_matches_one_address() {
        let v4 = trusted(&["203.0.113.7/32"]);
        assert!(v4.contains(ip("203.0.113.7")));
        assert!(!v4.contains(ip("203.0.113.8")));
        let v6 = trusted(&["2001:db8::1/128"]);
        assert!(v6.contains(ip("2001:db8::1")));
        assert!(!v6.contains(ip("2001:db8::2")));
    }

    #[test]
    fn an_ipv4_mapped_address_is_canonical() {
        let list = trusted(&["203.0.113.7"]);
        assert!(list.contains(ip("::ffff:203.0.113.7")));
        assert_eq!(
            resolve_client_ip(ip("::ffff:203.0.113.7"), None, &list),
            ip("203.0.113.7")
        );
    }

    #[test]
    fn the_socket_peer_is_used_without_a_trusted_proxy() {
        let list = trusted(&["10.0.0.0/8"]);
        assert_eq!(
            resolve_client_ip(ip("203.0.113.5"), Some("1.2.3.4"), &list),
            ip("203.0.113.5")
        );
    }

    #[test]
    fn a_spoofed_header_from_an_untrusted_peer_is_ignored() {
        let list = TrustedProxies::default();
        assert_eq!(
            resolve_client_ip(ip("198.51.100.9"), Some("203.0.113.7"), &list),
            ip("198.51.100.9")
        );
    }

    #[test]
    fn a_trusted_proxy_uses_the_header() {
        let list = trusted(&["10.0.0.0/8"]);
        assert_eq!(
            resolve_client_ip(ip("10.0.0.1"), Some("203.0.113.7"), &list),
            ip("203.0.113.7")
        );
    }

    #[test]
    fn the_right_most_untrusted_address_is_taken() {
        let list = trusted(&["10.0.0.0/8"]);
        assert_eq!(
            resolve_client_ip(ip("10.0.0.1"), Some("203.0.113.7, 10.0.0.2"), &list),
            ip("203.0.113.7")
        );
    }

    #[test]
    fn an_injected_left_address_does_not_win() {
        let list = trusted(&["10.0.0.0/8"]);
        assert_eq!(
            resolve_client_ip(ip("10.0.0.1"), Some("1.2.3.4, 203.0.113.7"), &list),
            ip("203.0.113.7")
        );
    }

    #[test]
    fn a_chain_of_trusted_proxies_gives_the_left_most() {
        let list = trusted(&["10.0.0.0/8"]);
        assert_eq!(
            resolve_client_ip(ip("10.0.0.1"), Some("10.0.0.2, 10.0.0.3"), &list),
            ip("10.0.0.2")
        );
    }

    #[test]
    fn two_header_lines_are_joined_in_order() {
        // The two lines join to `6.6.6.6,203.0.113.7`.
        let list = trusted(&["10.0.0.0/8"]);
        assert_eq!(
            resolve_client_ip(ip("10.0.0.1"), Some("6.6.6.6,203.0.113.7"), &list),
            ip("203.0.113.7")
        );
    }

    #[test]
    fn a_port_is_removed() {
        let list = trusted(&["10.0.0.0/8"]);
        assert_eq!(
            resolve_client_ip(ip("10.0.0.1"), Some("6.6.6.6, 203.0.113.7:1"), &list),
            ip("203.0.113.7")
        );
    }

    #[test]
    fn an_unparseable_entry_stops_the_walk() {
        let list = trusted(&["10.0.0.0/8"]);
        // The right-most entry is unknown. The walk stops and returns the peer.
        assert_eq!(
            resolve_client_ip(ip("10.0.0.1"), Some("unknown"), &list),
            ip("10.0.0.1")
        );
        // The right-most entry is a trusted hop. The next entry is unknown.
        assert_eq!(
            resolve_client_ip(ip("10.0.0.1"), Some("1.2.3.4, unknown, 10.0.0.9"), &list),
            ip("10.0.0.9")
        );
    }

    #[test]
    fn an_ipv6_address_with_a_port_is_parsed() {
        let list = trusted(&["10.0.0.0/8"]);
        assert_eq!(
            resolve_client_ip(ip("10.0.0.1"), Some("[2001:db8::1]:443"), &list),
            ip("2001:db8::1")
        );
        // A trusted IPv6 hop with a port is removed from the chain too.
        let list = trusted(&["10.0.0.0/8", "2001:db8::/32"]);
        assert_eq!(
            resolve_client_ip(
                ip("10.0.0.1"),
                Some("203.0.113.7, [2001:db8::9]:443"),
                &list
            ),
            ip("203.0.113.7")
        );
    }

    #[test]
    fn two_header_lines_are_read_in_order() {
        use axum::http::{HeaderMap, HeaderValue, Request};

        let mut headers = HeaderMap::new();
        headers.append("x-forwarded-for", HeaderValue::from_static("6.6.6.6"));
        headers.append("x-forwarded-for", HeaderValue::from_static("203.0.113.7"));
        let mut request = Request::new(());
        *request.headers_mut() = headers;
        let (parts, ()) = request.into_parts();
        assert_eq!(
            super::forwarded_for(&parts).as_deref(),
            Some("6.6.6.6,203.0.113.7")
        );
    }

    #[test]
    fn an_unreadable_header_line_is_ignored() {
        use axum::http::{HeaderMap, HeaderValue, Request};

        let mut headers = HeaderMap::new();
        headers.append("x-forwarded-for", HeaderValue::from_static("6.6.6.6"));
        // The value `0x80` is a valid header value, but it is not readable text.
        let bad = match HeaderValue::from_bytes(b"\x80") {
            Ok(value) => value,
            Err(error) => panic!("cannot build the header value: {error}"),
        };
        headers.append("x-forwarded-for", bad);
        let mut request = Request::new(());
        *request.headers_mut() = headers;
        let (parts, ()) = request.into_parts();
        // The header is ignored, so the resolver uses the peer.
        assert!(super::forwarded_for(&parts).is_none());
        let list = trusted(&["10.0.0.0/8"]);
        assert_eq!(
            resolve_client_ip(
                ip("10.0.0.1"),
                super::forwarded_for(&parts).as_deref(),
                &list
            ),
            ip("10.0.0.1")
        );
    }
}
