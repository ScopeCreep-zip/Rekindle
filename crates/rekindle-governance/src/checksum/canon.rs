//! The canonical encoding's primitives (plan E3.2 D15, S7c.3): integers are
//! big-endian, a count or a length is a `u32`, a string is its length and
//! UTF-8 bytes, an optional value is a `0` byte or a `1` byte and the
//! value, a list keeps its order, and a map or set is a count and its rows
//! sorted by their encoded key bytes, so no `HashMap` or `HashSet` is ever
//! read in its own order.

use std::collections::{BTreeMap, HashMap, HashSet};

use rekindle_types::governance::OpHash;
use rekindle_types::id::{CategoryId, ChannelId, EventId, PseudonymKey, RoleId, ThreadId};

use crate::op::OpStamp;

/// A value with one canonical byte encoding.
pub(crate) trait Canon {
    fn put(&self, out: &mut Vec<u8>);
}

/// A shared leaf type (an event rule, an attachment offer, an onboarding
/// question …) as its serde JSON text: those types hold no maps, so their
/// JSON is one deterministic string, fields in declaration order.
pub(crate) struct Json<'a, T>(pub &'a T);

impl<T: serde::Serialize> Canon for Json<'_, T> {
    fn put(&self, out: &mut Vec<u8>) {
        let bytes = serde_json::to_vec(self.0)
            .expect("a governance leaf type holds no map keys, so it always encodes as JSON");
        put_len(out, bytes.len());
        out.extend_from_slice(&bytes);
    }
}

/// A count or a length as a big-endian `u32`.
pub(crate) fn put_len(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&u32::try_from(n).unwrap_or(u32::MAX).to_be_bytes());
}

/// A map section: its count, then each `key ‖ value` row in key-byte order.
pub(crate) fn put_map<'a, K, V, I>(out: &mut Vec<u8>, rows: I)
where
    K: Canon + 'a,
    V: Canon + 'a,
    I: IntoIterator<Item = (&'a K, &'a V)>,
{
    let mut encoded: Vec<(Vec<u8>, Vec<u8>)> = rows
        .into_iter()
        .map(|(k, v)| (bytes_of(k), bytes_of(v)))
        .collect();
    encoded.sort();
    put_len(out, encoded.len());
    for (k, v) in encoded {
        out.extend_from_slice(&k);
        out.extend_from_slice(&v);
    }
}

/// A set section: its count, then each element in encoded-byte order.
pub(crate) fn put_set<'a, T, I>(out: &mut Vec<u8>, items: I)
where
    T: Canon + 'a,
    I: IntoIterator<Item = &'a T>,
{
    let mut encoded: Vec<Vec<u8>> = items.into_iter().map(bytes_of).collect();
    encoded.sort();
    put_len(out, encoded.len());
    for e in encoded {
        out.extend_from_slice(&e);
    }
}

fn bytes_of<T: Canon + ?Sized>(v: &T) -> Vec<u8> {
    let mut out = Vec::new();
    v.put(&mut out);
    out
}

impl Canon for bool {
    fn put(&self, out: &mut Vec<u8>) {
        out.push(u8::from(*self));
    }
}

impl Canon for u32 {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_be_bytes());
    }
}

impl Canon for u64 {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_be_bytes());
    }
}

impl<const N: usize> Canon for [u8; N] {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self);
    }
}

impl Canon for String {
    fn put(&self, out: &mut Vec<u8>) {
        put_len(out, self.len());
        out.extend_from_slice(self.as_bytes());
    }
}

impl<T: Canon> Canon for Option<T> {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            None => out.push(0),
            Some(v) => {
                out.push(1);
                v.put(out);
            }
        }
    }
}

impl<T: Canon> Canon for Vec<T> {
    fn put(&self, out: &mut Vec<u8>) {
        put_len(out, self.len());
        for v in self {
            v.put(out);
        }
    }
}

impl<A: Canon, B: Canon> Canon for (A, B) {
    fn put(&self, out: &mut Vec<u8>) {
        self.0.put(out);
        self.1.put(out);
    }
}

impl<T: Canon> Canon for HashSet<T> {
    fn put(&self, out: &mut Vec<u8>) {
        put_set(out, self);
    }
}

impl<K: Canon, V: Canon> Canon for HashMap<K, V> {
    fn put(&self, out: &mut Vec<u8>) {
        put_map(out, self);
    }
}

impl<K: Canon, V: Canon> Canon for BTreeMap<K, V> {
    fn put(&self, out: &mut Vec<u8>) {
        put_map(out, self);
    }
}

impl<T: Canon> Canon for std::collections::BTreeSet<T> {
    fn put(&self, out: &mut Vec<u8>) {
        put_set(out, self);
    }
}

impl Canon for OpHash {
    fn put(&self, out: &mut Vec<u8>) {
        self.0.put(out);
    }
}

impl Canon for OpStamp {
    fn put(&self, out: &mut Vec<u8>) {
        self.lamport.put(out);
        self.hash.put(out);
    }
}

macro_rules! canon_id {
    ($($t:ty),*) => {$(
        impl Canon for $t {
            fn put(&self, out: &mut Vec<u8>) {
                self.0.put(out);
            }
        }
    )*};
}

canon_id!(
    PseudonymKey,
    ChannelId,
    RoleId,
    CategoryId,
    ThreadId,
    EventId
);
