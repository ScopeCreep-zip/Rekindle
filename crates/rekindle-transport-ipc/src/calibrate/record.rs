//! Measurement record — the wire schema for a single calibration data point.
//!
//! The identifier is the primary key. The record carries value, dispersion,
//! provenance, dependency evidence, and validation results. Serializable
//! to JSON Lines for streaming ingestion, structured for columnar export.

use std::collections::BTreeMap;

use super::validate::ValidationResult;

/// A single calibration measurement record.
#[derive(Debug, Clone)]
pub struct CalibrationPoint {
    /// Canonical OSC identifier — the primary key.
    pub id: String,
    /// Parsed quantity token (redundant with id, for columnar query).
    pub quantity: String,
    /// Parsed operation path (redundant with id, for columnar query).
    pub operation: String,
    /// Parsed conditions (redundant with id, for high-dimensionality features).
    pub conditions: BTreeMap<String, String>,
    /// Substrate triple.
    pub substrate: SubstrateRecord,
    /// Central estimate in the quantity's canonical unit (ns).
    /// After backfill: `typical()` = slope if present, else mean.
    pub value: f64,
    /// Unit string derived from quantity (single sanctioned redundancy).
    pub unit: String,
    /// Statistical dispersion.
    pub dispersion: Dispersion,
    /// Machine and environment provenance.
    pub provenance: Provenance,
    /// Canonical IDs of established dependencies.
    pub dependencies: Vec<String>,
    /// Validation rule results.
    pub validations: Vec<ValidationResultRecord>,
    /// Harness output coordinates for post-run backfill.
    /// (group_id, function_id, value_str) maps to the harness's
    /// output directory structure. For criterion:
    /// `target/criterion/{group}/{function}/{value}/new/estimates.json`
    /// None for derived/synthetic measurements.
    pub harness_coords: Option<(String, String, String)>,

    // ── Criterion estimates (populated by backfill) ──────────────────

    /// Slope point_estimate (ns) — present when criterion uses Linear/Auto
    /// sampling mode (regression slope = per-iteration cost). None for Flat.
    pub slope_ns: Option<f64>,
    /// Mean point_estimate (ns) — always present from criterion.
    pub mean_ns: Option<f64>,
    /// Median point_estimate (ns) — always present from criterion.
    pub median_ns: Option<f64>,
    /// Standard deviation (ns) — always present from criterion.
    pub std_dev_ns: Option<f64>,
    /// Median absolute deviation (ns) — always present from criterion.
    pub median_abs_dev_ns: Option<f64>,
    /// Standard error of the typical estimate (ns).
    pub standard_error_ns: Option<f64>,
    /// Sampling mode criterion used: "Linear" or "Flat".
    /// Determines whether slope or mean is the typical estimator.
    pub sampling_mode: Option<String>,

    // ── Run-over-run change detection (populated by backfill) ────────

    /// Fractional change in mean from baseline: `new_mean / base_mean - 1.0`.
    /// Negative = faster, positive = slower. None if no prior baseline exists.
    /// Source: `change/estimates.json → mean.point_estimate`.
    pub change_mean_estimate: Option<f64>,
    /// Fractional change in median from baseline: `new_median / base_median - 1.0`.
    /// Source: `change/estimates.json → median.point_estimate`.
    pub change_median_estimate: Option<f64>,

    // ── Raw sample data (populated by backfill, for plotters) ────────

    /// Raw iteration counts per sample point. Length = sample_size.
    /// Source: `sample.json → iters`.
    pub raw_iters: Option<Vec<f64>>,
    /// Raw elapsed times (ns) per sample point. Length = sample_size.
    /// Source: `sample.json → times`.
    pub raw_times: Option<Vec<f64>>,

    // ── Baseline comparison (populated by apply_baseline) ────────────

    /// The matching floor measurement from the baseline (nanoseconds).
    /// None if no baseline is loaded or no matching floor exists.
    pub baseline_floor_ns: Option<f64>,
    /// Ratio of measured value to floor: `value / baseline_floor_ns`.
    /// > 1.0 means slower than the irreducible floor.
    /// == 1.0 means at the physical limit.
    /// None if no baseline floor is available.
    pub ratio_to_floor: Option<f64>,
}

/// Statistical dispersion of the measurement.
#[derive(Debug, Clone)]
pub struct Dispersion {
    pub ci_lower: f64,
    pub ci_upper: f64,
    pub confidence: f64,
    pub samples: u32,
    pub estimator: String,
}

/// Machine and environment provenance.
#[derive(Debug, Clone)]
pub struct Provenance {
    pub timestamp_ns: u64,
    pub grammar_version: String,
    pub harness_version: String,
    pub cpu_model: String,
    pub cpu_microarch: String,
    pub cache_geometry: String,
    pub kernel: String,
    pub runtime_detail: String,
}

/// Substrate in the record (always populated, never optional).
#[derive(Debug, Clone)]
pub struct SubstrateRecord {
    pub arch: String,
    pub os: String,
    pub runtime: String,
}

/// Validation result for serialization (owned strings, no lifetimes).
#[derive(Debug, Clone)]
pub struct ValidationResultRecord {
    pub rule_id: String,
    pub predicate: String,
    pub passed: bool,
    pub severity: String,
    pub observed: f64,
    pub bound: f64,
    pub diagnosis: Option<String>,
}

impl From<&ValidationResult> for ValidationResultRecord {
    fn from(r: &ValidationResult) -> Self {
        Self {
            rule_id: r.rule_id.to_owned(),
            predicate: r.predicate.clone(),
            passed: r.passed,
            severity: format!("{:?}", r.severity),
            observed: r.observed,
            bound: r.bound,
            diagnosis: r.diagnosis.clone(),
        }
    }
}

impl CalibrationPoint {
    /// Whether all validation rules passed.
    pub fn is_trusted(&self) -> bool {
        self.validations.iter().all(|v| v.passed || v.severity == "Warn")
    }

    /// Serialize to a single JSON line (no trailing newline).
    /// Uses manual serialization to avoid serde dependency in the
    /// calibration module. The format is intentionally simple:
    /// one flat JSON object per line.
    pub fn to_jsonl(&self) -> String {
        let mut out = String::with_capacity(512);
        out.push('{');

        json_str(&mut out, "id", &self.id); out.push(',');
        json_str(&mut out, "quantity", &self.quantity); out.push(',');
        json_str(&mut out, "operation", &self.operation); out.push(',');
        json_num(&mut out, "value", self.value); out.push(',');
        json_str(&mut out, "unit", &self.unit); out.push(',');
        json_num(&mut out, "ci_lower", self.dispersion.ci_lower); out.push(',');
        json_num(&mut out, "ci_upper", self.dispersion.ci_upper); out.push(',');
        json_num(&mut out, "samples", self.dispersion.samples as f64); out.push(',');
        json_str(&mut out, "estimator", &self.dispersion.estimator); out.push(',');
        json_str(&mut out, "arch", &self.substrate.arch); out.push(',');
        json_str(&mut out, "os", &self.substrate.os); out.push(',');
        json_str(&mut out, "runtime", &self.substrate.runtime); out.push(',');
        json_str(&mut out, "cpu_model", &self.provenance.cpu_model); out.push(',');
        json_str(&mut out, "kernel", &self.provenance.kernel); out.push(',');
        json_num(&mut out, "timestamp_ns", self.provenance.timestamp_ns as f64); out.push(',');
        json_str(&mut out, "grammar_version", &self.provenance.grammar_version); out.push(',');
        json_bool(&mut out, "trusted", self.is_trusted());

        // Criterion detailed estimates
        if let Some(slope) = self.slope_ns {
            out.push(',');
            json_num(&mut out, "slope_ns", slope);
        }
        if let Some(mean) = self.mean_ns {
            out.push(',');
            json_num(&mut out, "mean_ns", mean);
        }
        if let Some(median) = self.median_ns {
            out.push(',');
            json_num(&mut out, "median_ns", median);
        }
        if let Some(sd) = self.std_dev_ns {
            out.push(',');
            json_num(&mut out, "std_dev_ns", sd);
        }
        if let Some(mad) = self.median_abs_dev_ns {
            out.push(',');
            json_num(&mut out, "median_abs_dev_ns", mad);
        }
        if let Some(se) = self.standard_error_ns {
            out.push(',');
            json_num(&mut out, "standard_error_ns", se);
        }
        if let Some(ref mode) = self.sampling_mode {
            out.push(',');
            json_str(&mut out, "sampling_mode", mode);
        }

        // Run-over-run change detection
        if let Some(change) = self.change_mean_estimate {
            out.push(',');
            json_num(&mut out, "change_mean_estimate", change);
        }
        if let Some(change) = self.change_median_estimate {
            out.push(',');
            json_num(&mut out, "change_median_estimate", change);
        }

        // Baseline comparison
        if let Some(floor) = self.baseline_floor_ns {
            out.push(',');
            json_num(&mut out, "baseline_floor_ns", floor);
        }
        if let Some(ratio) = self.ratio_to_floor {
            out.push(',');
            json_num(&mut out, "ratio_to_floor", ratio);
        }

        // Conditions as nested object
        out.push_str(",\"conditions\":{");
        let mut first = true;
        for (k, v) in &self.conditions {
            if !first { out.push(','); }
            json_str(&mut out, k, v);
            first = false;
        }
        out.push('}');

        // Validations as array
        out.push_str(",\"validations\":[");
        let mut first = true;
        for v in &self.validations {
            if !first { out.push(','); }
            out.push('{');
            json_str(&mut out, "rule", &v.rule_id); out.push(',');
            json_bool(&mut out, "passed", v.passed); out.push(',');
            json_str(&mut out, "severity", &v.severity);
            if let Some(ref d) = v.diagnosis {
                out.push(',');
                json_str(&mut out, "diagnosis", d);
            }
            out.push('}');
            first = false;
        }
        out.push(']');

        out.push('}');
        out
    }
}

fn json_str(out: &mut String, key: &str, value: &str) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":\"");
    // Escape backslashes and quotes in value
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
}

fn json_num(out: &mut String, key: &str, value: f64) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":");
    out.push_str(&format!("{value}"));
}

fn json_bool(out: &mut String, key: &str, value: bool) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":");
    out.push_str(if value { "true" } else { "false" });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_point() -> CalibrationPoint {
        CalibrationPoint {
            id: "osc:bw/mem.copy?cache=l1&size=16k@x86_64.linux.uring".to_owned(),
            quantity: "bw".to_owned(),
            operation: "mem.copy".to_owned(),
            conditions: {
                let mut m = BTreeMap::new();
                m.insert("cache".to_owned(), "l1".to_owned());
                m.insert("size".to_owned(), "16k".to_owned());
                m
            },
            substrate: SubstrateRecord {
                arch: "x86_64".to_owned(),
                os: "linux".to_owned(),
                runtime: "uring".to_owned(),
            },
            value: 226.48,
            unit: "bytes/sec".to_owned(),
            dispersion: Dispersion {
                ci_lower: 225.53,
                ci_upper: 227.25,
                confidence: 0.95,
                samples: 20,
                estimator: "criterion.mean".to_owned(),
            },
            provenance: Provenance {
                timestamp_ns: 1717430400_000_000_000,
                grammar_version: "0.1.0".to_owned(),
                harness_version: "0.1.0".to_owned(),
                cpu_model: "i5-9300H".to_owned(),
                cpu_microarch: "6".to_owned(),
                cache_geometry: "L1d=32k,L2=256k,L3=8m".to_owned(),
                kernel: "linux-6.18.7".to_owned(),
                runtime_detail: "io_uring kernel=6.18.7".to_owned(),
            },
            dependencies: vec![],
            validations: vec![],
            harness_coords: None,
            slope_ns: None,
            mean_ns: None,
            median_ns: None,
            std_dev_ns: None,
            median_abs_dev_ns: None,
            standard_error_ns: None,
            sampling_mode: None,
            change_mean_estimate: None,
            change_median_estimate: None,
            raw_iters: None,
            raw_times: None,
            baseline_floor_ns: None,
            ratio_to_floor: None,
        }
    }

    #[test]
    fn jsonl_is_valid_single_line() {
        let point = sample_point();
        let line = point.to_jsonl();
        assert!(!line.contains('\n'));
        assert!(line.starts_with('{'));
        assert!(line.ends_with('}'));
    }

    #[test]
    fn jsonl_contains_id() {
        let point = sample_point();
        let line = point.to_jsonl();
        assert!(line.contains("osc:bw/mem.copy?cache=l1&size=16k@x86_64.linux.uring"));
    }

    #[test]
    fn trusted_when_no_validations() {
        let point = sample_point();
        assert!(point.is_trusted());
    }

    #[test]
    fn untrusted_when_error_fails() {
        let mut point = sample_point();
        point.validations.push(ValidationResultRecord {
            rule_id: "test_rule".to_owned(),
            predicate: "a <= b".to_owned(),
            passed: false,
            severity: "Error".to_owned(),
            observed: 10.0,
            bound: 5.0,
            diagnosis: Some("test failure".to_owned()),
        });
        assert!(!point.is_trusted());
    }

    #[test]
    fn trusted_when_warn_fails() {
        let mut point = sample_point();
        point.validations.push(ValidationResultRecord {
            rule_id: "test_rule".to_owned(),
            predicate: "a <= b".to_owned(),
            passed: false,
            severity: "Warn".to_owned(),
            observed: 10.0,
            bound: 5.0,
            diagnosis: Some("test warning".to_owned()),
        });
        assert!(point.is_trusted());
    }
}
