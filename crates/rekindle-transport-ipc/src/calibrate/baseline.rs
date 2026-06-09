//! Baseline loader — reads a calibration JSONL file and provides
//! floor lookups by OSC identifier or operation.
//!
//! The baseline is the output of `ipc:baseline` (`target/calibration.jsonl`).
//! Workload benches load it at startup to compute ratio-to-floor for
//! every measurement. If the file doesn't exist, the baseline is empty
//! and all ratio fields are None — workload benches still run.

use std::collections::BTreeMap;
use std::path::Path;

/// A loaded baseline — floor measurements indexed by canonical OSC id.
#[derive(Debug, Clone)]
pub struct Baseline {
    /// Canonical OSC id → typical value in nanoseconds.
    floors: BTreeMap<String, f64>,
}

impl Baseline {
    /// Load from a JSONL file. Returns an empty baseline if the file
    /// doesn't exist or can't be parsed — never fails.
    pub fn load(path: &Path) -> Self {
        let floors = match std::fs::read_to_string(path) {
            Ok(content) => parse_jsonl_floors(&content),
            Err(e) => {
                tracing::info!(
                    path = %path.display(),
                    error = %e,
                    "baseline: not found, ratios will be None"
                );
                BTreeMap::new()
            }
        };
        tracing::info!(floor_count = floors.len(), "baseline: loaded");
        Self { floors }
    }

    /// Create an empty baseline (no floors).
    pub fn empty() -> Self {
        Self { floors: BTreeMap::new() }
    }

    /// Look up the floor value for an exact OSC id match.
    /// Returns None if no floor measurement exists for this id.
    pub fn floor_ns(&self, canonical_osc: &str) -> Option<f64> {
        self.floors.get(canonical_osc).copied()
    }

    /// Look up the floor value by operation name. Returns the first
    /// floor whose operation matches, regardless of conditions.
    /// Used when a workload bench has different conditions than the
    /// floor bench but shares the same operation.
    pub fn floor_for_operation(&self, operation: &str) -> Option<f64> {
        self.floors.iter()
            .find(|(k, _)| {
                // Extract operation from canonical OSC id:
                // osc:<quantity>/<operation>[?conditions][@substrate]
                if let Some(rest) = k.strip_prefix("osc:") {
                    if let Some(slash) = rest.find('/') {
                        let after_slash = &rest[slash + 1..];
                        let op_end = after_slash.find('?')
                            .or_else(|| after_slash.find('@'))
                            .unwrap_or(after_slash.len());
                        let op = &after_slash[..op_end];
                        return op == operation;
                    }
                }
                false
            })
            .map(|(_, v)| *v)
    }

    /// Compute ratio of a measured value to its floor.
    /// Returns None if no matching floor exists.
    /// ratio > 1.0 means the measurement is slower than the floor.
    /// ratio == 1.0 means the measurement matches the floor exactly.
    pub fn ratio(&self, canonical_osc: &str, measured_ns: f64) -> Option<f64> {
        let floor = self.floor_ns(canonical_osc)?;
        if floor <= 0.0 { return None; }
        Some(measured_ns / floor)
    }

    /// Number of floor measurements loaded.
    pub fn len(&self) -> usize {
        self.floors.len()
    }

    /// Whether the baseline has any floors.
    pub fn is_empty(&self) -> bool {
        self.floors.is_empty()
    }

    /// All loaded floor entries.
    pub fn floors(&self) -> &BTreeMap<String, f64> {
        &self.floors
    }
}

/// Parse JSONL content into a map of canonical OSC id → value (ns).
/// Uses serde_json for reliable parsing. Each line is one JSON object
/// with at minimum `"id"` and `"value"` fields.
fn parse_jsonl_floors(content: &str) -> BTreeMap<String, f64> {
    let mut floors = BTreeMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(obj) => {
                if let (Some(id), Some(value)) = (
                    obj["id"].as_str(),
                    obj["value"].as_f64(),
                ) {
                    if value > 0.0 {
                        floors.insert(id.to_owned(), value);
                    }
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "baseline: skipped malformed JSONL line");
            }
        }
    }
    floors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_baseline() {
        let b = Baseline::empty();
        assert!(b.is_empty());
        assert_eq!(b.floor_ns("osc:bw/mem.copy?size=64k"), None);
        assert_eq!(b.ratio("osc:bw/mem.copy?size=64k", 100.0), None);
    }

    #[test]
    fn parse_jsonl() {
        let content = r#"{"id":"osc:bw/mem.copy?size=64k","value":1524.5}
{"id":"osc:lat/io.nop?ring=coop","value":634.0}
{"id":"osc:lat/sync.wake","value":0.0}
"#;
        let floors = parse_jsonl_floors(content);
        assert_eq!(floors.len(), 2); // sync.wake excluded (value=0)
        assert_eq!(floors["osc:bw/mem.copy?size=64k"], 1524.5);
        assert_eq!(floors["osc:lat/io.nop?ring=coop"], 634.0);
    }

    #[test]
    fn ratio_computation() {
        let mut b = Baseline::empty();
        b.floors.insert("osc:bw/mem.copy?size=64k".to_owned(), 1000.0);
        assert_eq!(b.ratio("osc:bw/mem.copy?size=64k", 2000.0), Some(2.0));
        assert_eq!(b.ratio("osc:bw/mem.copy?size=64k", 1000.0), Some(1.0));
        assert_eq!(b.ratio("osc:bw/mem.copy?size=1m", 1000.0), None);
    }

    #[test]
    fn floor_for_operation() {
        let mut b = Baseline::empty();
        b.floors.insert("osc:bw/mem.copy?size=64k@x86_64.linux.uring".to_owned(), 1524.5);
        assert_eq!(b.floor_for_operation("mem.copy"), Some(1524.5));
        assert_eq!(b.floor_for_operation("io.send"), None);
    }

    #[test]
    fn load_nonexistent_returns_empty() {
        let b = Baseline::load(Path::new("/nonexistent/path/calibration.jsonl"));
        assert!(b.is_empty());
    }
}
