//! Selective acknowledgement bitmaps.

pub struct SackBitmap {
    cumulative_through: u32,
    bitmap: Vec<u8>,
}

impl SackBitmap {
    #[must_use]
    pub fn new(cumulative_through: u32, bitmap: &[u8]) -> Self {
        Self {
            cumulative_through,
            bitmap: bitmap.to_vec(),
        }
    }

    pub fn acknowledged_chunks(&self) -> impl Iterator<Item = u32> + '_ {
        self.bitmap.iter().enumerate().flat_map(move |(byte_idx, &byte)| {
            let base = self.cumulative_through + 1
                + u32::try_from(byte_idx).expect("bitmap index exceeds u32") * 8;
            (0..8u32).filter_map(move |bit| {
                if byte & (1 << bit) != 0 {
                    Some(base + bit)
                } else {
                    None
                }
            })
        })
    }
}

pub struct SackBuilder {
    cumulative_through: u32,
    received: Vec<u32>,
}

impl SackBuilder {
    #[must_use]
    pub fn new(cumulative_through: u32) -> Self {
        Self {
            cumulative_through,
            received: Vec::new(),
        }
    }

    pub fn mark_received(&mut self, chunk_index: u32) {
        if chunk_index > self.cumulative_through {
            self.received.push(chunk_index);
        }
    }

    #[must_use]
    pub fn build(&self) -> (u32, Vec<u8>) {
        if self.received.is_empty() {
            return (self.cumulative_through, vec![]);
        }

        let max_chunk = *self.received.iter().max().unwrap();
        let relative_max = max_chunk - self.cumulative_through;
        let bitmap_len = (relative_max as usize).div_ceil(8);
        let mut bitmap = vec![0u8; bitmap_len];

        for &chunk in &self.received {
            let relative = chunk - self.cumulative_through - 1;
            let byte_idx = relative as usize / 8;
            let bit_idx = relative % 8;
            if byte_idx < bitmap.len() {
                bitmap[byte_idx] |= 1 << bit_idx;
            }
        }

        (self.cumulative_through, bitmap)
    }
}
