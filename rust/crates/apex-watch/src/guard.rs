//! The resolve-time request guard.
//!
//! Before any fetch the watched host is resolved through a [`HostResolver`]
//! and every answer must be a public address. Private, loopback, link-local,
//! shared, multicast, unspecified, broadcast and reserved answers are
//! refused with a typed [`SsrfRefusal`], named by class and address, and no
//! request is attempted. IPv4-mapped IPv6 answers are unwrapped and checked
//! as their IPv4 address, so `::ffff:127.0.0.1` is refused exactly like
//! `127.0.0.1`.
//!
//! The refusal travels into the subscriber's recorded finding with the exact
//! copy [`REFUSAL_MESSAGE`]; the class and address ride in the report for the
//! operator. The resolver is a seam so tests can prove that a hostile
//! answer produces zero fetch attempts.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};

/// The message recorded for every guard refusal.
pub const REFUSAL_MESSAGE: &str = "domain resolves to a private address; not fetched";

/// Resolve a host to its addresses. The production implementation is
/// [`StdResolver`]; tests inject fixed answers.
pub trait HostResolver: Send + Sync + std::fmt::Debug {
    /// All addresses the host name resolves to.
    fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, String>;
}

/// The standard library resolver (`std::net::ToSocketAddrs`).
#[derive(Debug, Clone, Copy, Default)]
pub struct StdResolver;

impl HostResolver for StdResolver {
    fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, String> {
        let addresses: Vec<IpAddr> = (host, 0u16)
            .to_socket_addrs()
            .map_err(|error| error.to_string())?
            .map(|address| address.ip())
            .collect();
        if addresses.is_empty() {
            return Err("the resolver returned no addresses".to_string());
        }
        Ok(addresses)
    }
}

/// One refused answer: the address and the class that refused it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SsrfRefusal {
    /// The offending address.
    pub address: IpAddr,
    /// The class name (`private`, `loopback`, `link-local`, `shared`,
    /// `multicast`, `unspecified`, `broadcast`, `reserved`).
    pub class: &'static str,
}

impl SsrfRefusal {
    /// The exact copy recorded for the subscriber.
    pub fn message(&self) -> &'static str {
        REFUSAL_MESSAGE
    }
}

/// Refuse the first non-public address in `addresses`.
pub fn guard_hosts(addresses: &[IpAddr]) -> Result<(), SsrfRefusal> {
    for address in addresses {
        if let Some(refusal) = classify(*address) {
            return Err(refusal);
        }
    }
    Ok(())
}

/// Classify one address; `None` means public and fetchable.
pub fn classify(ip: IpAddr) -> Option<SsrfRefusal> {
    match ip {
        IpAddr::V4(v4) => ipv4_class(v4).map(|class| SsrfRefusal { address: ip, class }),
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return ipv4_class(mapped).map(|class| SsrfRefusal { address: ip, class });
            }
            ipv6_class(v6).map(|class| SsrfRefusal { address: ip, class })
        }
    }
}

fn ipv4_class(v4: Ipv4Addr) -> Option<&'static str> {
    let octets = v4.octets();
    if v4.is_private() {
        Some("private")
    } else if v4.is_loopback() {
        Some("loopback")
    } else if v4.is_link_local() {
        Some("link-local")
    } else if v4.is_multicast() {
        Some("multicast")
    } else if v4.is_broadcast() {
        Some("broadcast")
    } else if v4.is_unspecified() || octets[0] == 0 {
        Some("unspecified")
    } else if v4.is_documentation()
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        || octets[0] >= 240
    {
        Some("reserved")
    } else if octets[0] == 100 && (64..128).contains(&octets[1]) {
        // 100.64.0.0/10 shared address space; the cloud metadata endpoint
        // 100.100.100.200 lives inside it.
        Some("shared")
    } else {
        None
    }
}

fn ipv6_class(v6: Ipv6Addr) -> Option<&'static str> {
    let segments = v6.segments();
    if v6.is_unspecified() {
        Some("unspecified")
    } else if v6.is_loopback() {
        Some("loopback")
    } else if segments[0] & 0xffc0 == 0xfe80 {
        // fe80::/10 link-local.
        Some("link-local")
    } else if segments[0] & 0xfe00 == 0xfc00 {
        // fc00::/7 unique local.
        Some("private")
    } else if segments[0] & 0xff00 == 0xff00 {
        // ff00::/8 multicast.
        Some("multicast")
    } else if segments[..6].iter().all(|segment| *segment == 0) {
        // ::/96 IPv4-compatible space (mapped addresses were unwrapped
        // above), including ::ffff:0:0 style shims.
        Some("reserved")
    } else if segments[0] == 0x0064 && segments[1] == 0xff9b {
        // 64:ff9b::/96 and 64:ff9b:1::/48 NAT64 translation.
        Some("reserved")
    } else if segments[0] == 0x2002 {
        // 6to4 can embed an arbitrary (including private) IPv4 address.
        Some("reserved")
    } else if segments[0] == 0x2001 && segments[1] == 0 {
        // Teredo (2001::/32) tunnels an arbitrary IPv4 endpoint.
        Some("reserved")
    } else if segments[0] == 0x2001 && segments[1] == 0x0db8 {
        // Documentation prefix.
        Some("reserved")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("test address")
    }

    #[test]
    fn private_and_special_ipv4_answers_are_refused() {
        let cases = [
            ("127.0.0.1", "loopback"),
            ("10.0.0.5", "private"),
            ("172.16.3.4", "private"),
            ("192.168.1.1", "private"),
            ("169.254.169.254", "link-local"),
            ("224.0.0.1", "multicast"),
            ("0.0.0.0", "unspecified"),
            ("0.1.2.3", "unspecified"),
            ("255.255.255.255", "broadcast"),
            ("100.64.0.1", "shared"),
            ("100.100.100.200", "shared"),
            ("192.0.2.10", "reserved"),
            ("192.0.0.8", "reserved"),
            ("198.18.0.1", "reserved"),
            ("240.0.0.1", "reserved"),
        ];
        for (address, class) in cases {
            let refusal = classify(ip(address)).unwrap_or_else(|| panic!("{address} allowed"));
            assert_eq!(refusal.class, class, "{address}");
            assert_eq!(refusal.message(), REFUSAL_MESSAGE);
        }
    }

    #[test]
    fn private_and_special_ipv6_answers_are_refused() {
        let cases = [
            ("::1", "loopback"),
            ("::", "unspecified"),
            ("fe80::1", "link-local"),
            ("fc00::1", "private"),
            ("fd12:3456::1", "private"),
            ("ff02::1", "multicast"),
            ("::7f00:1", "reserved"),
            ("64:ff9b::1", "reserved"),
            ("2002::1", "reserved"),
            ("2001::1", "reserved"),
            ("2001:db8::1", "reserved"),
        ];
        for (address, class) in cases {
            let refusal = classify(ip(address)).unwrap_or_else(|| panic!("{address} allowed"));
            assert_eq!(refusal.class, class, "{address}");
        }
    }

    #[test]
    fn mapped_answers_are_unwrapped_and_checked_as_ipv4() {
        for (address, class) in [
            ("::ffff:127.0.0.1", "loopback"),
            ("::ffff:10.0.0.1", "private"),
            ("::ffff:169.254.169.254", "link-local"),
            ("::ffff:100.100.100.200", "shared"),
        ] {
            let refusal = classify(ip(address)).unwrap_or_else(|| panic!("{address} allowed"));
            assert_eq!(refusal.class, class, "{address}");
        }
        // A mapped public address stays fetchable.
        assert_eq!(classify(ip("::ffff:8.8.8.8")), None);
    }

    #[test]
    fn public_answers_pass() {
        for address in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
        ] {
            assert_eq!(classify(ip(address)), None, "{address}");
        }
    }

    #[test]
    fn guard_refuses_the_first_non_public_answer() {
        let addresses = [ip("93.184.216.34"), ip("10.1.2.3"), ip("8.8.8.8")];
        let refusal = guard_hosts(&addresses).expect_err("refused");
        assert_eq!(refusal.address, ip("10.1.2.3"));
        assert_eq!(refusal.class, "private");
        assert!(guard_hosts(&[ip("93.184.216.34"), ip("8.8.8.8")]).is_ok());
        assert!(guard_hosts(&[]).is_ok());
    }

    #[test]
    fn the_std_resolver_refuses_reserved_names_by_answer_or_error() {
        // localhost resolves to loopback on every normal host; if the
        // environment refuses it outright that is still a refusal, never an
        // allow.
        match StdResolver.resolve("localhost") {
            Ok(addresses) => assert!(guard_hosts(&addresses).is_err()),
            Err(detail) => assert!(!detail.is_empty()),
        }
    }
}
