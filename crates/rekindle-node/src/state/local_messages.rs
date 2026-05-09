//! Encrypted local message store for sent DM plaintext.
//!
//! Signal Protocol forward secrecy means we cannot decrypt our own
//! outbound messages from the DhtLog ciphertext — the ratchet keys
//! are consumed and discarded. This store persists sent message
//! plaintext locally so conversations survive daemon restarts.
//!
//! Architecture:
//! - One encrypted file per peer at `~/.local/state/rekindle/dm/{peer_short}.enc`
//! - Each entry is AES-256-GCM encrypted JSON (one per line, hex-encoded)
//! - Key derived from signing key via BLAKE3 keyed hash
//! - Append-only: new messages appended, tail(N) reads last N
//! - 0600 file permissions, parent dir 0700

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use aes_gcm::{Aes256Gcm, aead::{Aead, KeyInit}, Nonce};

/// A single stored sent message.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct StoredMessage {
    body: String,
    timestamp: u64,
    message_id: String,
}

/// Encrypted local store for sent DM plaintext.
pub struct LocalMessageStore {
    /// Derived AES-256 key for at-rest encryption.
    key: [u8; 32],
    /// Base directory: `~/.local/state/rekindle/dm/`
    base_dir: PathBuf,
    /// In-memory cache: peer_key → Vec<StoredMessage> (loaded lazily per peer).
    cache: HashMap<String, Vec<StoredMessage>>,
}

impl LocalMessageStore {
    /// Create a new store with a key derived from the signing key.
    pub fn new(signing_key_bytes: &[u8; 32], state_dir: &Path) -> Self {
        let key = *blake3::keyed_hash(signing_key_bytes, b"rekindle-dm-local-v1").as_bytes();

        let base_dir = state_dir.join("dm");
        let _ = std::fs::create_dir_all(&base_dir);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&base_dir, std::fs::Permissions::from_mode(0o700));
        }

        Self {
            key,
            base_dir,
            cache: HashMap::new(),
        }
    }

    /// Store a sent message. Appends to the per-peer file and updates the cache.
    pub fn store_sent(&mut self, peer_key: &str, body: &str, timestamp: u64, message_id: &str) {
        let msg = StoredMessage {
            body: body.to_string(),
            timestamp,
            message_id: message_id.to_string(),
        };

        // Append to file
        let path = self.peer_file(peer_key);
        if let Ok(json) = serde_json::to_string(&msg) {
            if let Ok(encrypted) = self.encrypt_line(json.as_bytes()) {
                let line = format!("{}\n", hex::encode(&encrypted));
                let _ = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .and_then(|mut f| {
                        use std::io::Write;
                        f.write_all(line.as_bytes())?;
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                        }
                        Ok(())
                    });
            }
        }

        // Update cache
        self.cache.entry(peer_key.to_string()).or_default().push(msg);
    }

    /// Read the last N sent messages for a peer.
    pub fn query_sent(&mut self, peer_key: &str, limit: usize) -> Vec<rekindle_types::display::DmMessageDisplay> {
        let messages = self.load_peer(peer_key);
        let start = messages.len().saturating_sub(limit);
        messages[start..].iter().map(|m| {
            rekindle_types::display::DmMessageDisplay {
                sender_key: String::new(), // filled by caller with our public key
                sender_name: "you".to_string(),
                body: m.body.clone(),
                timestamp: m.timestamp,
                is_self: true,
            }
        }).collect()
    }

    fn load_peer(&mut self, peer_key: &str) -> &Vec<StoredMessage> {
        if !self.cache.contains_key(peer_key) {
            let messages = self.read_file(peer_key);
            self.cache.insert(peer_key.to_string(), messages);
        }
        self.cache.get(peer_key).expect("just inserted")
    }

    fn read_file(&self, peer_key: &str) -> Vec<StoredMessage> {
        let path = self.peer_file(peer_key);
        let Ok(content) = std::fs::read_to_string(&path) else {
            return Vec::new();
        };

        content.lines()
            .filter(|line| !line.is_empty())
            .filter_map(|line| {
                let encrypted = hex::decode(line.trim()).ok()?;
                let plaintext = self.decrypt_line(&encrypted).ok()?;
                serde_json::from_slice::<StoredMessage>(&plaintext).ok()
            })
            .collect()
    }

    fn peer_file(&self, peer_key: &str) -> PathBuf {
        let short = &peer_key[..16.min(peer_key.len())];
        self.base_dir.join(format!("{short}.enc"))
    }

    fn encrypt_line(&self, plaintext: &[u8]) -> Result<Vec<u8>, aes_gcm::Error> {
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|_| aes_gcm::Error)?;
        let mut nonce_bytes = [0u8; 12];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher.encrypt(nonce, plaintext)?;
        let mut output = Vec::with_capacity(12 + ciphertext.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }

    fn decrypt_line(&self, encrypted: &[u8]) -> Result<Vec<u8>, aes_gcm::Error> {
        if encrypted.len() < 28 { // 12 nonce + 16 tag minimum
            return Err(aes_gcm::Error);
        }
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|_| aes_gcm::Error)?;
        let nonce = Nonce::from_slice(&encrypted[..12]);
        cipher.decrypt(nonce, &encrypted[12..])
    }
}

impl Drop for LocalMessageStore {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.key.zeroize();
    }
}
