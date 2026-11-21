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

    /// Insert an entry at `index` (an entry pushed at the same instant as
    /// others, put in its canonical place among them).
    pub(crate) fn insert(&mut self, index: usize, node: usize, event: String) {
        self.0.insert(index, (node, event));
    }

    /// How many entries there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
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

/// A failing run names its seed and trace hash, so a failure seen on one
/// machine is replayed on another (C9.3: one seed, one trace). The lock is
/// only tried: a panic raised under it must not hang the unwind.
impl Drop for crate::Netsim {
    fn drop(&mut self) {
        use std::fmt::Write as _;
        use std::io::Write as _;
        if !std::thread::panicking() {
            return;
        }
        // Written to the process's stderr, past the test harness's capture,
        // so a failing run's seed shows in every runner's log.
        let mut stderr = std::io::stderr();
        let Some(inner) = self.net.inner.try_lock() else {
            let _ = writeln!(
                stderr,
                "netsim: failed at seed {} (trace locked)",
                self.seed
            );
            return;
        };
        let hash = inner.trace.hash().iter().fold(String::new(), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        });
        let _ = writeln!(
            stderr,
            "netsim: failed at seed {}, trace {} entries, hash {hash}",
            self.seed,
            inner.trace.len()
        );
    }
}
