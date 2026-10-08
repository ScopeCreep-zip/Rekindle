//! Identifier scrubbing for everything Rekindle writes to a log.
//!
//! Log files outlive the session and sit unencrypted on disk, so a seized
//! or shared device would otherwise reveal who the user talks to, which
//! communities they are in, and the IP addresses Veilid dialled. Scrubbing
//! happens once, at the sink, on the fully formatted line — the same design
//! as Signal-Android's `Scrubber` (applied in `PersistentLogger.formatBody`
//! and to submitted debug logs). A sink-level pass covers every target,
//! including `veilid_core`/`veilid_api` lines Rekindle does not author, and
//! cannot be forgotten at a new call site.
//!
//! Each identifier becomes `<tag>`: the first 8 hex digits of a BLAKE3
//! keyed hash under a key drawn from the OS RNG once per process (Signal
//! uses an HMAC-SHA256 key the same way). Within one run the same
//! identifier always gets the same tag, so a log can still be followed;
//! across runs, and against the real key, tags are unlinkable.
//!
//! Recognised shapes (each verified against real output — see the tests
//! here and `rekindle-protocol/tests/log_scrub_veilid.rs`):
//! - Veilid typed keys `KIND:<base64url>` and record keys
//!   `KIND:<base64url>:<base64url>` → `KIND:<tag>`
//! - bare base64url keys (32/64-byte encodings) and long base64 blobs
//! - hyphenated UUIDs
//! - IPv4/IPv6 addresses (loopback and unspecified are kept); ports kept
//! - hex runs of 16+ digits containing a letter (identity keys,
//!   pseudonyms, channel/call ids, window-label prefixes)

use std::borrow::Cow;
use std::io::{self, Write as _};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;

use rand::RngCore;
use regex::{Captures, Regex};
use tracing_subscriber::fmt::MakeWriter;

fn process_key() -> &'static [u8; 32] {
    static KEY: OnceLock<[u8; 32]> = OnceLock::new();
    KEY.get_or_init(|| {
        let mut k = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut k);
        k
    })
}

/// The `<tag>` an identifier is replaced with in this process.
#[must_use]
pub fn tag(identifier: &str) -> String {
    let h = blake3::keyed_hash(process_key(), identifier.as_bytes());
    format!("<{}>", &h.to_hex()[..8])
}

struct Rules {
    typed: Regex,
    uuid: Regex,
    base64_std: Regex,
    base64_url: Regex,
    ipv6: Regex,
    ipv4: Regex,
    hex: Regex,
}

fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        let re = |p: &str| Regex::new(p).expect("static log-scrub pattern");
        Rules {
            typed: re(r"\b([A-Z][A-Z0-9]{3}):([A-Za-z0-9_-]{20,}(?::[A-Za-z0-9_-]{20,})?)"),
            uuid: re(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b"),
            base64_std: re(r"[A-Za-z0-9+/]{60,}={0,2}"),
            base64_url: re(r"[A-Za-z0-9_-]{40,}"),
            ipv6: re(r"[0-9A-Fa-f:.]*:[0-9A-Fa-f:.]*:[0-9A-Fa-f:.]*"),
            ipv4: re(r"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}"),
            hex: re(r"[0-9A-Fa-f]{16,}"),
        }
    })
}

fn char_before(s: &str, idx: usize) -> Option<char> {
    s[..idx].chars().next_back()
}

fn char_after(s: &str, idx: usize) -> Option<char> {
    s[idx..].chars().next()
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn classes(s: &str) -> (bool, bool, bool) {
    (
        s.bytes().any(|b| b.is_ascii_uppercase()),
        s.bytes().any(|b| b.is_ascii_lowercase()),
        s.bytes().any(|b| b.is_ascii_digit()),
    )
}

fn replace<'a>(
    input: Cow<'a, str>,
    re: &Regex,
    mut f: impl FnMut(&str, &Captures<'_>) -> Option<String>,
) -> Cow<'a, str> {
    let text: &str = &input;
    let mut out: Option<String> = None;
    let mut last = 0;
    for caps in re.captures_iter(text) {
        let m = caps.get(0).expect("group 0 always present");
        if let Some(rep) = f(text, &caps) {
            let buf = out.get_or_insert_with(|| String::with_capacity(text.len()));
            buf.push_str(&text[last..m.start()]);
            buf.push_str(&rep);
            last = m.end();
        }
    }
    match out {
        None => input,
        Some(mut buf) => {
            buf.push_str(&text[last..]);
            Cow::Owned(buf)
        }
    }
}

fn keep_address(ip: IpAddr) -> bool {
    ip.is_loopback() || ip.is_unspecified()
}

/// Replace every identifier in `line` with its per-process tag.
#[must_use]
pub fn scrub(line: &str) -> Cow<'_, str> {
    let r = rules();
    let mut s = Cow::Borrowed(line);

    s = replace(s, &r.typed, |_, c| {
        Some(format!("{}:{}", &c[1], tag(&c[2])))
    });
    s = replace(s, &r.uuid, |_, c| Some(tag(&c[0])));
    s = replace(s, &r.base64_std, |_, c| {
        let (upper, lower, digit) = classes(&c[0]);
        (upper && lower && digit).then(|| tag(&c[0]))
    });
    s = replace(s, &r.base64_url, |_, c| {
        let m = &c[0];
        let (upper, lower, digit) = classes(m);
        // 43 / 86 chars = unpadded base64url of 32 / 64 bytes (Veilid
        // bare keys and signatures, e.g. inside `PublicKey(…)` Debug).
        (upper && lower && (digit || m.len() == 43 || m.len() == 86)).then(|| tag(m))
    });
    s = replace(s, &r.ipv6, |text, c| {
        let m = c.get(0).expect("group 0");
        if char_before(text, m.start()).is_some_and(is_word)
            || char_after(text, m.end()).is_some_and(is_word)
        {
            return None;
        }
        let cand = m.as_str().trim_end_matches('.');
        let ip: Ipv6Addr = cand.parse().ok()?;
        if keep_address(IpAddr::V6(ip)) {
            return None;
        }
        Some(format!("{}{}", tag(cand), &m.as_str()[cand.len()..]))
    });
    s = replace(s, &r.ipv4, |text, c| {
        let m = c.get(0).expect("group 0");
        if char_before(text, m.start()).is_some_and(|ch| is_word(ch) || ch == '.')
            || char_after(text, m.end()).is_some_and(|ch| ch.is_ascii_digit())
        {
            return None;
        }
        let ip: Ipv4Addr = m.as_str().parse().ok()?;
        (!keep_address(IpAddr::V4(ip))).then(|| tag(m.as_str()))
    });
    s = replace(s, &r.hex, |text, c| {
        let m = c.get(0).expect("group 0");
        if char_before(text, m.start()).is_some_and(|ch| ch.is_ascii_alphanumeric())
            || char_after(text, m.end()).is_some_and(|ch| ch.is_ascii_alphanumeric())
        {
            return None;
        }
        // A run with no a–f is a decimal number (timestamps, counters).
        m.as_str()
            .bytes()
            .any(|b| b.is_ascii_alphabetic())
            .then(|| tag(m.as_str()))
    });
    s
}

/// `MakeWriter` wrapper that scrubs each formatted event before it reaches
/// the inner writer. `tracing-subscriber`'s fmt layer formats a whole event
/// into one buffer and hands it to a single `write_all`, so identifiers are
/// never split across calls.
#[derive(Debug, Clone)]
pub struct ScrubbingMakeWriter<M>(M);

impl<M> ScrubbingMakeWriter<M> {
    pub fn new(inner: M) -> Self {
        Self(inner)
    }
}

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for ScrubbingMakeWriter<M> {
    type Writer = ScrubbingWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        ScrubbingWriter(self.0.make_writer())
    }

    fn make_writer_for(&'a self, meta: &tracing::Metadata<'_>) -> Self::Writer {
        ScrubbingWriter(self.0.make_writer_for(meta))
    }
}

/// Writer produced by [`ScrubbingMakeWriter`].
#[derive(Debug)]
pub struct ScrubbingWriter<W>(W);

impl<W: io::Write> io::Write for ScrubbingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        self.0.write_all(scrub(&text).as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// Default panic rendering: what std's hook prints, plus the backtrace
/// when `RUST_BACKTRACE` enables capture.
fn render_panic(info: &std::panic::PanicHookInfo<'_>) -> String {
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("<unnamed>");
    let backtrace = std::backtrace::Backtrace::capture();
    match backtrace.status() {
        std::backtrace::BacktraceStatus::Captured => {
            format!("thread '{name}' {info}\nstack backtrace:\n{backtrace}")
        }
        _ => format!("thread '{name}' {info}"),
    }
}

/// Replace the panic hook with one that writes the scrubbed panic report
/// to stderr. Panic payloads routinely carry the value that failed to
/// parse — a key, a record id.
pub fn install_panic_hook() {
    install_panic_hook_with(render_panic);
}

/// As [`install_panic_hook`], with the report produced by `render` — for
/// binaries that already have a panic formatter (the CLI's color-eyre),
/// so they keep their formatting and still go through the scrubber.
pub fn install_panic_hook_with(
    render: impl Fn(&std::panic::PanicHookInfo<'_>) -> String + Send + Sync + 'static,
) {
    std::panic::set_hook(Box::new(move |info| {
        // Write straight to stderr like std's own hook; a failed write
        // inside a panic hook has nowhere left to be reported.
        let _ = writeln!(io::stderr().lock(), "{}", scrub(&render(info)));
    }));
}

#[cfg(test)]
mod tests {
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
}
