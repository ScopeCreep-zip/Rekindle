//! The run's trace: what each node saw, in order, as semantic events
//! (event kinds and their endpoints, never wire bytes: those carry AEAD
//! nonces from `OsRng`, plan C9.2). Two runs with one seed have one trace.

/// Ordered `(node, event)` entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trace(Vec<(usize, String)>);

impl Trace {
    pub(crate) fn push(&mut self, node: usize, event: String) {
        self.0.push((node, event));
    }

    /// The entries, in order.
    #[must_use]
    pub fn entries(&self) -> &[(usize, String)] {
        &self.0
    }

    /// The entries `node` saw, in order.
    pub fn of(&self, node: usize) -> impl Iterator<Item = &str> {
        self.0
            .iter()
            .filter(move |(n, _)| *n == node)
            .map(|(_, e)| e.as_str())
    }

    /// A digest of the whole trace, for comparing runs.
    #[must_use]
    pub fn hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        for (node, event) in &self.0 {
            hasher.update(&(*node as u64).to_le_bytes());
            hasher.update(&(event.len() as u64).to_le_bytes());
            hasher.update(event.as_bytes());
        }
        *hasher.finalize().as_bytes()
    }
}
