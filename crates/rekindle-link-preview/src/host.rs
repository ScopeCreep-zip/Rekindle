//! Which hosts a link preview may reach (prod research batch 7 L3; review
//! lv21 M2, M3, L1, L3).
//!
//! Addresses are allowed, never denied: an address is fetched only if it is
//! global unicast per the IANA IPv4 and IPv6 special-purpose address
//! registries (RFC 6890 and its updates), after every IPv6 form that embeds
//! or translates to an IPv4 address has been unwrapped and the IPv4 rule
//! applied to what it carries. Names are refused when they are single-label
//! or under a TLD that never names a public host, and a label that mixes
//! scripts is refused as an IDN homograph (UTS #39 "highly restrictive").

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Top-level names a preview never fetches: hidden-service networks,
/// localhost, the RFC 2606 / RFC 6761 reserved names, mDNS `.local`
/// (RFC 6762), the private-use `.internal` (ICANN 2024), the common
/// home-router `.lan` and `.localdomain`, and `.alt` (RFC 9476).
const REFUSED_TLDS: [&str; 11] = [
    "onion",
    "i2p",
    "localhost",
    "test",
    "invalid",
    "example",
    "local",
    "internal",
    "lan",
    "localdomain",
    "alt",
];

/// Suffixes of more than one label that never name a public host: the
/// home network's `.home.arpa` (RFC 8375).
const REFUSED_SUFFIXES: [&str; 1] = ["home.arpa"];

/// IPv4 ranges that are not global unicast (IANA IPv4 special-purpose
/// registry, plus multicast and the reserved 240/4), as (network, prefix).
const NON_GLOBAL_V4: [([u8; 4], u8); 16] = [
    ([0, 0, 0, 0], 8),       // "this network" (RFC 791); 0/8 reaches local services
    ([10, 0, 0, 0], 8),      // private (RFC 1918)
    ([100, 64, 0, 0], 10),   // shared CGNAT space (RFC 6598), Tailscale's tailnet
    ([127, 0, 0, 0], 8),     // loopback
    ([169, 254, 0, 0], 16),  // link-local, cloud metadata 169.254.169.254
    ([172, 16, 0, 0], 12),   // private
    ([192, 0, 0, 0], 24),    // IETF protocol assignments (RFC 6890)
    ([192, 0, 2, 0], 24),    // TEST-NET-1
    ([192, 31, 196, 0], 24), // AS112-v4 (RFC 7535)
    ([192, 88, 99, 0], 24),  // deprecated 6to4 relay anycast (RFC 7526)
    ([192, 168, 0, 0], 16),  // private
    ([192, 175, 48, 0], 24), // direct delegation AS112 (RFC 7534)
    ([198, 18, 0, 0], 15),   // benchmarking (RFC 2544)
    ([198, 51, 100, 0], 24), // TEST-NET-2
    ([203, 0, 113, 0], 24),  // TEST-NET-3
    ([224, 0, 0, 0], 3),     // multicast 224/4 and reserved 240/4 (with broadcast)
];

/// IPv6 prefixes inside global unicast 2000::/3 that are not global
/// (IANA IPv6 special-purpose registry), as (network, prefix). Teredo and
/// 6to4 are unwrapped before this list is consulted.
const NON_GLOBAL_V6: [(u128, u8); 3] = [
    (0x2001_0000_0000_0000_0000_0000_0000_0000, 23), // IETF protocol assignments (RFC 2928)
    (0x2001_0db8_0000_0000_0000_0000_0000_0000, 32), // documentation (RFC 3849)
    (0x3fff_0000_0000_0000_0000_0000_0000_0000, 20), // documentation (RFC 9637)
];

fn in_v4(addr: Ipv4Addr, (net, prefix): ([u8; 4], u8)) -> bool {
    let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
    u32::from(addr) & mask == u32::from_be_bytes(net) & mask
}

fn in_v6(addr: Ipv6Addr, (net, prefix): (u128, u8)) -> bool {
    let mask = u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0);
    u128::from(addr) & mask == net & mask
}

/// Whether `v4` is global unicast.
pub(crate) fn is_global_v4(v4: Ipv4Addr) -> bool {
    !NON_GLOBAL_V4.iter().any(|&range| in_v4(v4, range))
}

/// The IPv4 addresses an IPv6 address carries, when it embeds or
/// translates to IPv4: mapped `::ffff:0:0/96`, compatible `::/96`, NAT64
/// `64:ff9b::/96` (RFC 6052) and the local-use `64:ff9b:1::/48` (RFC 8215,
/// read at both its /48 and /96 positions, since the operator picks the
/// length), 6to4 `2002::/16` (RFC 3056) and Teredo `2001::/32` (RFC 4380,
/// the server and the obfuscated client). `None` for any other address.
fn embedded_v4(v6: Ipv6Addr) -> Option<Vec<Ipv4Addr>> {
    let o = v6.octets();
    let last = Ipv4Addr::new(o[12], o[13], o[14], o[15]);
    let bits = u128::from(v6);
    if in_v6(v6, (0xffff_0000_0000, 96)) || in_v6(v6, (0, 96)) {
        return Some(vec![last]);
    }
    if in_v6(v6, (0x0064_ff9b_0000_0000_0000_0000_0000_0000, 96)) {
        return Some(vec![last]);
    }
    if in_v6(v6, (0x0064_ff9b_0001_0000_0000_0000_0000_0000, 48)) {
        return Some(vec![Ipv4Addr::new(o[6], o[7], o[9], o[10]), last]);
    }
    if in_v6(v6, (0x2002_0000_0000_0000_0000_0000_0000_0000, 16)) {
        return Some(vec![Ipv4Addr::new(o[2], o[3], o[4], o[5])]);
    }
    if in_v6(v6, (0x2001_0000_0000_0000_0000_0000_0000_0000, 32)) {
        let server = Ipv4Addr::new(o[4], o[5], o[6], o[7]);
        let client = Ipv4Addr::from(!u32::try_from(bits & 0xffff_ffff).unwrap_or(0));
        return Some(vec![server, client]);
    }
    None
}

/// Whether `ip` is an address a preview may connect to: global unicast,
/// and for an IPv6 form that carries IPv4, every IPv4 it carries global.
pub(crate) fn is_public(ip: &IpAddr) -> bool {
    match *ip {
        IpAddr::V4(v4) => is_global_v4(v4),
        IpAddr::V6(v6) => match embedded_v4(v6) {
            Some(carried) => carried.into_iter().all(is_global_v4),
            None => {
                in_v6(v6, (0x2000_0000_0000_0000_0000_0000_0000_0000, 3))
                    && !NON_GLOBAL_V6.iter().any(|&range| in_v6(v6, range))
            }
        },
    }
}

/// The script a character belongs to, for the mixed-script rule; `None`
/// for the characters every script shares (ASCII digits, the hyphen).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Script {
    Latin,
    Greek,
    Cyrillic,
    Han,
    Hiragana,
    Katakana,
    Hangul,
    Bopomofo,
    /// Any other script, told apart by its 128-code-point block.
    Other(u32),
}

fn script(c: char) -> Option<Script> {
    let cp = u32::from(c);
    Some(match cp {
        0x30..=0x39 | 0x2d => return None,
        0x41..=0x5a | 0x61..=0x7a | 0xc0..=0x24f | 0x1e00..=0x1eff => Script::Latin,
        0x370..=0x3ff | 0x1f00..=0x1fff => Script::Greek,
        0x400..=0x52f | 0x2de0..=0x2dff | 0xa640..=0xa69f => Script::Cyrillic,
        0x3040..=0x309f => Script::Hiragana,
        0x30a0..=0x30ff | 0x31f0..=0x31ff => Script::Katakana,
        0x1100..=0x11ff | 0x3130..=0x318f | 0xac00..=0xd7af => Script::Hangul,
        0x3100..=0x312f | 0x31a0..=0x31bf => Script::Bopomofo,
        0x3400..=0x4dbf | 0x4e00..=0x9fff | 0x20000..=0x3134f => Script::Han,
        _ => Script::Other(cp >> 7),
    })
}

/// Whether one label's scripts are allowed together: a single script, or
/// one of UTS #39's highly restrictive combinations (Latin with Han and
/// the Japanese kana, with Han and Bopomofo, or with Han and Hangul).
fn label_is_single_script(label: &str) -> bool {
    use Script::{Bopomofo, Han, Hangul, Hiragana, Katakana, Latin};
    let mut scripts: Vec<Script> = label.chars().filter_map(script).collect();
    scripts.sort_unstable();
    scripts.dedup();
    if scripts.len() <= 1 {
        return true;
    }
    let within = |allowed: &[Script]| scripts.iter().all(|s| allowed.contains(s));
    within(&[Latin, Han, Hiragana, Katakana])
        || within(&[Latin, Han, Bopomofo])
        || within(&[Latin, Han, Hangul])
}

/// Whether a domain name is never fetched: a single label, a refused TLD
/// or suffix, or a label (punycode decoded) that mixes scripts. A label in
/// one non-Latin script is fine under an ASCII TLD (`bücher.de`,
/// `пример.com`).
pub(crate) fn refused_domain(domain: &str) -> bool {
    let name = domain.trim_end_matches('.').to_ascii_lowercase();
    let Some((_, tld)) = name.rsplit_once('.') else {
        return true;
    };
    if REFUSED_TLDS.contains(&tld)
        || REFUSED_SUFFIXES
            .iter()
            .any(|suffix| name == *suffix || name.ends_with(&format!(".{suffix}")))
    {
        return true;
    }
    let unicode = url::quirks::domain_to_unicode(&name);
    !unicode.split('.').all(label_is_single_script)
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
