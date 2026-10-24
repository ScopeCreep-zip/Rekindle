use super::*;

fn ip(text: &str) -> IpAddr {
    text.parse().expect("valid test IP literal")
}

#[test]
fn only_global_unicast_addresses_are_public() {
    let refused: &[(&str, &str)] = &[
        ("0.0.0.0", "unspecified"),
        ("0.1.2.3", "this network 0/8"),
        ("10.0.0.1", "private"),
        ("100.64.0.1", "CGNAT"),
        ("100.100.100.100", "Tailscale"),
        ("127.53.1.9", "loopback 127/8"),
        ("169.254.169.254", "cloud metadata"),
        ("172.16.5.5", "private"),
        ("192.0.0.9", "IETF protocol assignments"),
        ("192.0.2.1", "TEST-NET-1"),
        ("192.88.99.1", "6to4 relay anycast"),
        ("192.168.1.1", "private"),
        ("198.18.0.1", "benchmarking"),
        ("198.19.255.255", "benchmarking"),
        ("198.51.100.1", "TEST-NET-2"),
        ("203.0.113.1", "TEST-NET-3"),
        ("224.0.0.1", "multicast"),
        ("240.0.0.1", "reserved 240/4"),
        ("255.255.255.255", "broadcast"),
        ("::", "unspecified"),
        ("::1", "loopback"),
        ("::a00:1", "IPv4-compatible 10.0.0.1"),
        ("::ffff:169.254.169.254", "mapped metadata"),
        ("64:ff9b::a00:1", "NAT64 10.0.0.1"),
        ("64:ff9b::7f00:1", "NAT64 127.0.0.1"),
        ("64:ff9b:1:a00:0:100::", "local NAT64 /48 10.0.0.1"),
        ("64:ff9b:1::a00:1", "local NAT64 /96 10.0.0.1"),
        ("2002:a00:1::1", "6to4 10.0.0.1"),
        ("2002:7f00:1::", "6to4 127.0.0.1"),
        (
            "2001:0:4136:e378:8000:63bf:f5ff:fffe",
            "Teredo client 10.0.0.1",
        ),
        ("2001:2::1", "benchmarking 2001:2::/48"),
        ("2001:db8::1", "documentation"),
        ("3fff::1", "documentation 3fff::/20"),
        ("fc00::1", "unique local"),
        ("fd12:3456::1", "unique local"),
        ("fe80::1", "link-local"),
        ("fec0::1", "site-local"),
        ("ff02::1", "multicast"),
        ("100::1", "discard-only"),
    ];
    for (text, why) in refused {
        assert!(!is_public(&ip(text)), "{text} ({why}) must be refused");
    }
    let allowed: &[(&str, &str)] = &[
        ("93.184.216.34", "public"),
        ("8.8.8.8", "public"),
        ("100.63.255.255", "just below CGNAT"),
        ("100.128.0.0", "just above CGNAT"),
        ("198.17.255.255", "just below benchmarking"),
        ("223.255.255.255", "just below multicast"),
        ("2606:4700:4700::1111", "public"),
        ("2a00:1450:4001::1", "public"),
        ("::ffff:8.8.8.8", "mapped public"),
        ("64:ff9b::808:808", "NAT64 8.8.8.8"),
        ("2002:808:808::1", "6to4 8.8.8.8"),
        (
            "2001:0:808:808:0:0:f7f7:f7f7",
            "Teredo server and client 8.8.8.8",
        ),
    ];
    for (text, why) in allowed {
        assert!(is_public(&ip(text)), "{text} ({why}) must be allowed");
    }
}

#[test]
fn names_are_refused_per_label_and_by_suffix() {
    for (name, refused) in [
        ("intranet", true),
        ("intranet.", true),
        ("printer.local", true),
        ("a.b.internal", true),
        ("nas.home.arpa", true),
        ("x.alt", true),
        ("\u{0430}pple.com", true),
        ("pay\u{03c1}al.com", true),
        ("b\u{00fc}cher.de", false),
        (
            "\u{043f}\u{0440}\u{0438}\u{043c}\u{0435}\u{0440}.\u{0440}\u{0444}",
            false,
        ),
        ("\u{d55c}\u{ad6d}abc.kr", false),
        ("example.com", false),
        ("home.arpa.example.com", false),
    ] {
        assert_eq!(refused_domain(name), refused, "{name}");
    }
}
