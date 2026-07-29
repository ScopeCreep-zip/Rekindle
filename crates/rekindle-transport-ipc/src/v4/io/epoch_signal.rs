//! Epoch key installation signal — bounded queue from control loop to read task.
//!
//! Control loop (writer) → read task (reader). Bounded crossbeam channel
//! with capacity 4 — enough for 2 full rotation cycles (2 decoder installs
//! each). No signal is ever dropped. The read task drains all pending
//! installs at every frame boundary.
//!
//! Retirement is NOT a signal. Retirement is slot overwrite: when install(epoch=N)
//! is called, key_slots[N & 1] is overwritten, dropping the previous occupant
//! (epoch=N-2 keys). ZeroizeOnDrop fires on the dropped keys. No timer needed.

use super::encode::EpochKeys;

/// Payload for a key installation signal — carries BOTH decoder and encoder keys.
/// The read task installs both atomically before reading the next frame.
pub struct EpochInstallData {
    pub epoch: u8,
    pub decoder_keys: EpochKeys,
    pub encoder_keys: EpochKeys,
}

/// SPSC bounded channel for epoch key installation signals.
/// Capacity 4 — no signal is ever dropped. The read task drains
/// all pending installs at every frame boundary via `drain_all`.
pub struct EpochSignal {
    tx: crossbeam::channel::Sender<EpochInstallData>,
    rx: crossbeam::channel::Receiver<EpochInstallData>,
}

impl EpochSignal {
    pub fn new() -> Self {
        let (tx, rx) = crossbeam::channel::bounded(4);
        Self { tx, rx }
    }

    /// Send a key installation signal. Bounded channel — blocks if full
    /// (should never happen with capacity 4 and drain_all at every frame boundary).
    pub fn send_install(&self, epoch: u8, decoder_keys: EpochKeys, encoder_keys: EpochKeys) {
        let key_fingerprint = hex::encode(&decoder_keys.envelope_key[..8]);
        let data = EpochInstallData { epoch, decoder_keys, encoder_keys };
        match self.tx.try_send(data) {
            Ok(()) => {
                tracing::debug!(epoch, key_fp = %key_fingerprint, "EpochSignal::send_install — queued");
            }
            Err(crossbeam::channel::TrySendError::Full(data)) => {
                tracing::error!(
                    epoch = data.epoch,
                    key_fp = %key_fingerprint,
                    "EpochSignal::send_install — channel FULL (capacity 4), blocking"
                );
                // Block — the read task will drain. This should never happen
                // in practice because drain_all runs at every frame boundary.
                let _ = self.tx.send(data);
            }
            Err(crossbeam::channel::TrySendError::Disconnected(_)) => {
                tracing::error!(epoch, "EpochSignal::send_install — channel disconnected (read task exited)");
            }
        }
    }

    /// Drain one pending signal (for backward compatibility with existing callers).
    pub fn drain(&self) -> Option<EpochInstallData> {
        match self.rx.try_recv() {
            Ok(data) => {
                tracing::debug!(epoch = data.epoch, dec_key_fp = %hex::encode(&data.decoder_keys.envelope_key[..8]), enc_key_fp = %hex::encode(&data.encoder_keys.envelope_key[..8]), "EpochSignal::drain — signal consumed");
                Some(data)
            }
            Err(_) => None,
        }
    }
}
