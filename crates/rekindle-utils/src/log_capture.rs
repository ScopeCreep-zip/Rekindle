//! A log sink a test reads back: every byte a `tracing_subscriber::fmt`
//! writer formats lands in one shared buffer (the route-trace and the
//! per-request debug-line tests read their lines through it). Behind the
//! `test-support` feature, which only dev-dependencies enable.

use std::io;
use std::sync::Arc;

use parking_lot::Mutex;

/// A cloneable writer whose clones share one buffer: hand a clone to the
/// subscriber (`.with_writer(move || lines.clone())`), read [`Self::text`].
#[derive(Clone, Default)]
pub struct CapturedLines(Arc<Mutex<Vec<u8>>>);

impl CapturedLines {
    /// Everything written so far, as text.
    ///
    /// # Panics
    /// When the bytes are not UTF-8 (a `fmt` subscriber writes only UTF-8).
    #[must_use]
    pub fn text(&self) -> String {
        let bytes = self.0.lock();
        String::from_utf8(bytes.clone()).expect("a fmt subscriber writes UTF-8")
    }
}

impl io::Write for CapturedLines {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut bytes = self.0.lock();
        bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
