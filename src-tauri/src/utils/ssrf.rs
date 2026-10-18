//! Shared SSRF defense: policy-aware DNS resolution + connection pinning (SEC-7).
//!
//! `url_validation` does static string checks only (its module doc records the
//! DNS-rebinding gap); this module closes that gap at runtime: resolve NOW, judge
//! every address under the caller's policy, and hand back a `(host, addr)` pair to
//! pin via `reqwest::ClientBuilder::resolve` — the connect then uses exactly the
//! validated IP and no second lookup can flip it to an internal target.
//!
//! Two policies, decided by who controls the URL:
//! - [`ConnectPolicy::PublicOnly`] — model-controlled egress (web_fetch /
//!   web_search): every resolved address must be public unicast, nothing local is
//!   ever legitimate. (web_fetch's historic `resolve_validated_addr` semantics,
//!   extracted verbatim.)
//! - [`ConnectPolicy::AllowLocalNoMetadata`] — user-configured endpoints (MCP
//!   HTTP transports, provider API URLs): loopback / LAN are legitimate (local
//!   engines, local MCP bridges run there), but cloud-metadata ranges are ALWAYS
//!   rejected — no legitimate user service lives on 169.254.169.254 — and the
//!   resolved address is pinned so a rebinding flip (benign at check time,
//!   internal at connect time) cannot redirect the key-bearing request.

use crate::ierr;

/// Who controls the URL decides how strict the resolved-address policy is.
pub enum ConnectPolicy {
    /// Model-controlled egress: every resolved address must be public unicast.
    PublicOnly,
    /// User-configured endpoints: local/LAN allowed, metadata ranges always
    /// rejected, resolution pinned.
    AllowLocalNoMetadata,
}

/// Outcome of a policy-aware resolution.
pub enum ResolveOutcome {
    /// The host is an IP literal and passed the policy — literals cannot rebind,
    /// so no pinning is needed.
    LiteralOk,
    /// Resolved and every address passed the policy — pin this pair.
    Pinned {
        host: String,
        addr: std::net::SocketAddr,
    },
    /// Policy violation — the caller MUST refuse the request.
    Rejected(String),
    /// DNS infrastructure failure (offline / resolver error / empty answer) —
    /// the caller MAY fall back to an unpinned client; the connect fails the
    /// same way anyway.
    DnsUnavailable(String),
}

/// SEC-03R: whether a resolved address is safe for PUBLIC-ONLY egress
/// (public unicast only). Historic web_fetch judgement, extracted verbatim.
pub fn is_public_addr(ip: std::net::IpAddr) -> bool {
    /// Shared IPv4 range check (also reused for IPv4 addresses embedded in IPv6 forms).
    fn is_public_v4(v4: std::net::Ipv4Addr) -> bool {
        let o = v4.octets();
        !(v4.is_private()
            || v4.is_loopback()
            || v4.is_link_local()
            || v4.is_broadcast()
            || v4.is_multicast()
            || v4.is_unspecified()
            || v4.is_documentation()
            // 0.0.0.0/8 ("this network") and carrier-grade NAT 100.64.0.0/10
            || o[0] == 0
            || (o[0] == 100 && (o[1] & 0xC0) == 64))
    }

    match ip {
        std::net::IpAddr::V4(v4) => is_public_v4(v4),
        std::net::IpAddr::V6(v6) => {
            // IPv4-mapped (::ffff:0:0/96) and IPv4-compatible (::a.b.c.d) forms embed a
            // v4 target: dual-stack sockets route connects on those addresses to the
            // embedded IPv4, so they MUST be judged by the embedded v4 (the historic
            // segment checks below see 0000:ffff... prefixes as "global" and let
            // ::ffff:127.0.0.1 / ::ffff:169.254.169.254 through — a full SSRF bypass
            // via a single AAAA record). `to_ipv4()` covers both mapped forms.
            if let Some(v4) = v6.to_ipv4() {
                return is_public_v4(v4);
            }
            let s = v6.segments();
            // 6to4 (2002::/16) embeds a v4 in bits 48..80: judge by the embedded v4.
            if s[0] == 0x2002 {
                let v4 = std::net::Ipv4Addr::new(
                    (s[1] >> 8) as u8,
                    s[1] as u8,
                    (s[2] >> 8) as u8,
                    s[2] as u8,
                );
                return is_public_v4(v4);
            }
            // NAT64 well-known prefix (64:ff9b::/96): 96 prefix bits = segments 0..5,
            // the embedded v4 occupies the LAST 32 bits = segments 6..7.
            if s[0] == 0x0064 && s[1] == 0xff9b && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0
            {
                let v4 = std::net::Ipv4Addr::new(
                    (s[6] >> 8) as u8,
                    s[6] as u8,
                    (s[7] >> 8) as u8,
                    s[7] as u8,
                );
                return is_public_v4(v4);
            }
            // Manual scope checks (Ipv6Addr::is_unicast_global is still unstable):
            // reject loopback ::1, unspecified ::, link-local fe80::/10,
            // unique-local fc00::/7, multicast ff00::/8; everything else counts as
            // global unicast.
            !(v6.is_loopback()
                || v6.is_unspecified()
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xff00) == 0xff00)
        }
    }
}

/// Whether the address sits in a cloud-metadata range that is never legitimate for
/// a user-configured endpoint, regardless of policy:
/// - IPv4 link-local 169.254.0.0/16 — AWS / GCP / Azure / OCI IMDS (169.254.169.254)
/// - IPv4 carrier-grade NAT 100.64.0.0/10 — Alibaba metadata (100.100.100.200);
///   a desktop app legitimately reaching a user service inside ISP CGNAT space is
///   not a real configuration, so the whole range is refused
/// - IPv6 link-local fe80::/10 (incl. any v6-mapped metadata forms via to_ipv4)
/// - the AWS IPv6 IMDS address fd00:ec2::254 (exact)
fn is_metadata_addr(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_link_local() || (o[0] == 100 && (o[1] & 0xC0) == 64)
        }
        std::net::IpAddr::V6(v6) => {
            // v4-mapped metadata forms (::ffff:169.254.169.254) judge by the embedded v4.
            if let Some(v4) = v6.to_ipv4() {
                let o = v4.octets();
                return v4.is_link_local() || (o[0] == 100 && (o[1] & 0xC0) == 64);
            }
            if v6.is_loopback() {
                return false; // ::1 is a legitimate local endpoint, not metadata
            }
            let s = v6.segments();
            (s[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
                // AWS IMDS IPv6 (fd00:ec2::254) — exact host
                || (s[0] == 0xfd00
                    && s[1] == 0x0ec2
                    && s[2] == 0
                    && s[3] == 0
                    && s[4] == 0
                    && s[5] == 0
                    && s[6] == 0
                    && s[7] == 0x0254)
        }
    }
}

/// Resolve the URL's host, judge every address under `policy`, and return the pin
/// pair for `ClientBuilder::resolve`. See the module doc for the policy semantics.
pub async fn resolve_pinned(parsed: &url::Url, policy: ConnectPolicy) -> ResolveOutcome {
    let Some(host_raw) = parsed.host_str() else {
        return ResolveOutcome::Rejected(ierr!("URL 缺少 host", "URL has no host"));
    };
    let host = host_raw
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let port = parsed
        .port()
        .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });

    // IP literal: cannot rebind, so no pinning is needed — but under the
    // user-configured policy it still must clear the metadata ranges.
    if let Ok(literal) = host.parse::<std::net::IpAddr>() {
        return match policy {
            ConnectPolicy::PublicOnly => {
                // web_fetch semantics preserved verbatim: literals were already
                // rejected by its static hostname check; nothing to pin here.
                ResolveOutcome::LiteralOk
            }
            ConnectPolicy::AllowLocalNoMetadata => {
                if is_metadata_addr(literal) {
                    ResolveOutcome::Rejected(format!(
                        "地址 {} 属于云元数据/保留段，已拒绝连接",
                        literal
                    ))
                } else {
                    ResolveOutcome::LiteralOk
                }
            }
        };
    }

    let addrs: Vec<std::net::SocketAddr> =
        match tokio::net::lookup_host((host.as_str(), port)).await {
            Ok(it) => it.collect(),
            Err(e) => {
                return ResolveOutcome::DnsUnavailable(format!("域名解析失败: {e}"));
            }
        };
    if addrs.is_empty() {
        return ResolveOutcome::DnsUnavailable("域名没有可用的解析记录".to_string());
    }
    let violated = |ip: std::net::IpAddr| match policy {
        ConnectPolicy::PublicOnly => !is_public_addr(ip),
        ConnectPolicy::AllowLocalNoMetadata => is_metadata_addr(ip),
    };
    if let Some(bad) = addrs.iter().find(|a| violated(a.ip())) {
        let label = match policy {
            ConnectPolicy::PublicOnly => "内网/保留地址",
            ConnectPolicy::AllowLocalNoMetadata => "云元数据/保留地址",
        };
        return ResolveOutcome::Rejected(format!("域名解析到{label}（{}），已拒绝访问", bad.ip()));
    }
    ResolveOutcome::Pinned {
        host,
        addr: addrs[0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v6(s: &str) -> std::net::Ipv6Addr {
        s.parse().unwrap()
    }

    #[test]
    fn public_addr_ranges() {
        use std::net::IpAddr;
        assert!(is_public_addr(IpAddr::V4("8.8.8.8".parse().unwrap())));
        assert!(is_public_addr(IpAddr::V4("1.1.1.1".parse().unwrap())));
        assert!(!is_public_addr(IpAddr::V4("10.0.0.1".parse().unwrap())));
        assert!(!is_public_addr(IpAddr::V4("192.168.1.1".parse().unwrap())));
        assert!(!is_public_addr(IpAddr::V4("172.16.0.1".parse().unwrap())));
        assert!(!is_public_addr(IpAddr::V4("127.0.0.1".parse().unwrap())));
        assert!(!is_public_addr(IpAddr::V4("0.0.0.0".parse().unwrap())));
        assert!(!is_public_addr(IpAddr::V4("169.254.1.1".parse().unwrap())));
        assert!(!is_public_addr(IpAddr::V4("100.64.0.1".parse().unwrap())));
        assert!(!is_public_addr(IpAddr::V4("224.0.0.1".parse().unwrap())));
        assert!(!is_public_addr(IpAddr::V6(v6("::1"))));
        assert!(!is_public_addr(IpAddr::V6(v6("::"))));
        assert!(!is_public_addr(IpAddr::V6(v6("fe80::1"))));
        assert!(!is_public_addr(IpAddr::V6(v6("fd00::1"))));
        assert!(is_public_addr(IpAddr::V6(v6("2606:4700::1111"))));
        // SEC-03R regression: IPv4-embedded forms must be judged by their embedded v4.
        // A rebinding attacker publishes ::ffff:127.0.0.1 (or the metadata IP) as an
        // AAAA record; dual-stack sockets route the connect to the embedded IPv4, so
        // these previously slipped through the segment checks as "global unicast".
        for mapped in [
            "::ffff:127.0.0.1",       // IPv4-mapped loopback
            "::ffff:169.254.169.254", // IPv4-mapped link-local / metadata
            "::ffff:10.0.0.1",        // IPv4-mapped private
            "::ffff:192.168.1.1",     // IPv4-mapped private
            "::127.0.0.1",            // IPv4-compatible loopback
            "::0.0.0.0",              // IPv4-compatible unspecified
            "2002:7f00:1::",          // 6to4 embedding 127.0.0.1
            "2002:a9fe:a9fe::",       // 6to4 embedding 169.254.169.254
            "64:ff9b::7f00:1",        // NAT64 embedding 127.0.0.1
            "64:ff9b::a9fe:a9fe",     // NAT64 embedding 169.254.169.254
        ] {
            assert!(
                !is_public_addr(IpAddr::V6(v6(mapped))),
                "{mapped} 应被拒绝（内嵌 IPv4 为私网/保留地址）"
            );
        }
        // Mapped/6to4/NAT64 forms embedding a PUBLIC v4 stay allowed.
        for allowed in [
            "::ffff:1.1.1.1",
            "::ffff:8.8.8.8",
            "2002:101:101::",   // 6to4 embedding 1.1.1.1
            "64:ff9b::101:101", // NAT64 embedding 1.1.1.1
        ] {
            assert!(
                is_public_addr(IpAddr::V6(v6(allowed))),
                "{allowed} 应被放行（内嵌 IPv4 为公网地址）"
            );
        }
    }

    #[test]
    fn metadata_ranges_never_legitimate() {
        use std::net::IpAddr;
        // v4: cloud IMDS + Alibaba CGNAT metadata
        for bad in [
            "169.254.169.254",
            "169.254.0.1",
            "100.100.100.200",
            "100.64.1.1",
        ] {
            assert!(
                is_metadata_addr(IpAddr::V4(bad.parse().unwrap())),
                "{bad} 属于元数据段，任何策略下都必须拒绝"
            );
        }
        // v6: link-local, v4-mapped metadata, AWS IMDS IPv6
        for bad in ["fe80::1", "::ffff:169.254.169.254", "fd00:ec2::254"] {
            assert!(
                is_metadata_addr(IpAddr::V6(v6(bad))),
                "{bad} 属于元数据段，任何策略下都必须拒绝"
            );
        }
        // …but local/LAN endpoints that users legitimately configure stay allowed.
        for ok in [
            "127.0.0.1",
            "192.168.1.10",
            "10.1.2.3",
            "172.17.0.5", // docker bridge
        ] {
            assert!(
                !is_metadata_addr(IpAddr::V4(ok.parse().unwrap())),
                "{ok} 是合法的用户本地端点，不得误伤"
            );
        }
        assert!(!is_metadata_addr(IpAddr::V6(v6("::1"))));
        assert!(!is_metadata_addr(IpAddr::V6(v6("fd12:3456::789a")))); // home-lab ULA
    }
}
