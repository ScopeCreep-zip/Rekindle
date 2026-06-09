//! Calibration session — dependency-gated, validated measurement tracking.
//!
//! `CalibratedSession` tracks established measurements across a benchmark
//! run. The bench file calls its measurement harness (criterion, divan, etc.)
//! directly for execution; the session provides dependency checking (L3),
//! validation (L4), and record emission (L5).
//!
//! The session has no dependency on any benchmark harness. It operates on
//! measured values after the harness produces them.

use std::collections::BTreeMap;

use super::dependency::DependencyGraph;
use super::grammar::OscId;
use super::platform::PlatformInfo;
use super::profile::Profile;
use super::record::{
    CalibrationPoint, Dispersion, Provenance, SubstrateRecord, ValidationResultRecord,
};
use super::substrate::Substrate;
use super::validate;

/// Grammar version — the single source of truth for in-band version.
pub const GRAMMAR_VERSION: &str = "0.1.0";

/// An established measurement — value + dispersion + validation status.
#[derive(Debug, Clone)]
pub struct Established {
    pub value: f64,
    pub ci_lower: f64,
    pub ci_upper: f64,
    pub trusted: bool,
}

/// A skip diagnosis — returned when a measurement's dependencies are not satisfied.
#[derive(Debug, Clone)]
pub struct SkipDiagnosis {
    pub id: String,
    pub missing: Vec<String>,
}

impl std::fmt::Display for SkipDiagnosis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SKIPPED: {}\n  unsatisfied dependencies:", self.id)?;
        for m in &self.missing {
            write!(f, "\n    - {m}")?;
        }
        Ok(())
    }
}

/// Tracks established measurements for dependency checking and
/// validation across a calibration run's lifetime.
pub struct CalibratedSession {
    profile: Profile,
    dep_graph: DependencyGraph,
    established: BTreeMap<String, Established>,
    records: Vec<CalibrationPoint>,
    substrate: Substrate,
    platform: PlatformInfo,
}

impl CalibratedSession {
    /// Create a new calibrated session at the given conformance profile.
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            dep_graph: DependencyGraph::seeded(),
            established: BTreeMap::new(),
            records: Vec::new(),
            substrate: Substrate::auto_detect(),
            platform: PlatformInfo::detect(),
        }
    }

    /// Check whether an operation's dependencies are all established.
    /// Returns `Ok(())` if all deps are satisfied, or `Err(SkipDiagnosis)`
    /// with the list of unsatisfied dependency canonical IDs.
    pub fn check_deps(&self, id: &OscId) -> Result<(), SkipDiagnosis> {
        if self.profile < Profile::L3Accumulation {
            return Ok(());
        }

        let deps = if id.operation().is_composed() {
            self.dep_graph.composed_deps(id.operation().operands())
        } else {
            self.dep_graph.transitive_deps(&id.operation().canonical())
        };

        let mut missing = Vec::new();
        for dep in &deps {
            // Operation dependencies are quantity-agnostic: a bw measurement
            // of mem.copy proves the operation works regardless of whether
            // the subject measures latency or bandwidth. Match any quantity.
            let satisfied = self.established.keys().any(|k| {
                is_dep_satisfied(k, dep)
            });
            if !satisfied {
                missing.push(format!("osc:*/{dep}"));
            }
        }

        if missing.is_empty() {
            Ok(())
        } else {
            Err(SkipDiagnosis {
                id: id.canonical(),
                missing,
            })
        }
    }

    /// Record a measurement result. Evaluates validation rules, constructs
    /// the CalibrationPoint, and marks the operation as established.
    ///
    /// `estimator` names the statistical estimator that produced the value
    /// (e.g. "criterion.mean", "divan.median", "manual"). The session does
    /// not assume any specific harness.
    /// Record a measurement. Call with `harness_coords` = None for
    /// derived/synthetic measurements, or Some((group, function, param))
    /// for harness-produced measurements that will be backfilled with
    /// real values from the harness's output files.
    pub fn record(
        &mut self,
        id: &OscId,
        value: f64,
        ci_lower: f64,
        ci_upper: f64,
        samples: u32,
        estimator: &str,
    ) {
        self.record_with_coords(id, value, ci_lower, ci_upper, samples, estimator, None);
    }

    /// Record with explicit harness coordinates for backfill.
    pub fn record_with_coords(
        &mut self,
        id: &OscId,
        value: f64,
        ci_lower: f64,
        ci_upper: f64,
        samples: u32,
        estimator: &str,
        harness_coords: Option<(&str, &str, &str)>,
    ) {
        let canonical = id.canonical_for_emission();

        // Validation (L4+)
        let established_values: BTreeMap<String, f64> = self.established.iter()
            .map(|(k, v)| (k.clone(), v.value))
            .collect();
        let validations = if self.profile >= Profile::L4Validation {
            validate::evaluate(&canonical, value, &established_values)
        } else {
            Vec::new()
        };

        let validation_records: Vec<ValidationResultRecord> =
            validations.iter().map(ValidationResultRecord::from).collect();

        let trusted = validation_records.iter()
            .all(|v| v.passed || v.severity == "Warn");

        // Dependencies: only the transitive deps of THIS operation that
        // are actually established — not the entire established set.
        let op_deps = if id.operation().is_composed() {
            self.dep_graph.composed_deps(id.operation().operands())
        } else {
            self.dep_graph.transitive_deps(&id.operation().canonical())
        };
        let satisfied_deps: Vec<String> = op_deps.iter()
            .filter_map(|dep| {
                self.established.keys()
                    .find(|k| is_dep_satisfied(k, dep))
                    .cloned()
            })
            .collect();

        let point = CalibrationPoint {
            id: canonical.clone(),
            quantity: id.quantity().token().to_owned(),
            operation: id.operation().canonical(),
            conditions: id.conditions().clone(),
            substrate: SubstrateRecord {
                arch: self.substrate.arch().to_owned(),
                os: self.substrate.os().to_owned(),
                runtime: self.substrate.runtime().to_owned(),
            },
            value,
            unit: id.quantity().unit_family().to_owned(),
            dispersion: Dispersion {
                ci_lower,
                ci_upper,
                confidence: 0.95,
                samples,
                estimator: estimator.to_owned(),
            },
            provenance: Provenance {
                timestamp_ns: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0),
                grammar_version: GRAMMAR_VERSION.to_owned(),
                harness_version: env!("CARGO_PKG_VERSION").to_owned(),
                cpu_model: self.platform.cpu_model.clone(),
                cpu_microarch: self.platform.cpu_microarch.clone(),
                cache_geometry: self.platform.cache_geometry.clone(),
                kernel: self.platform.kernel.clone(),
                runtime_detail: self.platform.runtime_detail.clone(),
            },
            dependencies: satisfied_deps,
            validations: validation_records,
            harness_coords: harness_coords.map(|(g, f, p)| {
                (g.to_owned(), f.to_owned(), p.to_owned())
            }),
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
        };

        self.records.push(point);

        self.established.insert(canonical, Established {
            value,
            ci_lower,
            ci_upper,
            trusted,
        });
    }

    /// All recorded CalibrationPoints.
    pub fn records(&self) -> &[CalibrationPoint] {
        &self.records
    }

    /// All established measurements.
    pub fn established(&self) -> &BTreeMap<String, Established> {
        &self.established
    }

    /// Emit all records as JSON Lines to a string.
    pub fn to_jsonl(&self) -> String {
        let mut out = String::new();
        for record in &self.records {
            out.push_str(&record.to_jsonl());
            out.push('\n');
        }
        out
    }

    /// The detected substrate.
    pub fn substrate(&self) -> &Substrate {
        &self.substrate
    }

    /// The conformance profile.
    pub fn profile(&self) -> Profile {
        self.profile
    }

    /// Backfill measurement values from criterion's output directory.
    ///
    /// Walks every recorded CalibrationPoint that has `harness_coords`,
    /// reads `{output_dir}/{group}/{function}/{param}/new/estimates.json`
    /// and `sample.json`, and populates value/ci_lower/ci_upper/samples
    /// with criterion's actual measured values (nanoseconds).
    ///
    /// Uses `typical()` logic: slope.point_estimate if present (Linear/Auto
    /// sampling mode), else mean.point_estimate (Flat mode).
    ///
    /// Applies `make_filename_safe` to each path component to match
    /// criterion's directory naming (replaces `?"/\*<>:|^` with `_`,
    /// truncates to 64 chars).
    ///
    /// Also updates the `established` map so validation rules operate
    /// on real values.
    ///
    /// Call this once after all benchmarks complete and before `to_jsonl()`.
    pub fn backfill_from_criterion(&mut self, output_dir: &std::path::Path) {
        let mut backfilled = 0usize;
        let mut missing = 0usize;

        for point in &mut self.records {
            let (group, function, param) = match &point.harness_coords {
                Some((g, f, p)) => (g.as_str(), f.as_str(), p.as_str()),
                None => continue,
            };

            // Apply make_filename_safe to match criterion's directory naming
            let safe_group = make_filename_safe(group);
            let safe_fn = make_filename_safe(function);
            let safe_param = make_filename_safe(param);

            // When param is empty, criterion stores the benchmark directly
            // under function/ with no param subdirectory.
            let bench_dir = if safe_param.is_empty() {
                output_dir.join(&safe_group).join(&safe_fn).join("new")
            } else {
                output_dir.join(&safe_group).join(&safe_fn).join(&safe_param).join("new")
            };

            // Read estimates.json
            let estimates_path = bench_dir.join("estimates.json");
            let estimates_content = match std::fs::read_to_string(&estimates_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        path = %estimates_path.display(),
                        osc = %point.id,
                        error = %e,
                        "backfill: estimates.json not found"
                    );
                    missing += 1;
                    continue;
                }
            };

            // Parse with serde_json — criterion's Estimates struct is stable
            let estimates: serde_json::Value = match serde_json::from_str(&estimates_content) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        path = %estimates_path.display(),
                        osc = %point.id,
                        error = %e,
                        "backfill: failed to parse estimates.json"
                    );
                    missing += 1;
                    continue;
                }
            };

            // typical() = slope if present, else mean
            let typical = if estimates["slope"].is_object() {
                &estimates["slope"]
            } else {
                &estimates["mean"]
            };

            let typical_ns = typical["point_estimate"].as_f64().unwrap_or(0.0);
            let ci_lower = typical["confidence_interval"]["lower_bound"].as_f64().unwrap_or(0.0);
            let ci_upper = typical["confidence_interval"]["upper_bound"].as_f64().unwrap_or(0.0);

            let estimator = if estimates["slope"].is_object() {
                "criterion.slope"
            } else {
                "criterion.mean"
            };

            // Extract all criterion estimates separately for JSONL enrichment
            let slope_ns = estimates["slope"]["point_estimate"].as_f64();
            let mean_ns = estimates["mean"]["point_estimate"].as_f64();
            let median_ns = estimates["median"]["point_estimate"].as_f64();
            let std_dev_ns = estimates["std_dev"]["point_estimate"].as_f64();
            let median_abs_dev_ns = estimates["median_abs_dev"]["point_estimate"].as_f64();
            // Standard error of the typical estimator (slope if present, else mean)
            let standard_error_ns = typical["standard_error"].as_f64();

            // Read sample.json for raw data and sampling mode
            let sample_path = bench_dir.join("sample.json");
            let sample_json = std::fs::read_to_string(&sample_path)
                .ok()
                .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok());

            let sample_count = sample_json.as_ref()
                .and_then(|v| v["iters"].as_array().map(|a| a.len() as u32))
                .unwrap_or(point.dispersion.samples);

            let sampling_mode = sample_json.as_ref()
                .and_then(|v| v["sampling_mode"].as_str())
                .map(|s| s.to_owned());

            let raw_iters = sample_json.as_ref()
                .and_then(|v| v["iters"].as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_f64()).collect::<Vec<_>>());

            let raw_times = sample_json.as_ref()
                .and_then(|v| v["times"].as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_f64()).collect::<Vec<_>>());

            // Read change/estimates.json for run-over-run regression data.
            // criterion writes fractional change ratios: new_mean/base_mean - 1.0
            // p_value is NOT written to disk — it's computed in-memory from the
            // t-distribution and only printed to CLI. See criterion analysis/compare.rs.
            let change_dir = if safe_param.is_empty() {
                output_dir.join(&safe_group).join(&safe_fn).join("change")
            } else {
                output_dir.join(&safe_group).join(&safe_fn).join(&safe_param).join("change")
            };
            let change_path = change_dir.join("estimates.json");
            let change_json = std::fs::read_to_string(&change_path)
                .ok()
                .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok());
            let change_mean_estimate = change_json.as_ref()
                .and_then(|v| v["mean"]["point_estimate"].as_f64());
            let change_median_estimate = change_json.as_ref()
                .and_then(|v| v["median"]["point_estimate"].as_f64());

            // Populate the record with all extracted data
            point.value = typical_ns;
            point.dispersion.ci_lower = ci_lower;
            point.dispersion.ci_upper = ci_upper;
            point.dispersion.samples = sample_count;
            point.dispersion.estimator = estimator.to_owned();
            point.slope_ns = slope_ns;
            point.mean_ns = mean_ns;
            point.median_ns = median_ns;
            point.std_dev_ns = std_dev_ns;
            point.median_abs_dev_ns = median_abs_dev_ns;
            point.standard_error_ns = standard_error_ns;
            point.sampling_mode = sampling_mode;
            point.change_mean_estimate = change_mean_estimate;
            point.change_median_estimate = change_median_estimate;
            point.raw_iters = raw_iters;
            point.raw_times = raw_times;

            // Update established map with real values
            if let Some(est) = self.established.get_mut(&point.id) {
                est.value = typical_ns;
                est.ci_lower = ci_lower;
                est.ci_upper = ci_upper;
            }

            backfilled += 1;
            tracing::debug!(
                osc = %point.id,
                typical_ns,
                ci_lower,
                ci_upper,
                ?slope_ns,
                ?mean_ns,
                ?median_ns,
                ?std_dev_ns,
                ?change_mean_estimate,
                ?change_median_estimate,
                sample_count,
                estimator,
                "backfill: populated"
            );
        }

        tracing::info!(backfilled, missing, "backfill_from_criterion: complete");
    }

    /// Apply a baseline to all records: populate `baseline_floor_ns` and
    /// `ratio_to_floor` for each record that has a matching floor.
    ///
    /// Call after `backfill_from_criterion` so records have real values.
    /// The baseline is loaded from a prior `target/calibration.jsonl`.
    pub fn apply_baseline(&mut self, baseline: &super::baseline::Baseline) {
        let mut matched = 0usize;
        for point in &mut self.records {
            // Try exact match first
            let floor = baseline.floor_ns(&point.id)
                // Fall back to operation-level match
                .or_else(|| baseline.floor_for_operation(&point.operation));

            if let Some(floor_ns) = floor {
                point.baseline_floor_ns = Some(floor_ns);
                if point.value > 0.0 && floor_ns > 0.0 {
                    point.ratio_to_floor = Some(point.value / floor_ns);
                }
                matched += 1;
            }
        }
        tracing::info!(
            matched,
            total = self.records.len(),
            baseline_floors = baseline.len(),
            "apply_baseline: complete"
        );
    }
}

/// Check if an established measurement key satisfies a dependency on an operation.
///
/// Operation dependencies are quantity-agnostic: `osc:bw/mem.copy?size=64m`
/// satisfies a dependency on `mem.copy` for a `lat` subject. The key must
/// start with `osc:` followed by any valid quantity, then `/{dep}` optionally
/// followed by `?` (conditions) or end-of-string.
fn is_dep_satisfied(established_key: &str, dep_operation: &str) -> bool {
    // Find the operation portion: after "osc:<quantity>/"
    let after_scheme = match established_key.strip_prefix("osc:") {
        Some(rest) => rest,
        None => return false,
    };
    // Skip the quantity token to find the operation
    let after_quantity = match after_scheme.find('/') {
        Some(slash) => &after_scheme[slash + 1..],
        None => return false,
    };
    // The operation is everything before `?` (conditions) or `@` (substrate)
    let op_end = after_quantity.find('?')
        .or_else(|| after_quantity.find('@'))
        .unwrap_or(after_quantity.len());
    let established_op = &after_quantity[..op_end];

    established_op == dep_operation
}

/// Replicate criterion's make_filename_safe: replace special characters
/// with `_` and truncate to 64 characters. Matches criterion's directory
/// naming so backfill path construction finds the right files.
///
/// Characters replaced: ? " / \ * < > : | ^
fn make_filename_safe(s: &str) -> String {
    const MAX_LEN: usize = 64;
    let mut out = s.replace(&['?', '"', '/', '\\', '*', '<', '>', ':', '|', '^'][..], "_");
    if out.len() > MAX_LEN {
        // Truncate at a character boundary
        let mut end = MAX_LEN;
        while end > 0 && !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
    out
}

// ── Commoditized criterion integration ──────────────────────────────
//
// These functions absorb criterion boilerplate so bench files contain
// only measurement bodies and OSC identifiers.
//
/// Format a byte size as an IEC magnitude string for OSC identifiers.
/// 1024 → "1k", 65536 → "64k", 1048576 → "1m", 64 → "64".
pub fn fmt_mag(bytes: usize) -> String {
    if bytes >= 1024 * 1024 * 1024 && bytes % (1024 * 1024 * 1024) == 0 {
        format!("{}g", bytes / (1024 * 1024 * 1024))
    } else if bytes >= 1024 * 1024 && bytes % (1024 * 1024) == 0 {
        format!("{}m", bytes / (1024 * 1024))
    } else if bytes >= 1024 && bytes % 1024 == 0 {
        format!("{}k", bytes / 1024)
    } else {
        format!("{bytes}")
    }
}

// ── Criterion-dependent helpers (gated behind bench-harness feature) ──

#[cfg(feature = "bench-harness")]
/// Build a Criterion instance pre-configured for calibration.
///
/// Applies all calibration defaults:
/// - `sample_size(20)` — sufficient for irreducible floors
/// - `measurement_time(10s)` — long enough for stable medians
/// - `nresamples(100_000)` — criterion default, explicit for documentation
/// - CI detection: disables plots when `CI=true` or `REKINDLE_BENCH_NO_PLOTS=1`
///
/// Bench authors call this once in `fn main()` instead of manually
/// configuring `Criterion::default()`.
///
/// # Example
/// ```rust,ignore
/// let mut criterion = calibrated_criterion();
/// ```
pub fn calibrated_criterion() -> criterion::Criterion {
    let mut c = criterion::Criterion::default()
        .measurement_time(std::time::Duration::from_secs(10))
        .sample_size(20)
        .nresamples(100_000);

    if std::env::var("REKINDLE_BENCH_NO_PLOTS").is_ok()
        || std::env::var("CI").is_ok()
    {
        c = c.without_plots();
        tracing::info!("calibrated criterion: plotting disabled (CI mode)");
    }

    c.configure_from_args()
}

#[cfg(feature = "bench-harness")]
/// Initialize tracing for calibration benchmarks.
///
/// - `RUST_LOG=info`: dep checks, skips, records, backfill summary
/// - `RUST_LOG=debug`: per-iteration timing, per-record backfill detail
///
/// Writes to stderr so it doesn't interfere with criterion's stdout.
pub fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"))
        )
        .with_writer(std::io::stderr)
        .init();
}

#[cfg(feature = "bench-harness")]
/// Resolve the workspace target directory from CARGO_MANIFEST_DIR.
///
/// Returns the canonicalized `../../target` relative to the crate's
/// manifest directory. Falls back to `./target` if canonicalization fails.
pub fn workspace_target_dir() -> std::path::PathBuf {
    let manifest = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_owned())
    ).join("../../target");
    manifest.canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from("target"))
}

#[cfg(feature = "bench-harness")]
/// Finalize a calibration session: backfill from criterion, apply baseline,
/// write JSONL, and call `criterion.final_summary()`.
///
/// This absorbs all the post-benchmark boilerplate that every bench file
/// would otherwise need to copy.
/// Finalize a calibration session: backfill from criterion, apply baseline,
/// write JSONL, and call `criterion.final_summary()`.
///
/// `name` identifies this bench target's JSONL output file:
/// - `"floors"` → `target/calibration_floors.jsonl`
/// - `"crypto"` → `target/calibration_crypto.jsonl`
/// - `"e2e"`    → `target/calibration_e2e.jsonl`
///
/// The baseline is always loaded from `target/calibration_floors.jsonl`
/// (the irreducible floor reference). Workload bench JSONL files are
/// separate so they don't overwrite the floors data.
pub fn finalize(
    session: &mut CalibratedSession,
    criterion: &mut criterion::Criterion,
    name: &str,
) {
    let workspace_target = workspace_target_dir();

    let criterion_dir = workspace_target.join("criterion");
    tracing::info!(path = %criterion_dir.display(), "backfill: criterion output directory");

    session.backfill_from_criterion(&criterion_dir);

    // Always load baseline from the floors file — the reference dataset.
    let baseline_path = workspace_target.join("calibration_floors.jsonl");
    let baseline = super::baseline::Baseline::load(&baseline_path);
    if !baseline.is_empty() {
        session.apply_baseline(&baseline);
    }

    let jsonl = session.to_jsonl();
    let record_count = session.records().len();
    let established_count = session.established().len();
    let jsonl_path = workspace_target.join(format!("calibration_{name}.jsonl"));
    match std::fs::write(&jsonl_path, &jsonl) {
        Ok(()) => tracing::info!(
            record_count, established_count,
            path = %jsonl_path.display(),
            "{name}: wrote JSONL"
        ),
        Err(e) => tracing::error!(
            error = %e,
            "{name}: failed to write JSONL"
        ),
    }

    criterion.final_summary();

    tracing::info!(
        record_count, established_count,
        "{name}: complete"
    );
}

#[cfg(feature = "bench-harness")]
/// Run a single calibrated benchmark: parse OSC, check deps, execute via
/// criterion, record with harness coordinates for backfill.
///
/// This is the single function a bench author calls per measurement point.
/// All criterion BenchmarkId decomposition, OSC parsing, dependency checking,
/// and session recording is handled internally.
///
/// # Parameters
/// - `session`: the calibration session for dep checking and recording
/// - `group`: the criterion benchmark group (caller creates and finishes it)
/// - `group_name`: criterion group name string (for harness_coords)
/// - `osc_str`: canonical OSC identifier string
/// - `throughput`: criterion throughput annotation
/// - `crit_fn`: criterion BenchmarkId function name
/// - `crit_param`: criterion BenchmarkId parameter (raw numeric for line charts)
/// - `sample_count`: must match the group's `sample_size`
/// - `body`: closure receiving `iters: u64`, returns elapsed `Duration`
pub fn calibrated_bench<F>(
    session: &mut CalibratedSession,
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    group_name: &str,
    osc_str: &str,
    throughput: criterion::Throughput,
    crit_fn: &str,
    crit_param: &str,
    sample_count: u32,
    mut body: F,
) where
    F: FnMut(u64) -> std::time::Duration,
{
    let id = OscId::parse(osc_str, Profile::L1Conditions)
        .unwrap_or_else(|e| panic!("malformed OSC id '{osc_str}': {e}"));

    if let Err(skip) = session.check_deps(&id) {
        tracing::warn!(osc = %id, missing = ?skip.missing, "bench: SKIPPED");
        return;
    }

    tracing::info!(osc = %id, "bench: running");
    group.throughput(throughput);
    group.bench_function(
        criterion::BenchmarkId::new(crit_fn, crit_param),
        |b| b.iter_custom(|iters| body(iters)),
    );
    session.record_with_coords(
        &id, 0.0, 0.0, 0.0, sample_count, "criterion.pending_backfill",
        Some((group_name, crit_fn, crit_param)),
    );
}

#[cfg(feature = "bench-harness")]
/// Run a contended benchmark with `thread_count` pre-spawned worker threads.
///
/// Threads are created inside `bench_function` (per criterion call),
/// barrier-gated by `iter_custom`, joined when `bench_function` returns.
///
/// `worker_fn` receives the per-thread iteration count. Thread spawn and
/// join are outside the timed region. Timing uses `elapsed.mul_f64()` to
/// scale for the actual work performed (no u32 truncation).
pub fn bench_contended<W>(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    bench_name: &str,
    thread_count: usize,
    worker_fn: W,
) where
    W: Fn(u64) + Send + Sync + 'static,
{
    use std::sync::{Arc, Barrier};
    use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};

    let worker = Arc::new(worker_fn);

    group.bench_function(bench_name, |b| {
        let go = Arc::new(Barrier::new(thread_count + 1));
        let done = Arc::new(Barrier::new(thread_count + 1));
        let iterations = Arc::new(AtomicU64::new(0));
        let running = Arc::new(AtomicBool::new(true));

        let handles: Vec<_> = (0..thread_count).map(|_| {
            let g = Arc::clone(&go);
            let d = Arc::clone(&done);
            let it = Arc::clone(&iterations);
            let r = Arc::clone(&running);
            let w = Arc::clone(&worker);
            std::thread::spawn(move || {
                loop {
                    g.wait();
                    if !r.load(Ordering::Acquire) { d.wait(); break; }
                    let n = it.load(Ordering::Acquire);
                    w(n);
                    d.wait();
                }
            })
        }).collect();

        b.iter_custom(|iters| {
            let per_thread = iters.div_ceil(thread_count as u64);
            iterations.store(per_thread, Ordering::Release);
            go.wait();
            let start = std::time::Instant::now();
            done.wait();
            let elapsed = start.elapsed();
            let actual = per_thread * thread_count as u64;
            elapsed.mul_f64(iters as f64 / actual as f64)
        });

        running.store(false, Ordering::Release);
        iterations.store(0, Ordering::Release);
        go.wait();
        done.wait();
        for h in handles { let _ = h.join(); }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_has_no_established() {
        let session = CalibratedSession::new(Profile::L4Validation);
        assert!(session.established().is_empty());
        assert!(session.records().is_empty());
    }

    #[test]
    fn record_establishes_measurement() {
        let mut session = CalibratedSession::new(Profile::L0Core);
        let id = OscId::parse("osc:bw/mem.copy?size=64m", Profile::L0Core).unwrap();
        session.record(&id, 15.0, 14.5, 15.5, 20, "test");
        assert_eq!(session.established().len(), 1);
        assert_eq!(session.records().len(), 1);
    }

    #[test]
    fn l3_checks_deps_returns_err() {
        let session = CalibratedSession::new(Profile::L3Accumulation);
        let id = OscId::parse("osc:lat/io.rtt?size=64", Profile::L0Core).unwrap();
        let result = session.check_deps(&id);
        assert!(result.is_err());
        let diag = result.unwrap_err();
        assert!(!diag.missing.is_empty());
    }

    #[test]
    fn l0_skips_dep_check() {
        let session = CalibratedSession::new(Profile::L0Core);
        let id = OscId::parse("osc:lat/io.rtt?size=64", Profile::L0Core).unwrap();
        assert!(session.check_deps(&id).is_ok());
    }

    #[test]
    fn l3_deps_satisfied_after_recording_primitives() {
        let mut session = CalibratedSession::new(Profile::L3Accumulation);

        // Record the primitives io.rtt depends on
        let nop = OscId::parse("osc:lat/io.nop", Profile::L0Core).unwrap();
        let send = OscId::parse("osc:lat/io.send?size=64", Profile::L0Core).unwrap();
        let recv = OscId::parse("osc:lat/io.recv?size=64", Profile::L0Core).unwrap();
        let wake = OscId::parse("osc:lat/sync.wake", Profile::L0Core).unwrap();

        session.record(&nop, 0.6, 0.5, 0.7, 20, "test");
        session.record(&send, 1.0, 0.9, 1.1, 20, "test");
        session.record(&recv, 1.0, 0.9, 1.1, 20, "test");
        session.record(&wake, 5.0, 4.5, 5.5, 20, "test");

        let rtt = OscId::parse("osc:lat/io.rtt?size=64", Profile::L0Core).unwrap();
        assert!(session.check_deps(&rtt).is_ok());
    }

    #[test]
    fn dependencies_field_contains_only_op_deps() {
        let mut session = CalibratedSession::new(Profile::L0Core);

        // Record two unrelated primitives
        let read = OscId::parse("osc:bw/mem.read?size=64m", Profile::L0Core).unwrap();
        let atomic = OscId::parse("osc:lat/sync.atomic", Profile::L0Core).unwrap();
        session.record(&read, 19.0, 18.5, 19.5, 20, "test");
        session.record(&atomic, 0.005, 0.004, 0.006, 20, "test");

        // Record mem.copy which depends on nothing (primitive)
        let copy = OscId::parse("osc:bw/mem.copy?size=64m", Profile::L0Core).unwrap();
        session.record(&copy, 15.0, 14.5, 15.5, 20, "test");

        // mem.copy's dependencies field should be empty (it's a primitive)
        let copy_record = &session.records()[2];
        assert!(copy_record.dependencies.is_empty());
    }

    #[test]
    fn jsonl_output_has_newlines() {
        let mut session = CalibratedSession::new(Profile::L0Core);
        let id1 = OscId::parse("osc:bw/mem.read?size=64m", Profile::L0Core).unwrap();
        let id2 = OscId::parse("osc:bw/mem.write?size=64m", Profile::L0Core).unwrap();
        session.record(&id1, 19.0, 18.5, 19.5, 20, "test");
        session.record(&id2, 32.0, 31.5, 32.5, 20, "test");
        let jsonl = session.to_jsonl();
        assert_eq!(jsonl.lines().count(), 2);
    }

    #[test]
    fn skip_diagnosis_display() {
        let diag = SkipDiagnosis {
            id: "osc:lat/io.rtt?size=64".to_owned(),
            missing: vec!["osc:lat/io.send".to_owned(), "osc:lat/io.recv".to_owned()],
        };
        let s = format!("{diag}");
        assert!(s.contains("SKIPPED"));
        assert!(s.contains("io.send"));
        assert!(s.contains("io.recv"));
    }

    #[test]
    fn estimator_is_caller_provided() {
        let mut session = CalibratedSession::new(Profile::L0Core);
        let id = OscId::parse("osc:bw/mem.copy?size=64m", Profile::L0Core).unwrap();
        session.record(&id, 15.0, 14.5, 15.5, 20, "custom.median");
        assert_eq!(session.records()[0].dispersion.estimator, "custom.median");
    }

    #[test]
    fn cross_quantity_dep_satisfaction() {
        let mut session = CalibratedSession::new(Profile::L3Accumulation);

        // Record mem.copy as bw (the mem tier measures bandwidth)
        let copy_bw = OscId::parse("osc:bw/mem.copy?size=64m", Profile::L0Core).unwrap();
        session.record(&copy_bw, 15.0, 14.5, 15.5, 20, "test");

        // crypto.seal depends on mem.copy (operation dep, quantity-agnostic)
        // Even though crypto.seal uses lat quantity, bw/mem.copy satisfies it
        let seal = OscId::parse("osc:lat/crypto.seal?size=64k&variant=aegis128l&alloc=none", Profile::L0Core).unwrap();
        let result = session.check_deps(&seal);
        assert!(result.is_ok(), "bw/mem.copy should satisfy lat/crypto.seal's dep on mem.copy");
    }

    #[test]
    fn is_dep_satisfied_helper() {
        assert!(is_dep_satisfied("osc:bw/mem.copy?size=64m", "mem.copy"));
        assert!(is_dep_satisfied("osc:lat/mem.copy?size=64m@x86_64.linux.uring", "mem.copy"));
        assert!(is_dep_satisfied("osc:ops/io.nop?ring=coop", "io.nop"));
        assert!(!is_dep_satisfied("osc:bw/mem.copy?size=64m", "mem.read"));
        assert!(!is_dep_satisfied("osc:bw/mem.copy?size=64m", "io.nop"));
        assert!(!is_dep_satisfied("not_osc:bw/mem.copy", "mem.copy"));
    }

    #[test]
    fn grammar_version_is_const() {
        let mut session = CalibratedSession::new(Profile::L0Core);
        let id = OscId::parse("osc:bw/mem.copy?size=64m", Profile::L0Core).unwrap();
        session.record(&id, 15.0, 14.5, 15.5, 20, "test");
        assert_eq!(session.records()[0].provenance.grammar_version, GRAMMAR_VERSION);
    }
}
