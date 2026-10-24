//! The Ogg container (RFC 3533) for one logical stream: pages written from
//! packets, and packets read back from pages with every CRC checked.

/// One packet with the granule position its page ends at, when it is the
/// last packet completed on that page.
pub(super) struct Packet {
    pub data: Vec<u8>,
    /// The granule position of the page this packet completes, if it is
    /// that page's last packet.
    pub page_granule: Option<u64>,
}

const CAPTURE: &[u8; 4] = b"OggS";
const HEADER_LEN: usize = 27;
const FLAG_CONTINUED: u8 = 0x01;
const FLAG_FIRST: u8 = 0x02;
const FLAG_LAST: u8 = 0x04;
/// Most lacing values one page carries (RFC 3533 §6).
const MAX_LACING_VALUES: usize = 255;
/// Largest packet read: 120 ms of 1275-byte Opus frames (RFC 6716 §3.4,
/// §3.2.5), bounding a packet continued across pages.
const MAX_PACKET_BYTES: usize = 1275 * 48;
/// The granule position meaning "no packet finishes on this page".
const NO_GRANULE: u64 = u64::MAX;

/// The Ogg CRC-32 lookup table: polynomial 0x04c11db7, MSB first.
const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i: u32 = 0;
    while i < 256 {
        let mut crc = i << 24;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04c1_1db7
            } else {
                crc << 1
            };
            bit += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
};

/// The Ogg CRC-32: polynomial 0x04c11db7, no reflection, initial value and
/// final XOR zero (RFC 3533 §6, the `CRC_checksum` field), a byte at a
/// time from [`CRC_TABLE`].
fn crc32(data: &[u8]) -> u32 {
    data.iter().fold(0u32, |crc, &byte| {
        (crc << 8) ^ CRC_TABLE[usize::from(u8::try_from(crc >> 24).unwrap_or(0) ^ byte)]
    })
}

/// The lacing values of one packet: 255s, then the rest (0 when the
/// length is a multiple of 255).
fn lacing(len: usize) -> Vec<u8> {
    let mut values = vec![255u8; len / 255];
    values.push(u8::try_from(len % 255).unwrap_or(0));
    values
}

/// Writes one logical stream's pages.
pub(super) struct OggWriter {
    out: Vec<u8>,
    serial: u32,
    sequence: u32,
}

impl OggWriter {
    pub(super) fn new(serial: u32) -> Self {
        Self {
            out: Vec::new(),
            serial,
            sequence: 0,
        }
    }

    /// Write `packets` (each fits one page's lacing) as one page ending at
    /// `granule`; `first` and `last` mark the stream's first and last page.
    pub(super) fn page(&mut self, packets: &[&[u8]], granule: u64, first: bool, last: bool) {
        let segments: Vec<u8> = packets.iter().flat_map(|p| lacing(p.len())).collect();
        debug_assert!(segments.len() <= MAX_LACING_VALUES);
        let start = self.out.len();
        self.out.extend_from_slice(CAPTURE);
        self.out.push(0);
        self.out
            .push(if first { FLAG_FIRST } else { 0 } | if last { FLAG_LAST } else { 0 });
        self.out.extend_from_slice(&granule.to_le_bytes());
        self.out.extend_from_slice(&self.serial.to_le_bytes());
        self.out.extend_from_slice(&self.sequence.to_le_bytes());
        self.out.extend_from_slice(&[0; 4]);
        self.out
            .push(u8::try_from(segments.len()).unwrap_or(u8::MAX));
        self.out.extend_from_slice(&segments);
        for packet in packets {
            self.out.extend_from_slice(packet);
        }
        let crc = crc32(&self.out[start..]);
        self.out[start + 22..start + 26].copy_from_slice(&crc.to_le_bytes());
        self.sequence += 1;
    }

    /// How many lacing values `packet` takes.
    pub(super) fn segments(packet: &[u8]) -> usize {
        packet.len() / 255 + 1
    }

    pub(super) const MAX_LACING_VALUES: usize = MAX_LACING_VALUES;

    pub(super) fn finish(self) -> Vec<u8> {
        self.out
    }
}

/// Read every packet of a single-stream Ogg file: one serial, page
/// sequence numbers in order, every CRC valid, the first page marked
/// first and the last marked last, no packet left open, refusing a stream of more than `max_packets`: the
/// count is checked as each page completes packets, so a stream of tiny
/// packets stops at the bound.
pub(super) fn read(bytes: &[u8], max_packets: usize) -> Result<Vec<Packet>, String> {
    let mut packets = Vec::new();
    let mut open: Vec<u8> = Vec::new();
    let mut at = 0usize;
    let mut serial = None;
    let mut sequence = 0u32;
    let mut ended = false;
    while at < bytes.len() {
        if ended {
            return Err("data after the last page".into());
        }
        let header = bytes
            .get(at..at + HEADER_LEN)
            .ok_or("truncated page header")?;
        if &header[..4] != CAPTURE || header[4] != 0 {
            return Err("not an Ogg page".into());
        }
        let flags = header[5];
        let granule = u64::from_le_bytes(header[6..14].try_into().map_err(|_| "granule")?);
        let page_serial = u32::from_le_bytes(header[14..18].try_into().map_err(|_| "serial")?);
        let page_sequence = u32::from_le_bytes(header[18..22].try_into().map_err(|_| "seq")?);
        let crc = u32::from_le_bytes(header[22..26].try_into().map_err(|_| "crc")?);
        let count = usize::from(header[26]);
        if *serial.get_or_insert(page_serial) != page_serial {
            return Err("more than one logical stream".into());
        }
        if page_sequence != sequence {
            return Err("pages out of sequence".into());
        }
        if (flags & FLAG_FIRST != 0) != (sequence == 0) {
            return Err("first-page flag misplaced".into());
        }
        if (flags & FLAG_CONTINUED != 0) == open.is_empty() {
            return Err("continuation flag disagrees with the open packet".into());
        }
        let lacing = bytes
            .get(at + HEADER_LEN..at + HEADER_LEN + count)
            .ok_or("truncated lacing")?;
        let body_len: usize = lacing.iter().map(|&v| usize::from(v)).sum();
        let end = at + HEADER_LEN + count + body_len;
        let page = bytes.get(at..end).ok_or("truncated page body")?;
        let mut zeroed = page.to_vec();
        zeroed[22..26].fill(0);
        if crc32(&zeroed) != crc {
            return Err("page CRC mismatch".into());
        }
        let mut body = &page[HEADER_LEN + count..];
        let mut completed = Vec::new();
        for &value in lacing {
            let (take, rest) = body.split_at(usize::from(value));
            open.extend_from_slice(take);
            body = rest;
            if open.len() > MAX_PACKET_BYTES {
                return Err("packet larger than any Opus packet".into());
            }
            if value < 255 {
                completed.push(std::mem::take(&mut open));
            }
        }
        let last = completed.len();
        if last > 0 && granule == NO_GRANULE {
            return Err("a page that completes a packet has no granule".into());
        }
        if packets.len() + last > max_packets {
            return Err(format!("more than {max_packets} packets"));
        }
        for (i, data) in completed.into_iter().enumerate() {
            let page_granule = (i + 1 == last).then_some(granule);
            packets.push(Packet { data, page_granule });
        }
        ended = flags & FLAG_LAST != 0;
        sequence += 1;
        at = end;
    }
    if !ended || !open.is_empty() {
        return Err("stream has no last page".into());
    }
    Ok(packets)
}
