use super::{scrub, tag};

const B64_A: &str = "um7m8HxBluv6XceSaB3dK9Lq0ZpWtYv1NcRe4Gh7Jf2";
const B64_B: &str = "5JN8bi47CYSxohJ9YCbWzULVQmbtr7z9O_zVZmtdU8g";
const HEX64: &str = "3f1a9c0b7e2d4f6a8b1c3e5d7f9a0b2c4d6e8f0a1b3c5d7e9f1a2b4c6d8e0f1a";

fn assert_gone(out: &str, secret: &str) {
    assert!(!out.contains(secret), "`{secret}` survived in `{out}`");
}

#[test]
fn typed_keys_keep_kind_and_lose_value() {
    let line = format!("open record=VLD0:{B64_A} failed");
    let out = scrub(&line);
    assert_gone(&out, B64_A);
    assert!(out.contains(&format!("record=VLD0:{}", tag(B64_A))));
}

#[test]
fn encrypted_record_keys_are_one_identifier() {
    let line = format!("community_id=\"VLD0:{B64_A}:{B64_B}\"");
    let out = scrub(&line);
    assert_gone(&out, B64_A);
    assert_gone(&out, B64_B);
    assert!(out.contains(&format!("VLD0:{}", tag(&format!("{B64_A}:{B64_B}")))));
}

#[test]
fn bare_base64url_keys() {
    let out = scrub(&format!("BareRecordKey({B64_A}) sig={B64_A}{B64_B}")).into_owned();
    assert_gone(&out, B64_A);
    assert_gone(&out, B64_B);
}

#[test]
fn hex_identifiers() {
    let channel = "e3513a747e412077f0a1b2c3d4e5f6a7";
    let out = scrub(&format!(
        "friend={HEX64} channel_id=\"{channel}\" window=chat-{}",
        &HEX64[..16]
    ))
    .into_owned();
    assert_gone(&out, HEX64);
    assert_gone(&out, channel);
    assert_gone(&out, &HEX64[..16]);
    assert!(out.contains(&format!("friend={}", tag(HEX64))));
}

#[test]
fn uuids() {
    let id = "6f9619ff-8b86-d011-b42d-00cf4fc964ff";
    assert_gone(&scrub(&format!("file {id} done")), id);
}

#[test]
fn ip_addresses_but_not_ports_loopback_or_unspecified() {
    let line = "dial Direct:udp|[fe80::ce81:b1c:bd2c:69e]:5150 Direct:udp|[fd03:802f:11a8:9639:144f:918e:eaa2:618d]:5150 Direct:tcp|81.2.69.142:5150 bind [0.0.0.0, ::] lo 127.0.0.1 ::1";
    let out = scrub(line);
    for ip in [
        "fe80::ce81:b1c:bd2c:69e",
        "fd03:802f:11a8:9639:144f:918e:eaa2:618d",
        "81.2.69.142",
    ] {
        assert_gone(&out, ip);
    }
    assert!(out.contains(&format!("{}:5150", tag("81.2.69.142"))));
    assert!(out.contains("[0.0.0.0, ::]"));
    assert!(out.contains("127.0.0.1"));
    assert!(out.contains("::1"));
}

#[test]
fn ordinary_log_text_is_untouched() {
    let line = "2026-10-04T10:09:43.385923Z  INFO rekindle_voice::media_ready: media-ready transition ready=false start_ts: 1759572583123456 v1.2.3 lamport=42 src-tauri/src/setup.rs:120:5 SubscriptionEventGovernanceSegmentsChanged deadbeef";
    assert_eq!(scrub(line), line);
}

#[test]
fn tags_are_stable_within_a_process_and_distinct_per_identifier() {
    assert_eq!(tag(HEX64), tag(HEX64));
    assert_ne!(tag(HEX64), tag(B64_A));
    let a = scrub(&format!("a={HEX64}")).into_owned();
    let b = scrub(&format!("b={HEX64}")).into_owned();
    assert_eq!(&a[2..], &b[2..]);
}

#[test]
fn a_shared_key_tags_alike_across_scrubbers() {
    let key = "00112233445566778899aabbccddeeff00112233445566778899AABBCCDDEEFF";
    let one = super::key_from(Some(key)).unwrap();
    let two = super::key_from(Some(key)).unwrap();
    assert_eq!(super::tag_with(&one, HEX64), super::tag_with(&two, HEX64));
}

#[test]
fn without_a_shared_key_each_scrubber_draws_its_own() {
    let one = super::key_from(None).unwrap();
    let two = super::key_from(None).unwrap();
    assert_ne!(super::tag_with(&one, HEX64), super::tag_with(&two, HEX64));
}

#[test]
fn a_malformed_shared_key_is_refused() {
    assert!(super::key_from(Some("abc")).is_err());
    assert!(super::key_from(Some(&"g".repeat(64))).is_err());
}

#[test]
fn writer_scrubs_whole_events() {
    use std::io::Write as _;
    let mut sink = Vec::new();
    {
        let mut w = super::ScrubbingWriter(&mut sink);
        w.write_all(format!("peer={HEX64}\n").as_bytes()).unwrap();
    }
    let out = String::from_utf8(sink).unwrap();
    assert_gone(&out, HEX64);
    assert!(out.ends_with('\n'));
}
