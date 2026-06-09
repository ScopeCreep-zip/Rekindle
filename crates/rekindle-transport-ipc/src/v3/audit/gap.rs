//! Gap detection and missing-frame bitmap.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GapEvent {
    InOrder,
    GapDetected { start: u64, end: u64 },
    Duplicate { seq: u64 },
    GapTooLarge { start: u64, end: u64, size: u64 },
}

pub struct GapDetector {
    expected_next: u64,
    max_gap: u64,
}

impl Default for GapDetector {
    fn default() -> Self { Self::new() }
}

impl GapDetector {
    #[must_use]
    pub fn new() -> Self {
        Self { expected_next: 0, max_gap: 65536 }
    }

    #[must_use]
    pub fn with_max_gap(max_gap: u64) -> Self {
        Self { expected_next: 0, max_gap }
    }

    pub fn observe(&mut self, seq: u64) -> GapEvent {
        use std::cmp::Ordering;
        match seq.cmp(&self.expected_next) {
            Ordering::Equal => {
                self.expected_next = seq + 1;
                GapEvent::InOrder
            }
            Ordering::Less => GapEvent::Duplicate { seq },
            Ordering::Greater => {
                let gap_start = self.expected_next;
                let gap_end = seq - 1;
                let gap_size = seq - self.expected_next;
                self.expected_next = seq + 1;
                if gap_size > self.max_gap {
                    GapEvent::GapTooLarge { start: gap_start, end: gap_end, size: gap_size }
                } else {
                    GapEvent::GapDetected { start: gap_start, end: gap_end }
                }
            }
        }
    }

    #[must_use]
    pub fn expected_next(&self) -> u64 {
        self.expected_next
    }
}

pub struct MissingBitmapBuilder {
    start: u64,
    end: u64,
    bits: Vec<u8>,
}

impl MissingBitmapBuilder {
    #[must_use]
    pub fn new(start: u64, end: u64) -> Self {
        let range = if end >= start { end - start + 1 } else { 0 };
        let byte_count = usize::try_from(range).expect("bitmap range exceeds usize").div_ceil(8);
        Self { start, end, bits: vec![0u8; byte_count] }
    }

    pub fn mark_missing(&mut self, seq: u64) {
        if seq < self.start || seq > self.end {
            return;
        }
        let relative = usize::try_from(seq - self.start).expect("bitmap offset exceeds usize");
        let byte_idx = relative / 8;
        let bit_idx = relative % 8;
        if byte_idx < self.bits.len() {
            self.bits[byte_idx] |= 1 << bit_idx;
        }
    }

    #[must_use]
    pub fn build(&self) -> MissingBitmap {
        MissingBitmap {
            start: self.start,
            end: self.end,
            bits: self.bits.clone(),
        }
    }
}

pub struct MissingBitmap {
    start: u64,
    end: u64,
    bits: Vec<u8>,
}

impl MissingBitmap {
    pub fn from_bytes(start: u64, end: u64, bits: &[u8]) -> Self {
        Self { start, end, bits: bits.to_vec() }
    }

    #[must_use]
    pub fn is_missing(&self, seq: u64) -> bool {
        if seq < self.start || seq > self.end {
            return false;
        }
        let relative = usize::try_from(seq - self.start).expect("bitmap offset exceeds usize");
        let byte_idx = relative / 8;
        let bit_idx = relative % 8;
        byte_idx < self.bits.len() && (self.bits[byte_idx] & (1 << bit_idx)) != 0
    }

    #[must_use]
    pub fn missing_count(&self) -> u32 {
        self.bits.iter().map(|b| b.count_ones()).sum()
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bits
    }

    pub fn missing_seqs(&self) -> impl Iterator<Item = u64> + '_ {
        let start = self.start;
        self.bits.iter().enumerate().flat_map(move |(byte_idx, &byte)| {
            (0..8u64).filter_map(move |bit| {
                if byte & (1 << bit) != 0 {
                    Some(start + (byte_idx as u64) * 8 + bit)
                } else {
                    None
                }
            })
        })
    }
}
