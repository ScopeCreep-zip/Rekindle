//! Cap'n Proto packing (the encoding spec, "Packing") of a builder's
//! message into bytes, with no fallible sink.
//!
//! `capnp::serialize_packed::write_message` returns a `Result` even into a
//! `Vec`, whose sink never fails (`capnp-0.20.6/src/io.rs:142-147`; its doc:
//! "the only source of errors from this function are `write.write_all()`
//! calls"). It is reproduced here byte for byte over the builder's output
//! segments: `serialize::write_message` hands the packer the segment table
//! and then each segment in separate `write_all` calls, each packed on its
//! own (`serialize_packed.rs:304-437`), so no run crosses from one into the
//! next, which a reader refuses. `packing_tests.rs` checks equality with
//! capnp's own packer.

use super::wire_len;

/// Each byte's bit in a word's tag.
const TAG_BITS: [u8; 8] = [1, 2, 4, 8, 16, 32, 64, 128];

/// Serialize a Cap'n Proto builder into packed bytes.
pub(crate) fn pack(builder: &capnp::message::Builder<capnp::message::HeapAllocator>) -> Vec<u8> {
    let segments = builder.get_segments_for_output();
    let mut out = Vec::new();
    pack_words(&segment_table(&segments), &mut out);
    for segment in segments.iter() {
        pack_words(segment, &mut out);
    }
    out
}

/// The standard serialization's segment table: the segment count less one,
/// each segment's length in words, padded to a whole word. capnp counts
/// segments and their words in `u32`, so [`wire_len`] never saturates here.
fn segment_table(segments: &[&[u8]]) -> Vec<u8> {
    let mut table = Vec::new();
    table.extend_from_slice(&wire_len(segments.len().saturating_sub(1)).to_le_bytes());
    for segment in segments {
        table.extend_from_slice(&wire_len(segment.len() / 8).to_le_bytes());
    }
    if !table.len().is_multiple_of(8) {
        table.extend_from_slice(&[0; 4]);
    }
    table
}

/// Pack whole words (a Cap'n Proto segment is whole words): each word is
/// its tag and nonzero bytes; an all-zero word is followed by the count of
/// zero words after it, an all-nonzero word by the count of following
/// words with at most one zero byte and those words raw, each count at
/// most 255.
fn pack_words(mut rest: &[u8], out: &mut Vec<u8>) {
    while let Some((word, tail)) = rest.split_first_chunk::<8>() {
        rest = tail;
        let tag = word
            .iter()
            .zip(TAG_BITS)
            .filter(|(byte, _)| **byte != 0)
            .fold(0u8, |tag, (_, bit)| tag | bit);
        out.push(tag);
        out.extend(word.iter().copied().filter(|byte| *byte != 0));
        if tag == 0 {
            let (count, after) = take_run(rest, |w| *w == [0; 8], |_| {});
            out.push(count);
            rest = after;
        } else if tag == u8::MAX {
            let mut raw = Vec::new();
            let (count, after) = take_run(
                rest,
                |w| w.iter().filter(|byte| **byte == 0).nth(1).is_none(),
                |w| raw.extend_from_slice(w),
            );
            out.push(count);
            out.append(&mut raw);
            rest = after;
        }
    }
}

/// The words at the front of `rest` that `keep` accepts, at most 255 (a
/// run's count is one byte), each handed to `take`: their count and what
/// follows them.
fn take_run(
    mut rest: &[u8],
    keep: impl Fn(&[u8; 8]) -> bool,
    mut take: impl FnMut(&[u8; 8]),
) -> (u8, &[u8]) {
    let mut count = 0u8;
    while count < u8::MAX {
        match rest.split_first_chunk::<8>() {
            Some((word, tail)) if keep(word) => {
                take(word);
                rest = tail;
                // Below u8::MAX (the loop condition), so this never saturates.
                count = count.saturating_add(1);
            }
            _ => break,
        }
    }
    (count, rest)
}

#[cfg(test)]
#[path = "packing_tests.rs"]
mod tests;
