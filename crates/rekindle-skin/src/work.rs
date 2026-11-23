//! Test-only work counters of [`crate::serve::stylesheet`]'s stages, the
//! pattern of the title's `COL_EVALS` (`regions/title.rs`): each stage
//! that walks window sizes, pixels or image bytes adds its steps on this
//! thread, and `serve_tests.rs` holds every built-in's totals to bounds
//! no machine load can move (plan E7.7.7: the work gates, the wall clock
//! is a report, `tests/serve_budget.rs`).

use std::cell::Cell;
use std::thread::LocalKey;

thread_local! {
    /// The pixels [`crate::regions`]' `worst_contrast` visits (columns ×
    /// rows of each call).
    pub(crate) static CONTRAST_VISITS: Cell<u64> = const { Cell::new(0) };
    /// The pixels of the images those calls measure, one image per call.
    pub(crate) static CONTRAST_PIXELS: Cell<u64> = const { Cell::new(0) };
    /// The `worst_contrast` calls.
    pub(crate) static CONTRAST_CALLS: Cell<u64> = const { Cell::new(0) };
    /// The window × side cases the region check measures (`WINDOWS` × min
    /// and max per frame).
    pub(crate) static REGION_CASES: Cell<u64> = const { Cell::new(0) };
    /// The window widths (or heights) a label box over
    /// `Extent::Window` spans, summed over its windows.
    pub(crate) static LABEL_SPANS: Cell<u64> = const { Cell::new(0) };
    /// The image decodes (`regions::frames` and `images::decode_png`).
    pub(crate) static DECODES: Cell<u64> = const { Cell::new(0) };
    /// The frames those decodes returned.
    pub(crate) static DECODED_FRAMES: Cell<u64> = const { Cell::new(0) };
}

/// Adds `n` to the counter `key` on this thread.
pub(crate) fn add(key: &'static LocalKey<Cell<u64>>, n: u64) {
    key.with(|c| c.set(c.get().saturating_add(n)));
}

/// The work one run of a stage cost on this thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Work {
    pub col_evals: u64,
    pub contrast_visits: u64,
    pub contrast_pixels: u64,
    pub contrast_calls: u64,
    pub region_cases: u64,
    pub label_spans: u64,
    pub decodes: u64,
    pub decoded_frames: u64,
}

const KEYS: [&LocalKey<Cell<u64>>; 8] = [
    &crate::regions::title::COL_EVALS,
    &CONTRAST_VISITS,
    &CONTRAST_PIXELS,
    &CONTRAST_CALLS,
    &REGION_CASES,
    &LABEL_SPANS,
    &DECODES,
    &DECODED_FRAMES,
];

/// The work `f` costs on this thread, every counter reset first.
pub(crate) fn measure(f: impl FnOnce()) -> Work {
    for k in KEYS {
        k.with(|c| c.set(0));
    }
    f();
    let [col_evals, contrast_visits, contrast_pixels, contrast_calls, region_cases, label_spans, decodes, decoded_frames] =
        KEYS.map(|k| k.with(Cell::get));
    Work {
        col_evals,
        contrast_visits,
        contrast_pixels,
        contrast_calls,
        region_cases,
        label_spans,
        decodes,
        decoded_frames,
    }
}
