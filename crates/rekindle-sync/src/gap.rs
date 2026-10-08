//! Gap detection for record subkey sequence numbers.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceGap {
    pub subkey: usize,
    pub local: u64,
    pub network: u64,
}

#[derive(Debug, Clone, Default)]
pub struct GapDetector;

impl GapDetector {
    pub fn detect(local: &[u64], network: &[u64]) -> Vec<SequenceGap> {
        let max_len = local.len().max(network.len());
        let mut gaps = Vec::new();

        for subkey in 0..max_len {
            let local_seq = local.get(subkey).copied().unwrap_or(0);
            let network_seq = network.get(subkey).copied().unwrap_or(0);
            if network_seq > local_seq {
                gaps.push(SequenceGap {
                    subkey,
                    local: local_seq,
                    network: network_seq,
                });
            }
        }

        gaps
    }
}

#[cfg(test)]
mod tests {
    use super::{GapDetector, SequenceGap};

    #[test]
    fn detects_only_forward_gaps() {
        let gaps = GapDetector::detect(&[1, 5, 3], &[1, 7, 3, 2]);
        assert_eq!(
            gaps,
            vec![
                SequenceGap {
                    subkey: 1,
                    local: 5,
                    network: 7,
                },
                SequenceGap {
                    subkey: 3,
                    local: 0,
                    network: 2,
                },
            ]
        );
    }

    /// Three-path independence: SMPL write lands on DHT and inspect catches up
    /// without any gossip notification. Local sequences are stale (no gossip
    /// delivered), but inspect polling reveals network sequences advanced.
    #[test]
    fn smpl_write_detected_by_inspect_without_gossip() {
        // Scenario: 4 member subkeys. Gossip never delivered anything, so local
        // sequences are all 0. But two members wrote to DHT via SMPL Path 1.
        let local = [0, 0, 0, 0];
        let network = [3, 0, 5, 0]; // Members 0 and 2 wrote messages

        let gaps = GapDetector::detect(&local, &network);

        // Inspect (Path 3) detects both gaps — can fetch without gossip
        assert_eq!(gaps.len(), 2);
        assert_eq!(
            gaps[0],
            SequenceGap {
                subkey: 0,
                local: 0,
                network: 3
            }
        );
        assert_eq!(
            gaps[1],
            SequenceGap {
                subkey: 2,
                local: 0,
                network: 5
            }
        );
    }

    /// Equal sequences produce no gaps — system is converged.
    #[test]
    fn no_gaps_when_sequences_match() {
        let gaps = GapDetector::detect(&[5, 3, 7], &[5, 3, 7]);
        assert!(gaps.is_empty());
    }
}
