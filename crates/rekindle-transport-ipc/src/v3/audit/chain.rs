//! Audit chain — BLAKE3-keyed hash chain with per-frame Link computation.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorSource {
    Handshake,
    Rotation,
}

#[derive(Debug, Clone, Copy)]
pub struct AnchorRecord {
    pub value: [u8; 32],
    pub source: AnchorSource,
    pub start_session_seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkInput {
    pub session_seq: u64,
    pub envelope_hash: [u8; 32],
    pub header_hash: [u8; 32],
    pub ciphertext_hash: [u8; 32],
}

/// A per-direction audit chain.
pub struct AuditChain {
    key: [u8; 32],
    current: [u8; 32],
    anchor: AnchorRecord,
    length: u64,
    links: Vec<[u8; 32]>,
}

impl AuditChain {
    /// Create a new chain anchored to a handshake transcript hash.
    pub fn new(audit_key: [u8; 32], handshake_anchor: [u8; 32]) -> Self {
        Self {
            key: audit_key,
            current: handshake_anchor,
            anchor: AnchorRecord {
                value: handshake_anchor,
                source: AnchorSource::Handshake,
                start_session_seq: 0,
            },
            length: 0,
            links: Vec::new(),
        }
    }

    /// Create a chain anchored to a rotation link (post-rotation continuation).
    pub fn with_rotation_anchor(
        audit_key: [u8; 32],
        rotation_link: [u8; 32],
        start_session_seq: u64,
    ) -> Self {
        Self {
            key: audit_key,
            current: rotation_link,
            anchor: AnchorRecord {
                value: rotation_link,
                source: AnchorSource::Rotation,
                start_session_seq,
            },
            length: 0,
            links: Vec::new(),
        }
    }

    /// Advance the chain by one frame, returning the new Link.
    pub fn advance(&mut self, input: LinkInput) -> [u8; 32] {
        let link = compute_link(&self.key, &self.current, &input);
        self.current = link;
        self.length += 1;
        self.links.push(link);
        link
    }

    pub fn current_link(&self) -> [u8; 32] {
        self.current
    }

    pub fn length(&self) -> u64 {
        self.length
    }

    pub fn anchor_record(&self) -> AnchorRecord {
        self.anchor
    }

    /// All Links computed so far, in order.
    pub fn links(&self) -> &[[u8; 32]] {
        &self.links
    }
}

/// Compute a single chain link.
fn compute_link(key: &[u8; 32], previous: &[u8; 32], input: &LinkInput) -> [u8; 32] {
    let mut data = Vec::with_capacity(32 + 8 + 32 + 32 + 32);
    data.extend_from_slice(previous);
    data.extend_from_slice(&input.session_seq.to_be_bytes());
    data.extend_from_slice(&input.envelope_hash);
    data.extend_from_slice(&input.header_hash);
    data.extend_from_slice(&input.ciphertext_hash);

    *blake3::keyed_hash(key, &data).as_bytes()
}

/// Verify a chain by recomputing from anchor through all inputs,
/// comparing each recomputed Link against the expected Links
/// produced by the original chain.
///
/// If any input is tampered, the recomputed Link at that position
/// diverges from the expected Link and `LinkMismatch` is returned
/// identifying the exact position of the first divergence.
pub fn verify_chain(
    key: &[u8; 32],
    anchor: &[u8; 32],
    inputs: &[LinkInput],
    expected_links: &[[u8; 32]],
) -> Result<[u8; 32], ChainVerifyError> {
    if expected_links.len() != inputs.len() {
        return Err(ChainVerifyError::InputCountMismatch {
            inputs: inputs.len(),
            expected: expected_links.len(),
        });
    }

    let mut current = *anchor;
    for (i, input) in inputs.iter().enumerate() {
        let expected_seq = i as u64;
        if input.session_seq != expected_seq {
            return Err(ChainVerifyError::SequenceOutOfOrder {
                index: i,
                expected: expected_seq,
                found: input.session_seq,
            });
        }
        current = compute_link(key, &current, input);
        if current != expected_links[i] {
            return Err(ChainVerifyError::LinkDivergence { index: i });
        }
    }
    Ok(current)
}

#[derive(Debug)]
pub enum ChainVerifyError {
    SequenceOutOfOrder {
        index: usize,
        expected: u64,
        found: u64,
    },
    LinkDivergence {
        index: usize,
    },
    InputCountMismatch {
        inputs: usize,
        expected: usize,
    },
}
