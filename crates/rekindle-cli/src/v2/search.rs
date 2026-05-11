//! Project-wide search integration via fff (Fast File Finder).
//!
//! Wraps `fff-core` with rekindle-specific initialization, lifecycle
//! management, and search APIs. Used by both CLI one-shot commands
//! and the TUI's interactive search surfaces.
//!
//! Thread safety:
//! - Multiple concurrent `search_files` / `grep` calls are safe (read locks)
//! - `on_open` drops frecency lock before acquiring picker write lock
//! - `track_query_completion` spawned in background thread (LMDB write)
//! - NEVER call `fuzzy_search` or `grep` on an async executor — use `spawn_blocking`

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use fff_search::{
    FFFMode, FilePickerOptions, FuzzySearchOptions, GrepMode, GrepSearchOptions,
    PaginationArgs, QueryParser, SharedFrecency, SharedFilePicker, SharedQueryTracker,
};
use fff_search::file_picker::FilePicker;
use fff_search::frecency::FrecencyTracker;
use fff_search::grep::parse_grep_query;
use fff_search::query_tracker::QueryTracker;
use fff_search::types::Score;
use fff_query_parser::MixedSearchConfig;

/// Project-wide search engine backed by fff.
pub struct RekindleSearch {
    pub picker: SharedFilePicker,
    pub frecency: SharedFrecency,
    pub query_tracker: SharedQueryTracker,
    grep_abort: Arc<AtomicBool>,
}

impl RekindleSearch {
    /// Initialize search for a project root.
    ///
    /// Spawns background scan + watcher (non-blocking, returns immediately).
    /// Call `wait_for_scan()` if you need results before the scan completes.
    pub fn init(project_root: &str, ai_mode: bool) -> anyhow::Result<Self> {
        let picker = SharedFilePicker::default();
        let frecency = SharedFrecency::default();
        let qt = SharedQueryTracker::default();

        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rekindle");
        std::fs::create_dir_all(&data_dir)?;

        // Init frecency DB (LMDB)
        let ft = FrecencyTracker::open(data_dir.join("frecency"))?;
        frecency.init(ft)?;
        let _ = frecency.spawn_gc(data_dir.join("frecency").to_string_lossy().to_string());

        // Init query tracker DB (LMDB)
        let tracker = QueryTracker::open(data_dir.join("queries"))?;
        qt.init(tracker)?;

        // Start background scan + file watcher
        FilePicker::new_with_shared_state(
            picker.clone(),
            frecency.clone(),
            FilePickerOptions {
                base_path: project_root.into(),
                enable_mmap_cache: !cfg!(target_os = "windows"),
                enable_content_indexing: true,
                watch: true,
                mode: if ai_mode { FFFMode::Ai } else { FFFMode::Neovim },
                cache_budget: None,
            },
        )?;

        Ok(Self {
            picker,
            frecency,
            query_tracker: qt,
            grep_abort: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Initialize for CLI one-shot (synchronous scan, no watcher).
    pub fn init_oneshot(project_root: &str) -> anyhow::Result<Self> {
        let picker = SharedFilePicker::default();
        let frecency = SharedFrecency::default();
        let qt = SharedQueryTracker::default();

        let mut p = FilePicker::new(FilePickerOptions {
            base_path: project_root.into(),
            enable_mmap_cache: false,
            enable_content_indexing: false,
            watch: false,
            mode: FFFMode::Neovim,
            cache_budget: None,
        })?;
        p.collect_files()?;

        {
            let mut guard = picker.write()?;
            *guard = Some(p);
        }

        Ok(Self {
            picker,
            frecency,
            query_tracker: qt,
            grep_abort: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Block until the background scan completes (with timeout).
    pub fn wait_for_scan(&self, timeout: std::time::Duration) -> bool {
        self.picker.wait_for_scan(timeout)
    }

    /// Whether the background scan is still in progress.
    /// Returns false if the picker is not yet initialized or scan status
    /// cannot be determined.
    pub fn is_scanning(&self) -> bool {
        // SharedFilePicker::wait_for_scan with zero timeout returns
        // true immediately if scan is complete, false if still running.
        !self.picker.wait_for_scan(std::time::Duration::ZERO)
    }

    /// Fuzzy file search — for TUI quick switcher (`Ctrl+K`), CLI `rekindle search`.
    pub fn search_files(
        &self,
        query: &str,
        current_file: Option<&str>,
        limit: usize,
    ) -> Vec<(String, Score)> {
        let Ok(guard) = self.picker.read() else { return vec![] };
        let Some(ref picker) = *guard else { return vec![]; };
        let qt_guard = self.query_tracker.read().ok();

        let parser = QueryParser::default();
        let q = parser.parse(query);

        let results = picker.fuzzy_search(
            &q,
            qt_guard.as_ref().and_then(|g| g.as_ref()),
            FuzzySearchOptions {
                max_threads: 4,
                current_file,
                project_path: Some(picker.base_path()),
                combo_boost_score_multiplier: 100,
                min_combo_count: 3,
                pagination: PaginationArgs { offset: 0, limit },
            },
        );

        results
            .items
            .iter()
            .zip(&results.scores)
            .map(|(f, s)| (f.relative_path(picker), s.clone()))
            .collect()
    }

    /// Content grep — for TUI grep, CLI `rekindle grep`.
    ///
    /// Takes `&mut self` because each grep invocation replaces the abort signal
    /// (`self.grep_abort = Arc::clone(&new_abort)`) so that previous in-flight
    /// greps are cancelled. This requires exclusive access. No current caller
    /// needs concurrent grep — CLI is single-threaded, TUI already holds
    /// `&mut self` on App. If concurrent grep is needed in the future, replace
    /// `grep_abort` with `ArcSwap<AtomicBool>` to make this `&self`.
    pub fn grep(
        &mut self,
        query: &str,
        mode: GrepMode,
        file_offset: usize,
        limit: usize,
    ) -> Vec<(String, u64, String)> {
        // Abort the previous grep (if running concurrently in another context)
        self.grep_abort.store(true, Ordering::Relaxed);
        // Create a fresh abort signal and store it for future cancellation
        let new_abort = Arc::new(AtomicBool::new(false));
        self.grep_abort = Arc::clone(&new_abort);

        let Ok(guard) = self.picker.read() else { return vec![] };
        let Some(ref picker) = *guard else { return vec![]; };

        let parsed = parse_grep_query(query);
        let result = picker.grep(&parsed, &GrepSearchOptions {
            mode,
            smart_case: true,
            file_offset,
            page_limit: limit,
            time_budget_ms: 150,
            classify_definitions: true,
            trim_whitespace: false,
            abort_signal: Some(new_abort),
            ..Default::default()
        });

        result.matches.iter().map(|m| {
            let path = result.files.get(m.file_index)
                .map(|f| f.relative_path(picker))
                .unwrap_or_default();
            (path, m.line_number, m.line_content.clone())
        }).collect()
    }

    /// Mixed file+dir search — for command palette.
    pub fn search_mixed(
        &self,
        query: &str,
        current_file: Option<&str>,
        limit: usize,
    ) -> Vec<(String, Score)> {
        let Ok(guard) = self.picker.read() else { return vec![] };
        let Some(ref picker) = *guard else { return vec![]; };
        let qt_guard = self.query_tracker.read().ok();

        let parser = QueryParser::new(MixedSearchConfig);
        let q = parser.parse(query);

        let results = picker.fuzzy_search_mixed(
            &q,
            qt_guard.as_ref().and_then(|g| g.as_ref()),
            FuzzySearchOptions {
                max_threads: 4,
                current_file,
                project_path: Some(picker.base_path()),
                combo_boost_score_multiplier: 100,
                min_combo_count: 3,
                pagination: PaginationArgs { offset: 0, limit },
            },
        );

        results.items.iter().zip(&results.scores)
            .map(|(item, score)| {
                let path = match item {
                    fff_search::types::MixedItemRef::File(f) => f.relative_path(picker),
                    fff_search::types::MixedItemRef::Dir(d) => d.relative_path(picker),
                };
                (path, score.clone())
            })
            .collect()
    }

    /// Track file open — updates frecency and combo boost.
    ///
    /// Lock ordering: drop frecency read → acquire picker write.
    pub fn on_open(&self, query: &str, abs_path: &str) {
        let project_path = {
            let g = self.picker.read().ok();
            g.and_then(|g| g.as_ref().map(|p| p.base_path().to_path_buf()))
        };
        let Some(project_path) = project_path else { return; };

        // 1. Track frecency (LMDB write — do NOT hold picker lock)
        if let Ok(g) = self.frecency.read() {
            if let Some(ref ft) = *g {
                let _ = ft.track_access(Path::new(abs_path));
            }
        }

        // 2. Update in-memory frecency score (brief write lock)
        if let Ok(mut g) = self.picker.write() {
            if let Some(ref mut picker) = *g {
                if let Ok(fg) = self.frecency.read() {
                    if let Some(ref ft) = *fg {
                        let _ = picker.update_single_file_frecency(abs_path, ft);
                    }
                }
            }
        }

        // 3. Track query→file for combo boost (background thread)
        let qt = self.query_tracker.clone();
        let q = query.to_string();
        let fp = abs_path.to_string();
        std::thread::spawn(move || {
            if let Ok(mut g) = qt.write() {
                if let Some(ref mut tracker) = *g {
                    let _ = tracker.track_query_completion(&q, &project_path, Path::new(&fp));
                }
            }
        });
    }

    /// Get recent file picker queries from history.
    pub fn recent_queries(&self, limit: usize) -> Vec<String> {
        let project_path = {
            let g = self.picker.read().ok();
            g.and_then(|g| g.as_ref().map(|p| p.base_path().to_path_buf()))
        };
        let Some(project_path) = project_path else { return vec![]; };

        let Ok(qt_guard) = self.query_tracker.read() else { return vec![] };
        let Some(ref tracker) = *qt_guard else { return vec![]; };

        let mut queries = Vec::new();
        for offset in 0..limit {
            match tracker.get_historical_query(&project_path, offset) {
                Ok(Some(q)) => {
                    if !queries.contains(&q) {
                        queries.push(q);
                    }
                }
                _ => break,
            }
        }
        queries
    }

    /// Get the project base path.
    pub fn base_path(&self) -> Option<PathBuf> {
        self.picker
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|p| p.base_path().to_path_buf()))
    }

    /// Trigger manual rescan (e.g., after git pull).
    pub fn rescan(&self) -> anyhow::Result<()> {
        self.picker.trigger_full_rescan_async(&self.frecency)?;
        Ok(())
    }
}
