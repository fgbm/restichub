//! The address policy of the webhook channels (ADR-0036, threat model section Webhook targets).
//!
//! The control plane connects to the address that this check approved, never to a second DNS
//! answer, so a rebinding host name cannot reach a private network.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use axum::http::Uri;
use ipnet::IpNet;

use super::settings::WebhookSettings;

/// Bound in characters for a stored URL.
pub const MAX_URL_LEN: usize = 2048;

/// The reason why the control plane refuses a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UrlError {
    #[error("the URL is not valid")]
    Invalid,
    #[error("the URL must use https")]
    Scheme,
    #[error("the URL must not hold a user name or a fragment")]
    Parts,
    #[error("the URL names an address that the policy forbids")]
    Forbidden,
}

/// A webhook URL that passed the syntax check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub tls: bool,
    pub host: String,
    pub port: u16,
    /// The path and the query, always starting with `/`.
    pub request_uri: String,
}

impl Target {
    /// An IPv6 literal gets brackets, and the default port is omitted.
    pub fn host_header(&self) -> String {
        let default = if self.tls { 443 } else { 80 };
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if self.port == default {
            host
        } else {
            format!("{host}:{}", self.port)
        }
    }
}

/// Parses a webhook URL. A literal IP address is checked at once; a host name is checked after the
/// DNS lookup at the send.
pub fn parse_target(url: &str, policy: &WebhookSettings) -> Result<Target, UrlError> {
    if url.len() > MAX_URL_LEN || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(UrlError::Invalid);
    }
    if url.contains('#') {
        return Err(UrlError::Parts);
    }
    let uri: Uri = url
        .parse()
        .map_err(super::transport::because(UrlError::Invalid))?;
    let tls = match uri.scheme_str() {
        Some("https") => true,
        Some("http") if policy.allow_http => false,
        Some("http") => return Err(UrlError::Scheme),
        _ => return Err(UrlError::Invalid),
    };
    let authority = uri.authority().ok_or(UrlError::Invalid)?;
    if authority.as_str().contains('@') {
        return Err(UrlError::Parts);
    }
    let host = authority.host();
    let host = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host)
        .to_owned();
    if host.is_empty() {
        return Err(UrlError::Invalid);
    }
    let port = authority.port_u16().unwrap_or(if tls { 443 } else { 80 });
    if port == 0 {
        return Err(UrlError::Invalid);
    }
    if let Ok(ip) = host.parse::<IpAddr>()
        && !is_allowed(ip, policy)
    {
        return Err(UrlError::Forbidden);
    }
    let request_uri = uri
        .path_and_query()
        .map_or_else(|| "/".to_owned(), |value| value.as_str().to_owned());
    let request_uri = if request_uri.starts_with('/') {
        request_uri
    } else {
        format!("/{request_uri}")
    };
    Ok(Target {
        tls,
        host,
        port,
        request_uri,
    })
}

/// True when the policy admits the address.
pub fn is_allowed(ip: IpAddr, policy: &WebhookSettings) -> bool {
    let ip = unwrap_embedded_v4(ip);
    if policy.allow_networks.iter().any(|net| net.contains(&ip)) {
        return true;
    }
    !is_forbidden(ip)
}

/// Picks the address to connect to. Every resolved address must pass: one forbidden answer among
/// the others refuses the target, so a mixed answer cannot hide a private address.
pub fn choose_address(
    addresses: &[IpAddr],
    port: u16,
    policy: &WebhookSettings,
) -> Result<SocketAddr, UrlError> {
    if addresses.is_empty() {
        return Err(UrlError::Invalid);
    }
    if addresses.iter().any(|ip| !is_allowed(*ip, policy)) {
        return Err(UrlError::Forbidden);
    }
    Ok(SocketAddr::new(addresses[0], port))
}

/// Maps an IPv6 form that wraps an IPv4 address to that address, so the IPv4 rules apply.
fn unwrap_embedded_v4(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => embedded_v4(v6).map_or(IpAddr::V6(v6), IpAddr::V4),
        v4 => v4,
    }
}

fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = ip.segments();
    let octets = ip.octets();
    let v4 = |a: usize| Ipv4Addr::new(octets[a], octets[a + 1], octets[a + 2], octets[a + 3]);
    // IPv4-mapped ::ffff:a.b.c.d and the deprecated IPv4-compatible ::a.b.c.d.
    if segments[..5] == [0, 0, 0, 0, 0] && (segments[5] == 0xffff || segments[5] == 0) {
        return Some(v4(12));
    }
    // NAT64 64:ff9b::/96.
    if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return Some(v4(12));
    }
    // 6to4 2002::/16.
    if segments[0] == 0x2002 {
        return Some(v4(2));
    }
    None
}

fn is_forbidden(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => forbidden_v4(v4),
        IpAddr::V6(v6) => forbidden_v6(v6),
    }
}

fn forbidden_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_documentation()
        // 0.0.0.0/8, carrier-grade NAT 100.64.0.0/10, IETF protocol 192.0.0.0/24,
        // benchmarking 198.18.0.0/15 and the reserved 240.0.0.0/4.
        || a == 0
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        // The deprecated 6to4 relay anycast 192.88.99.0/24.
        || (a == 192 && b == 88 && c == 99)
        || (a == 198 && (b == 18 || b == 19))
        || a >= 240
}

fn forbidden_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        // Unique local fc00::/7, link-local fe80::/10 and the old site-local fec0::/10.
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || (first & 0xffc0) == 0xfec0
        // Documentation 2001:db8::/32 and the discard prefix 100::/64.
        || (first == 0x2001 && ip.segments()[1] == 0x0db8)
        || (first == 0x0100 && ip.segments()[1..4] == [0, 0, 0])
        // IETF protocol assignments 2001::/23 (Teredo 2001::/32 is inside) and the local-use
        // NAT64 prefix 64:ff9b:1::/48.
        || (first == 0x2001 && ip.segments()[1] < 0x0200)
        || ip.segments()[..3] == [0x64, 0xff9b, 1]
}

/// For the tests: builds the allow-list from CIDR strings.
pub fn networks(values: &[&str]) -> Vec<IpNet> {
    values
        .iter()
        .filter_map(|value| value.parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> WebhookSettings {
        WebhookSettings::default()
    }

    fn allowed(address: &str) -> bool {
        is_allowed(address.parse().expect("an address"), &policy())
    }

    /// AC-MN-05: the address policy refuses the private ranges.
    #[test]
    fn ac_mn_05_forbidden_addresses() {
        for address in [
            "127.0.0.1",
            "127.1.2.3",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "169.254.0.1",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "198.18.0.1",
            "::1",
            "::",
            "fe80::1",
            "fd00::1",
            "fc00::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:169.254.169.254",
            "64:ff9b::7f00:1",
            "2002:7f00:1::",
            "2001:db8::1",
            "64:ff9b:1::1",
            "64:ff9b:1:ffff::1",
            "2001::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            "2001:1ff::1",
            "192.88.99.1",
        ] {
            assert!(!allowed(address), "{address} must be refused");
        }
    }

    /// AC-MN-05
    #[test]
    fn ac_mn_05_public_addresses_pass() {
        for address in [
            "93.184.216.34",
            "8.8.8.8",
            "2606:4700:4700::1111",
            "172.32.0.1",
            "2001:200::1",
            "192.88.98.1",
        ] {
            assert!(allowed(address), "{address} must pass");
        }
    }

    /// AC-MN-05
    #[test]
    fn ac_mn_05_the_allow_list_admits_a_private_network() {
        let policy = WebhookSettings {
            allow_http: false,
            allow_networks: networks(&["10.20.0.0/16", "fd00::/8"]),
        };
        assert!(is_allowed("10.20.1.1".parse().expect("ip"), &policy));
        assert!(!is_allowed("10.21.1.1".parse().expect("ip"), &policy));
        assert!(is_allowed("fd00::5".parse().expect("ip"), &policy));
    }

    /// AC-MN-05
    #[test]
    fn ac_mn_05_one_forbidden_answer_refuses_the_target() {
        let addresses: Vec<IpAddr> = vec![
            "93.184.216.34".parse().expect("ip"),
            "10.0.0.5".parse().expect("ip"),
        ];
        assert_eq!(
            choose_address(&addresses, 443, &policy()),
            Err(UrlError::Forbidden)
        );
        assert!(choose_address(&addresses[..1], 443, &policy()).is_ok());
    }

    /// AC-MN-05
    #[test]
    fn ac_mn_05_the_url_check() {
        let policy = policy();
        assert!(parse_target("https://hooks.example.com/in/abc?x=1", &policy).is_ok());
        assert_eq!(
            parse_target("http://hooks.example.com/", &policy),
            Err(UrlError::Scheme)
        );
        assert_eq!(
            parse_target("https://user:pass@hooks.example.com/", &policy),
            Err(UrlError::Parts)
        );
        assert_eq!(
            parse_target("https://hooks.example.com/#frag", &policy),
            Err(UrlError::Parts)
        );
        assert_eq!(
            parse_target("https://127.0.0.1/", &policy),
            Err(UrlError::Forbidden)
        );
        assert_eq!(
            parse_target("https://[::1]:8443/", &policy),
            Err(UrlError::Forbidden)
        );
        assert_eq!(
            parse_target("https://169.254.169.254/latest/meta-data", &policy),
            Err(UrlError::Forbidden)
        );
        assert_eq!(
            parse_target("ftp://example.com/", &policy),
            Err(UrlError::Invalid)
        );
        assert_eq!(parse_target("https://", &policy), Err(UrlError::Invalid));
        let target = parse_target("https://hooks.example.com:8443", &policy).expect("target");
        assert_eq!(target.request_uri, "/");
        assert_eq!(target.host_header(), "hooks.example.com:8443");
    }

    /// AC-MN-05
    #[test]
    fn ac_mn_05_plain_http_needs_the_setting() {
        let policy = WebhookSettings {
            allow_http: true,
            allow_networks: Vec::new(),
        };
        assert!(parse_target("http://hooks.example.com/", &policy).is_ok());
    }
}
