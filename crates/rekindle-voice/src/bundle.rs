//! Several media datagrams in one Veilid message (plan E4.3 T3).
//!
//! Every Veilid message costs ~1,660 B on the route whatever it carries
//! (`transport::egress::ROUTE_OVERHEAD_BYTES`), so a call's fixed demand is
//! set by how many messages it sends, not by its bitrate (calls 1–3:
//! voice alone ~720 kbps of overhead at 50 messages a second, against a
//! Mac → Pop route of 0.3–0.8 Mbps). Datagrams due at the same moment go
//! out together under one message: the coalescing QUIC does for frames in
//! a packet (RFC 9000 §12.2, RFC 9221 §4–5).
//!
//! Each inner datagram is unchanged: its own tag, its route sequence
//! number and its own signature, so arrivals, transport feedback and
//! verification see exactly what they would have seen apart. The bundle
//! adds no authentication of its own: Veilid's route AEAD covers the
//! message, and every inner datagram is verified as it is today.
//!
//! Layout: `BUNDLE_TAG || count u8 || (len u16 LE || datagram) × count`.

/// A bundle of media datagrams.
pub const BUNDLE_TAG: u8 = b'B';

/// Most datagrams one bundle carries.
pub const MAX_BUNDLE_DATAGRAMS: usize = 32;

/// Largest bundle: one 4 KiB video fragment with its envelope, plus voice
/// and control. Veilid splits a message into 1,272-byte datagrams per hop
/// with all-or-nothing reassembly (`veilid-tools` `assembly_buffer.rs`),
/// so a bundle stays near the video fragment size it is sized around
/// (`rekindle-video` `fragment.rs` `FRAGMENT_PAYLOAD_LIMIT`, 4 KiB) rather
/// than growing loss sensitivity.
pub const MAX_BUNDLE_BYTES: usize = 6 * 1024;

/// Bytes the bundle adds for `count` datagrams.
#[must_use]
pub const fn framing_bytes(count: usize) -> usize {
    2 + 2 * count
}

/// Whether `datagram` can join a bundle already `bundled_bytes` long (its
/// framing included) holding `count` datagrams.
#[must_use]
pub fn fits(bundled_bytes: usize, count: usize, datagram: usize) -> bool {
    count < MAX_BUNDLE_DATAGRAMS
        && u16::try_from(datagram).is_ok()
        && bundled_bytes + 2 + datagram <= MAX_BUNDLE_BYTES
}

/// One message carrying `datagrams`. A single datagram goes as itself.
#[must_use]
pub fn encode(datagrams: &[Vec<u8>]) -> Vec<u8> {
    if let [only] = datagrams {
        return only.clone();
    }
    let total: usize = datagrams.iter().map(Vec::len).sum();
    let mut out = Vec::with_capacity(framing_bytes(datagrams.len()) + total);
    out.push(BUNDLE_TAG);
    out.push(u8::try_from(datagrams.len()).unwrap_or(u8::MAX));
    for d in datagrams {
        out.extend_from_slice(&u16::try_from(d.len()).unwrap_or(u16::MAX).to_le_bytes());
        out.extend_from_slice(d);
    }
    out
}

/// The datagrams of a bundle; `None` when `message` is not a well-formed
/// bundle (a malformed bundle is dropped whole).
#[must_use]
pub fn decode(message: &[u8]) -> Option<Vec<&[u8]>> {
    let (&tag, rest) = message.split_first()?;
    if tag != BUNDLE_TAG || message.len() > MAX_BUNDLE_BYTES + framing_bytes(MAX_BUNDLE_DATAGRAMS) {
        return None;
    }
    let (&count, mut rest) = rest.split_first()?;
    let count = usize::from(count);
    if count == 0 || count > MAX_BUNDLE_DATAGRAMS {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let (len, tail) = rest.split_first_chunk::<2>()?;
        let len = usize::from(u16::from_le_bytes(*len));
        if len == 0 || len > tail.len() {
            return None;
        }
        let (datagram, tail) = tail.split_at(len);
        out.push(datagram);
        rest = tail;
    }
    rest.is_empty().then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let a = vec![b'V', 1, 2, 3, 4, 9];
        let b = vec![b'T'; 40];
        let msg = encode(&[a.clone(), b.clone()]);
        assert_eq!(msg[0], BUNDLE_TAG);
        assert_eq!(decode(&msg).unwrap(), vec![&a[..], &b[..]]);
        assert_eq!(msg.len(), framing_bytes(2) + a.len() + b.len());
    }

    #[test]
    fn one_datagram_goes_as_itself() {
        let a = vec![b'V', 1, 2, 3, 4];
        assert_eq!(encode(std::slice::from_ref(&a)), a);
    }

    #[test]
    fn malformed_bundles_are_dropped_whole() {
        let msg = encode(&[vec![b'V'; 10], vec![b'M'; 10]]);
        assert!(decode(&msg[..msg.len() - 1]).is_none(), "truncated");
        let mut extra = msg.clone();
        extra.push(0);
        assert!(decode(&extra).is_none(), "trailing bytes");
        assert!(decode(&[BUNDLE_TAG, 0]).is_none(), "empty");
        assert!(
            decode(&[BUNDLE_TAG, 1, 0, 0]).is_none(),
            "zero-length datagram"
        );
        assert!(decode(&[b'V', 1, 2]).is_none(), "not a bundle");
    }

    #[test]
    fn fits_respects_both_limits() {
        assert!(fits(framing_bytes(1) + 100, 1, 4_400));
        assert!(!fits(framing_bytes(1) + 2_000, 1, 4_400), "over 6 KiB");
        assert!(!fits(framing_bytes(0), MAX_BUNDLE_DATAGRAMS, 10), "count");
    }
}
