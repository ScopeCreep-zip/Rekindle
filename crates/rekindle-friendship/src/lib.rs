#![forbid(unsafe_code)]
//! Three-tier inbox-scan coordinator for Rekindle friendship requests.
//!
//! Friend requests land in a per-peer DHT inbox. Without a coordinator,
//! the only way to notice an inbound request is to scan the inbox — but
//! scanning every second wastes the network, and scanning every minute
//! makes the UX feel laggy. This module exposes a single coordinator
//! that fans three independent triggers into one stream of scans:
//!
//! 1. **`watch_rx`** — a `tokio::sync::watch::Receiver<u64>` that fires
//!    the moment Veilid's DHT subscription on the inbox subkey reports a
//!    `ValueChanged` event. ~1 s end-to-end when the watch is healthy.
//! 2. **30-second poll** — a backstop for the watch dying silently
//!    (Veilid drops watches under route churn). Guarantees a request is
//!    seen within 30 s in the worst case.
//! 3. **Direct trigger** — `mpsc::Sender<()>` exposed via the Tauri
//!    command `friendship_scan_now` for the user-initiated case
//!    ("I'm waiting on a request, scan now").
//!
//! All three are debounced on a 500 ms coalesce window so a burst of
//! triggers (watch fires + user clicks scan + poll tick all within a
//! second) collapses to ONE scan.
//!
//! Plan reference: `/Users/kali/.claude/plans/memoized-dazzling-torvalds.md` § Phase 7.

pub mod coordinator;
pub mod veilid_scanner;
pub mod watch_trigger;

pub use coordinator::{InboxScanCoordinator, InboxScanner, ScanError};
pub use veilid_scanner::{VeilidInboxScanner, VeilidScannerDeps};
pub use watch_trigger::WatchTrigger;
