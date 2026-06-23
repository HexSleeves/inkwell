//! SSRF address/host classifier (card T11, federation P3).
//!
//! Every outbound federation fetch — Webmention source verification AND
//! Webmention send — must pass through this classifier so the server can never
//! be coerced into reaching an internal address. The rules are deliberately
//! conservative: an http(s) scheme allowlist, and a rejection of every IP range
//! that could reach loopback/private/link-local/unique-local space or a cloud
//! metadata endpoint.
//!
//! This module is pure and network-free so the classifier can be unit-tested
//! directly (no DNS, no sockets): [`is_blocked_ip`] takes an already-resolved
//! [`IpAddr`], and [`validate_public_url`] parses + scheme-checks a URL string.
//! The DNS resolution and the actual fetch live in [`super::fetch`], which calls
//! into here on every hop.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use reqwest::Url;

/// Why a URL was rejected before (or during) an outbound fetch. All variants are
/// surfaced only in logs and best-effort flows — a blocked fetch is dropped, it
/// never 500s a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsrfError {
    /// The string did not parse as an absolute URL.
    InvalidUrl,
    /// The scheme was not `http` or `https`.
    DisallowedScheme(String),
    /// The URL had no host component (e.g. a `file:` or `data:` URL, or a bare
    /// path).
    MissingHost,
    /// The host resolved to (or literally was) a blocked address range.
    BlockedAddress(IpAddr),
    /// The host could not be resolved to any IP address.
    Unresolvable,
}

impl std::fmt::Display for SsrfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SsrfError::InvalidUrl => write!(f, "not a valid absolute URL"),
            SsrfError::DisallowedScheme(scheme) => {
                write!(f, "scheme \"{scheme}\" is not allowed (only http/https)")
            }
            SsrfError::MissingHost => write!(f, "URL has no host"),
            SsrfError::BlockedAddress(ip) => {
                write!(f, "host resolves to a blocked address ({ip})")
            }
            SsrfError::Unresolvable => write!(f, "host could not be resolved"),
        }
    }
}

impl std::error::Error for SsrfError {}

/// Whether an already-resolved IP address must be refused as an outbound fetch
/// target. Pure and total so the classifier is unit-testable without a network.
///
/// Blocked ranges (the SSRF deny-list):
///   - IPv4: loopback `127.0.0.0/8`, private `10/8` `172.16/12` `192.168/16`,
///     link-local `169.254/16` (which contains the cloud metadata address
///     `169.254.169.254`), the unspecified `0.0.0.0`, broadcast
///     `255.255.255.255`, "this host" `0.0.0.0/8`, shared CGNAT `100.64/10`,
///     and benchmarking `198.18/15`.
///   - IPv6: loopback `::1`, unspecified `::`, unique-local `fc00::/7`,
///     link-local `fe80::/10`, and any IPv4-mapped/compat address (re-checked
///     against the IPv4 rules so `::ffff:169.254.169.254` can't sneak through).
pub fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_ipv4(v4),
        IpAddr::V6(v6) => is_blocked_ipv6(v6),
    }
}

fn is_blocked_ipv4(ip: &Ipv4Addr) -> bool {
    // Catch-all stdlib classifiers first (loopback, private, link-local,
    // broadcast, documentation, unspecified), then the ranges std doesn't
    // expose on stable as their own predicates.
    if ip.is_loopback()        // 127.0.0.0/8
        || ip.is_private()     // 10/8, 172.16/12, 192.168/16
        || ip.is_link_local()  // 169.254/16 (incl. 169.254.169.254 metadata)
        || ip.is_broadcast()   // 255.255.255.255
        || ip.is_documentation()
        || ip.is_unspecified()
    // 0.0.0.0
    {
        return true;
    }
    let [a, b, ..] = ip.octets();
    // "This host on this network" 0.0.0.0/8 (RFC 1122) — not covered by
    // is_unspecified, which is only the single 0.0.0.0 address.
    if a == 0 {
        return true;
    }
    // Shared address space / CGNAT 100.64.0.0/10 (RFC 6598).
    if a == 100 && (64..=127).contains(&b) {
        return true;
    }
    // Benchmarking 198.18.0.0/15 (RFC 2544) — can route to internal test gear.
    if a == 198 && (b == 18 || b == 19) {
        return true;
    }
    false
}

fn is_blocked_ipv6(ip: &Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() {
        return true;
    }
    // Any address embedding an IPv4 address (IPv4-mapped `::ffff:a.b.c.d` or the
    // deprecated IPv4-compatible form) is re-checked against the IPv4 rules, so
    // a mapped private/metadata address can't bypass the v4 deny-list.
    if let Some(v4) = ip.to_ipv4() {
        return is_blocked_ipv4(&v4);
    }
    let segments = ip.segments();
    // Unique-local fc00::/7 (the first 7 bits are 1111110).
    if (segments[0] & 0xfe00) == 0xfc00 {
        return true;
    }
    // Link-local unicast fe80::/10.
    if (segments[0] & 0xffc0) == 0xfe80 {
        return true;
    }
    false
}

/// Parse `raw` into an absolute URL and enforce the scheme allowlist, returning
/// the parsed [`Url`] (with a guaranteed host) on success.
///
/// This is the syntactic half of the guard — it does NOT resolve DNS or check
/// the host's IP (that happens per-hop in [`super::fetch`], because DNS can
/// change and a redirect can point somewhere new). It rejects anything that
/// isn't `http`/`https`, anything without a host, and (defensively) a host that
/// is itself a literal blocked IP, so an obvious `http://127.0.0.1/` is refused
/// without a network round-trip.
pub fn validate_public_url(raw: &str) -> Result<Url, SsrfError> {
    let url = Url::parse(raw.trim()).map_err(|_| SsrfError::InvalidUrl)?;
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(SsrfError::DisallowedScheme(other.to_string())),
    }
    // `Url::host` classifies the host as a domain, an IPv4, or an IPv6 literal —
    // the IPv6 form already has its `[...]` brackets stripped (unlike
    // `host_str`), so a literal `http://[::1]/` is recognized as an IP here.
    let ip = match url.host() {
        Some(url::Host::Ipv4(v4)) => Some(IpAddr::V4(v4)),
        Some(url::Host::Ipv6(v6)) => Some(IpAddr::V6(v6)),
        Some(url::Host::Domain(domain)) if !domain.is_empty() => None,
        // No host, or an empty domain.
        _ => return Err(SsrfError::MissingHost),
    };
    // If the host is a literal IP, reject obvious internal targets up front
    // (defense in depth; the per-hop DNS check in `fetch` is authoritative).
    if let Some(ip) = ip
        && is_blocked_ip(&ip)
    {
        return Err(SsrfError::BlockedAddress(ip));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(s: &str) -> IpAddr {
        IpAddr::V4(s.parse().unwrap())
    }
    fn v6(s: &str) -> IpAddr {
        IpAddr::V6(s.parse().unwrap())
    }

    #[test]
    fn blocks_loopback_and_unspecified() {
        assert!(is_blocked_ip(&v4("127.0.0.1")));
        assert!(is_blocked_ip(&v4("127.255.255.254")));
        assert!(is_blocked_ip(&v4("0.0.0.0")));
        assert!(is_blocked_ip(&v6("::1")));
        assert!(is_blocked_ip(&v6("::")));
    }

    #[test]
    fn blocks_private_ipv4_ranges() {
        assert!(is_blocked_ip(&v4("10.0.0.1")));
        assert!(is_blocked_ip(&v4("10.255.255.255")));
        assert!(is_blocked_ip(&v4("172.16.0.1")));
        assert!(is_blocked_ip(&v4("172.31.255.255")));
        assert!(is_blocked_ip(&v4("192.168.0.1")));
        assert!(is_blocked_ip(&v4("192.168.255.255")));
    }

    #[test]
    fn does_not_block_172_addresses_outside_the_private_block() {
        // 172.15/16 and 172.32/16 are public — only 172.16/12 is private.
        assert!(!is_blocked_ip(&v4("172.15.0.1")));
        assert!(!is_blocked_ip(&v4("172.32.0.1")));
    }

    #[test]
    fn blocks_link_local_and_cloud_metadata() {
        assert!(is_blocked_ip(&v4("169.254.0.1")));
        // The AWS/GCP/Azure metadata endpoint.
        assert!(is_blocked_ip(&v4("169.254.169.254")));
        assert!(is_blocked_ip(&v6("fe80::1")));
        assert!(is_blocked_ip(&v6("febf::1")));
    }

    #[test]
    fn blocks_unique_local_ipv6() {
        assert!(is_blocked_ip(&v6("fc00::1")));
        assert!(is_blocked_ip(&v6("fd00::1")));
        assert!(is_blocked_ip(&v6("fdff:ffff::1")));
    }

    #[test]
    fn blocks_ipv4_mapped_and_compat_metadata() {
        // An IPv4-mapped metadata address must NOT slip past the v6 path.
        assert!(is_blocked_ip(&v6("::ffff:169.254.169.254")));
        assert!(is_blocked_ip(&v6("::ffff:127.0.0.1")));
        assert!(is_blocked_ip(&v6("::ffff:10.0.0.1")));
    }

    #[test]
    fn blocks_cgnat_and_benchmark_ranges() {
        assert!(is_blocked_ip(&v4("100.64.0.1")));
        assert!(is_blocked_ip(&v4("100.127.255.255")));
        assert!(is_blocked_ip(&v4("198.18.0.1")));
        assert!(is_blocked_ip(&v4("198.19.255.255")));
        // Just outside CGNAT and benchmarking are public.
        assert!(!is_blocked_ip(&v4("100.63.255.255")));
        assert!(!is_blocked_ip(&v4("100.128.0.1")));
        assert!(!is_blocked_ip(&v4("198.20.0.1")));
    }

    #[test]
    fn accepts_normal_public_addresses() {
        assert!(!is_blocked_ip(&v4("8.8.8.8")));
        assert!(!is_blocked_ip(&v4("1.1.1.1")));
        assert!(!is_blocked_ip(&v4("93.184.216.34"))); // example.com
        assert!(!is_blocked_ip(&v6("2606:4700:4700::1111"))); // cloudflare
        assert!(!is_blocked_ip(&v6("2001:4860:4860::8888"))); // google
    }

    #[test]
    fn validate_public_url_accepts_http_and_https() {
        assert!(validate_public_url("https://example.com/post").is_ok());
        assert!(validate_public_url("http://example.com/post").is_ok());
        assert!(validate_public_url("  https://example.com/post  ").is_ok());
    }

    #[test]
    fn validate_public_url_rejects_non_http_schemes() {
        assert!(matches!(
            validate_public_url("ftp://example.com"),
            Err(SsrfError::DisallowedScheme(_))
        ));
        assert!(matches!(
            validate_public_url("file:///etc/passwd"),
            Err(SsrfError::DisallowedScheme(_))
        ));
        assert!(matches!(
            validate_public_url("gopher://example.com"),
            Err(SsrfError::DisallowedScheme(_))
        ));
        // data: and javascript: are not absolute http(s) URLs.
        assert!(validate_public_url("data:text/plain,hi").is_err());
    }

    #[test]
    fn validate_public_url_rejects_literal_internal_hosts() {
        assert!(matches!(
            validate_public_url("http://127.0.0.1/x"),
            Err(SsrfError::BlockedAddress(_))
        ));
        assert!(matches!(
            validate_public_url("http://169.254.169.254/latest/meta-data/"),
            Err(SsrfError::BlockedAddress(_))
        ));
        assert!(matches!(
            validate_public_url("http://[::1]/x"),
            Err(SsrfError::BlockedAddress(_))
        ));
        assert!(matches!(
            validate_public_url("http://10.0.0.5/x"),
            Err(SsrfError::BlockedAddress(_))
        ));
    }

    #[test]
    fn validate_public_url_rejects_garbage_and_missing_host() {
        assert!(matches!(
            validate_public_url("not a url"),
            Err(SsrfError::InvalidUrl)
        ));
        assert!(validate_public_url("http://").is_err());
    }

    #[test]
    fn validate_public_url_accepts_public_literal_ip() {
        // A public literal IP is fine; only blocked ranges are refused up front.
        assert!(validate_public_url("https://8.8.8.8/").is_ok());
    }
}
