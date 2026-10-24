use super::*;

#[test]
fn extracts_basic_open_graph() {
    let html = r#"
        <html><head>
            <title>Page Title</title>
            <meta property="og:title" content="OG Title">
            <meta property="og:description" content="OG Description">
            <meta property="og:image" content="https://example.com/image.png">
            <meta property="og:site_name" content="Example Site">
        </head></html>
    "#;
    let m = extract_open_graph(html);
    assert_eq!(m.title.as_deref(), Some("OG Title"));
    assert_eq!(m.description.as_deref(), Some("OG Description"));
    assert_eq!(m.site_name.as_deref(), Some("Example Site"));
}

#[test]
fn falls_back_to_title_tag_and_meta_description() {
    let html = r#"
        <html><head>
            <title>Plain Title</title>
            <meta name="description" content="Plain description">
        </head></html>
    "#;
    let m = extract_open_graph(html);
    assert_eq!(m.title.as_deref(), Some("Plain Title"));
    assert_eq!(m.description.as_deref(), Some("Plain description"));
}

#[test]
fn handles_attribute_order_swapped() {
    let html = r#"<meta content="Swapped" property="og:title">"#;
    let m = extract_open_graph(html);
    assert_eq!(m.title.as_deref(), Some("Swapped"));
}

#[test]
fn handles_single_quotes() {
    let html = r"<meta property='og:title' content='Single'>";
    let m = extract_open_graph(html);
    assert_eq!(m.title.as_deref(), Some("Single"));
}

#[test]
fn decodes_basic_html_entities() {
    let html = r#"<meta property="og:title" content="Tom &amp; Jerry"#.to_owned() + r#"">"#;
    let m = extract_open_graph(&html);
    assert_eq!(m.title.as_deref(), Some("Tom & Jerry"));
}

#[test]
fn ssrf_guard_blocks_loopback_private_link_local_and_metadata() {
    let disallowed: &[&str] = &[
        "127.0.0.1",              // IPv4 loopback
        "127.53.1.9",             // IPv4 loopback, full 127.0.0.0/8 range
        "10.0.0.1",               // IPv4 private
        "172.16.5.5",             // IPv4 private
        "192.168.1.1",            // IPv4 private
        "169.254.169.254",        // cloud metadata endpoint (IPv4 link-local)
        "169.254.1.1",            // IPv4 link-local, general
        "0.0.0.0",                // IPv4 unspecified
        "224.0.0.1",              // IPv4 multicast
        "255.255.255.255",        // IPv4 broadcast
        "::1",                    // IPv6 loopback
        "::",                     // IPv6 unspecified
        "fc00::1",                // IPv6 unique-local
        "fe80::1",                // IPv6 link-local
        "ff02::1",                // IPv6 multicast
        "::ffff:169.254.169.254", // IPv4-mapped IPv6 metadata endpoint
    ];
    for ip in disallowed {
        let parsed: IpAddr = ip.parse().expect("valid test IP literal");
        assert!(
            is_disallowed_target(&parsed),
            "{ip} should be disallowed but was not"
        );
    }
}

#[test]
fn ssrf_guard_allows_ordinary_public_addresses() {
    let allowed: &[&str] = &[
        "93.184.216.34",        // example.com-class public IPv4
        "8.8.8.8",              // public IPv4
        "2606:4700:4700::1111", // public IPv6 (Cloudflare)
    ];
    for ip in allowed {
        let parsed: IpAddr = ip.parse().expect("valid test IP literal");
        assert!(
            !is_disallowed_target(&parsed),
            "{ip} should be allowed but was disallowed"
        );
    }
}

#[test]
fn the_user_agent_does_not_name_rekindle() {
    assert!(
        !USER_AGENT.to_ascii_lowercase().contains("rekindle"),
        "{USER_AGENT}"
    );
}

#[test]
fn fetchable_url_refuses_hidden_reserved_local_and_homograph_hosts() {
    for refused in [
        "https://exampleabcdefgh.onion/",
        "https://site.i2p/",
        "https://localhost/",
        "https://app.localhost/",
        "https://shop.test/",
        "https://a.invalid/",
        "https://www.example/",
        "https://127.0.0.1/admin",
        "https://169.254.169.254/latest/meta-data",
        "https://[::1]/",
        // Cyrillic \u{0430} in an otherwise Latin name.
        "https://\u{0430}pple.com/",
        "https://xn--pple-43d.com/",
        "http://example.com/",
    ] {
        assert!(fetchable_url(refused).is_err(), "{refused} must be refused");
    }
    for allowed in [
        "https://example.com/page",
        "https://news.ycombinator.com/",
        "https://\u{043f}\u{0440}\u{0438}\u{043c}\u{0435}\u{0440}.\u{0440}\u{0444}/",
        "https://93.184.216.34/",
    ] {
        assert!(fetchable_url(allowed).is_ok(), "{allowed} must be allowed");
    }
}

#[test]
fn every_redirect_hop_is_rechecked_and_capped() {
    let ok = Url::parse("https://example.com/next").unwrap();
    assert!(redirect_allowed(0, &ok).is_ok());
    assert!(redirect_allowed(4, &ok).is_ok());
    assert!(matches!(
        redirect_allowed(5, &ok),
        Err(LinkPreviewError::TooManyRedirects)
    ));
    for hop in [
        "http://example.com/next",
        "https://hidden.onion/",
        "https://localhost/",
        "https://shop.test/",
        "https://10.0.0.1/",
        "https://xn--pple-43d.com/",
    ] {
        let next = Url::parse(hop).unwrap();
        assert!(
            redirect_allowed(1, &next).is_err(),
            "redirect to {hop} must be refused"
        );
    }
}

#[test]
fn https_url_accepts_only_bounded_https() {
    assert!(https_url("https://example.com/a?b=c").is_ok());
    assert!(matches!(
        https_url("http://example.com"),
        Err(LinkPreviewError::BadScheme)
    ));
    assert!(matches!(
        https_url("javascript:alert(1)"),
        Err(LinkPreviewError::BadScheme)
    ));
    assert!(matches!(
        https_url("file:///etc/hosts"),
        Err(LinkPreviewError::BadScheme)
    ));
    assert!(matches!(
        https_url("not a url"),
        Err(LinkPreviewError::InvalidUrl(_))
    ));
    let long = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
    assert!(matches!(
        https_url(&long),
        Err(LinkPreviewError::TooLong { .. })
    ));
    assert_eq!(
        https_url("https://exаmple.com/").unwrap().host_str(),
        Some("xn--exmple-4nf.com"),
        "IDN hosts surface in punycode"
    );
}

#[test]
fn accept_inbound_normalizes_or_drops() {
    let preview = |url: &str| LinkPreview {
        message_id: "m".into(),
        url: url.into(),
        title: Some(format!("  Hi\u{202E}there\u{7} {}", "x".repeat(400))),
        description: Some("   ".into()),
        site_name: None,
        fetched_at: 1,
    };
    assert!(accept_inbound(preview("http://example.com")).is_none());
    assert!(accept_inbound(preview("javascript:alert(1)")).is_none());
    let ok = accept_inbound(preview("https://example.com/a")).unwrap();
    let title = ok.title.unwrap();
    assert!(title.starts_with("Hithere "));
    assert_eq!(title.chars().count(), MAX_TITLE_CHARS);
    assert_eq!(ok.description, None);
}
