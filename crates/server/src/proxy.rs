//! Which peers may tell us the real client address, and how to read it.

use std::net::IpAddr;

#[derive(Debug, Clone, Default)]
pub struct ProxyList {
    ranges: Vec<(IpAddr, u8)>,
}

impl ProxyList {
    /// Comma-separated IPs or CIDR ranges; empty means "trust no proxy".
    pub fn parse(input: &str) -> Result<Self, String> {
        let ranges = input
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|entry| {
                let (addr, prefix) = entry
                    .split_once('/')
                    .map_or((entry, None), |(a, p)| (a, Some(p)));
                let addr: IpAddr = addr
                    .parse()
                    .map_err(|_| format!("{entry:?} is not an IP address or CIDR range"))?;
                let max = if addr.is_ipv4() { 32 } else { 128 };
                let prefix = match prefix {
                    None => max,
                    Some(p) => p
                        .parse::<u8>()
                        .ok()
                        .filter(|p| *p <= max)
                        .ok_or(format!("{entry:?} has an invalid prefix length"))?,
                };
                // Peers and hops are canonicalised, so `::ffff:a.b.c.d/n` must match as IPv4.
                // A prefix shorter than 96 reaches beyond the mapped range and stays IPv6.
                Ok(match addr.to_canonical() {
                    v4 @ IpAddr::V4(_) if addr.is_ipv6() && prefix >= 96 => (v4, prefix - 96),
                    _ => (addr, prefix),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self { ranges })
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        self.ranges.iter().any(|&(net, prefix)| match (net, ip) {
            (IpAddr::V4(n), IpAddr::V4(i)) => masked(
                u128::from(u32::from(n)),
                u128::from(u32::from(i)),
                prefix,
                32,
            ),
            (IpAddr::V6(n), IpAddr::V6(i)) => masked(u128::from(n), u128::from(i), prefix, 128),
            _ => false,
        })
    }
}

fn masked(net: u128, ip: u128, prefix: u8, bits: u32) -> bool {
    let host_bits = bits - u32::from(prefix);
    if host_bits >= 128 {
        return true;
    }
    (net >> host_bits) == (ip >> host_bits)
}

/// The client address used for rate limiting. `X-Forwarded-For` counts only when the TCP peer is a
/// trusted proxy; then the rightmost entry that is not itself a trusted proxy is the client.
/// Anything unparsable stops the walk and the peer is used: entries left of it are
/// client-controlled. IPv4-mapped IPv6 addresses (`::ffff:a.b.c.d`, what a dual-stack listener
/// sees) are read as IPv4, so one client has one key.
pub fn client_ip(peer: IpAddr, forwarded_for: Option<&str>, trusted: &ProxyList) -> IpAddr {
    let peer = peer.to_canonical();
    if !trusted.contains(peer) {
        return peer;
    }
    let Some(header) = forwarded_for else {
        return peer;
    };
    for hop in header.rsplit(',') {
        let Ok(ip) = hop.trim().parse::<IpAddr>() else {
            return peer;
        };
        let ip = ip.to_canonical();
        if !trusted.contains(ip) {
            return ip;
        }
    }
    peer
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn parse_accepts_ips_cidrs_and_empty() {
        let list = ProxyList::parse("10.0.0.5, 172.16.0.0/12,fd00::/8").unwrap();
        assert!(list.contains(ip("10.0.0.5")));
        assert!(list.contains(ip("172.31.250.3")));
        assert!(!list.contains(ip("172.32.0.1")));
        assert!(list.contains(ip("fd00::1")));
        assert!(!list.contains(ip("10.0.0.6")));
        assert!(ProxyList::parse("").unwrap().is_empty());
        assert!(ProxyList::parse("10.0.0.0/33").is_err());
        assert!(ProxyList::parse("nonsense").is_err());
    }

    #[test]
    fn parse_handles_zero_prefix_and_rejects_bad_prefixes() {
        let all = ProxyList::parse("0.0.0.0/0, ::/0").unwrap();
        assert!(all.contains(ip("203.0.113.9")));
        assert!(all.contains(ip("2001:db8::1")));
        assert!(ProxyList::parse("fd00::/129").is_err());
        assert!(ProxyList::parse("10.0.0.0/").is_err());
        assert!(ProxyList::parse("10.0.0.0/-1").is_err());
    }

    #[test]
    fn parse_reads_ipv4_mapped_entries_as_ipv4() {
        let list = ProxyList::parse("::ffff:10.0.0.5, ::ffff:172.16.0.0/108").unwrap();
        assert!(list.contains(ip("10.0.0.5")));
        assert!(list.contains(ip("172.16.3.4")));
        assert!(!list.contains(ip("172.32.0.1")));
        assert_eq!(
            client_ip(ip("10.0.0.5"), Some("198.51.100.7"), &list),
            ip("198.51.100.7")
        );
    }

    #[test]
    fn client_ip_ignores_header_from_untrusted_peer() {
        let trusted = ProxyList::parse("10.0.0.5").unwrap();
        assert_eq!(
            client_ip(ip("203.0.113.9"), Some("1.2.3.4"), &trusted),
            ip("203.0.113.9")
        );
        // An empty list trusts nobody, whatever the header says.
        assert_eq!(
            client_ip(ip("10.0.0.5"), Some("1.2.3.4"), &ProxyList::default()),
            ip("10.0.0.5")
        );
    }

    #[test]
    fn client_ip_takes_rightmost_untrusted_hop() {
        let trusted = ProxyList::parse("10.0.0.0/8").unwrap();
        // Leftmost entry is client-controlled and ignored.
        assert_eq!(
            client_ip(
                ip("10.0.0.5"),
                Some("6.6.6.6, 198.51.100.7, 10.0.0.9"),
                &trusted
            ),
            ip("198.51.100.7")
        );
        assert_eq!(
            client_ip(ip("10.0.0.5"), Some("2001:db8::1"), &trusted),
            ip("2001:db8::1")
        );
    }

    #[test]
    fn client_ip_spoofed_trusted_entries_on_the_left_do_not_help() {
        let trusted = ProxyList::parse("10.0.0.0/8").unwrap();
        // The client wrote "10.1.1.1, 9.9.9.9"; our proxy appended the real address.
        assert_eq!(
            client_ip(
                ip("10.0.0.5"),
                Some("10.1.1.1, 9.9.9.9, 198.51.100.7"),
                &trusted
            ),
            ip("198.51.100.7")
        );
    }

    #[test]
    fn client_ip_ipv6_hops_and_ipv4_mapped_peers() {
        let trusted = ProxyList::parse("10.0.0.0/8, fd00::/8").unwrap();
        assert_eq!(
            client_ip(ip("fd00::2"), Some("2001:db8::7, fd00::3"), &trusted),
            ip("2001:db8::7")
        );
        // A dual-stack listener sees IPv4 peers as ::ffff:a.b.c.d.
        assert_eq!(
            client_ip(ip("::ffff:10.0.0.5"), Some("198.51.100.7"), &trusted),
            ip("198.51.100.7")
        );
        assert_eq!(
            client_ip(ip("::ffff:203.0.113.9"), Some("198.51.100.7"), &trusted),
            ip("203.0.113.9")
        );
        assert_eq!(
            client_ip(
                ip("10.0.0.5"),
                Some("::ffff:10.0.0.9, ::ffff:198.51.100.7"),
                &trusted
            ),
            ip("198.51.100.7")
        );
    }

    #[test]
    fn client_ip_falls_back_to_peer_on_missing_or_malformed_header() {
        let trusted = ProxyList::parse("10.0.0.5").unwrap();
        assert_eq!(client_ip(ip("10.0.0.5"), None, &trusted), ip("10.0.0.5"));
        assert_eq!(
            client_ip(ip("10.0.0.5"), Some("garbage"), &trusted),
            ip("10.0.0.5")
        );
        assert_eq!(
            client_ip(ip("10.0.0.5"), Some("10.0.0.5"), &trusted),
            ip("10.0.0.5")
        );
        assert_eq!(
            client_ip(ip("10.0.0.5"), Some(""), &trusted),
            ip("10.0.0.5")
        );
        // A malformed hop stops the walk: entries left of it are client-controlled.
        assert_eq!(
            client_ip(ip("10.0.0.5"), Some("198.51.100.7, garbage"), &trusted),
            ip("10.0.0.5")
        );
        assert_eq!(
            client_ip(ip("10.0.0.5"), Some("[2001:db8::1]:443"), &trusted),
            ip("10.0.0.5")
        );
        assert_eq!(
            client_ip(ip("10.0.0.5"), Some("198.51.100.7:443"), &trusted),
            ip("10.0.0.5")
        );
    }
}
