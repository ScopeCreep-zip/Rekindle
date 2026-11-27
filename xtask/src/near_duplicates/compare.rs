//! The three comparisons of research §1.4 (steps 2, 3 and 5): function
//! pairs by the Jaccard index of their window shingles, statement runs by
//! the renamed hash of every run of `RUN_STMTS` consecutive statements, and
//! file pairs by the Jaccard index of their shingle sets.
//!
//! Candidate pairs come from an inverted index over the shingles. A shingle
//! held by more than `POSTING_CAP` units only proposes no candidates (it is
//! boilerplate, it says nothing about which two units are related); every
//! candidate's similarity is then computed over its full shingle sets, so the
//! cap never hides shared code from the score.

use std::collections::{HashMap, HashSet};

use super::tokens::{ratio, renamed_hash, shared, shingles, Tok};
use super::units::FileUnits;
use super::{Finding, Loc, Thresholds};

/// The most units one shingle may propose candidates among.
const POSTING_CAP: usize = 32;

/// One parsed file.
pub(crate) struct Parsed {
    pub(crate) path: String,
    pub(crate) units: FileUnits,
}

type FnRef = (usize, usize);

fn fn_loc(files: &[Parsed], (f, u): FnRef, what: String) -> Loc {
    let unit = &files[f].units.fns[u];
    Loc {
        file: files[f].path.clone(),
        start: unit.start,
        end: unit.end,
        what,
    }
}

/// Pairs of ids that share at least one shingle held by at most
/// `POSTING_CAP` ids.
fn candidates(sets: &[Vec<u64>]) -> HashSet<(usize, usize)> {
    let mut index: HashMap<u64, Vec<usize>> = HashMap::new();
    for (id, set) in sets.iter().enumerate() {
        for &h in set {
            index.entry(h).or_default().push(id);
        }
    }
    let mut pairs = HashSet::new();
    for ids in index.values() {
        if ids.len() < 2 || ids.len() > POSTING_CAP {
            continue;
        }
        for (k, &a) in ids.iter().enumerate() {
            for &b in &ids[k + 1..] {
                pairs.insert((a.min(b), a.max(b)));
            }
        }
    }
    pairs
}

fn jaccard(a: &[u64], b: &[u64]) -> f64 {
    let n = shared(a, b);
    ratio(n, a.len() + b.len() - n)
}

/// Function pairs at or above `t.fn_similarity`; returns the pairs found.
pub(crate) fn function_pairs(
    files: &[Parsed],
    t: &Thresholds,
    out: &mut Vec<Finding>,
) -> HashSet<(FnRef, FnRef)> {
    let mut refs: Vec<FnRef> = Vec::new();
    let mut sets: Vec<Vec<u64>> = Vec::new();
    for (f, file) in files.iter().enumerate() {
        for (u, unit) in file.units.fns.iter().enumerate() {
            if unit.toks.len() >= t.min_fn_tokens {
                refs.push((f, u));
                sets.push(shingles(&unit.toks, t.shingle));
            }
        }
    }
    let mut found = HashSet::new();
    let mut pairs: Vec<(usize, usize)> = candidates(&sets).into_iter().collect();
    pairs.sort_unstable();
    for (a, b) in pairs {
        let (ra, rb) = (refs[a], refs[b]);
        if nested(files, ra, rb) {
            continue;
        }
        let sim = jaccard(&sets[a], &sets[b]);
        if sim < t.fn_similarity {
            continue;
        }
        let (ua, ub) = (&files[ra.0].units.fns[ra.1], &files[rb.0].units.fns[rb.1]);
        out.push(Finding {
            kind: "function",
            a: fn_loc(files, ra, format!("fn {}", ua.name)),
            b: fn_loc(files, rb, format!("fn {}", ub.name)),
            tokens: ua.toks.len().min(ub.toks.len()),
            similarity: sim,
        });
        found.insert((ra, rb));
    }
    found
}

/// True when one function lies inside the other (a nested `fn`).
fn nested(files: &[Parsed], a: FnRef, b: FnRef) -> bool {
    if a.0 != b.0 {
        return false;
    }
    let (x, y) = (&files[a.0].units.fns[a.1], &files[b.0].units.fns[b.1]);
    (x.start <= y.start && y.end <= x.end) || (y.start <= x.start && x.end <= y.end)
}

/// Identical (after renaming) runs of `t.run_stmts` statements of at least
/// `t.min_run_tokens` tokens in two places, contiguous windows merged, runs
/// inside an already reported function pair left out.
pub(crate) fn statement_runs(
    files: &[Parsed],
    t: &Thresholds,
    fn_pairs: &HashSet<(FnRef, FnRef)>,
    out: &mut Vec<Finding>,
) {
    let blocks: Vec<(usize, usize)> = files
        .iter()
        .enumerate()
        .flat_map(|(f, file)| (0..file.units.blocks.len()).map(move |b| (f, b)))
        .collect();
    let stmts_of = |g: usize| &files[blocks[g].0].units.blocks[blocks[g].1].stmts;
    let mut windows: HashMap<u64, Vec<(usize, usize)>> = HashMap::new();
    for g in 0..blocks.len() {
        let stmts = stmts_of(g);
        for i in 0..stmts.len().saturating_sub(t.run_stmts - 1) {
            let run: Vec<Tok> = stmts[i..i + t.run_stmts].concat();
            if run.len() >= t.min_run_tokens {
                windows.entry(renamed_hash(&run)).or_default().push((g, i));
            }
        }
    }
    // (block a, block b, start a − start b) → starts in a.
    let mut aligned: HashMap<(usize, usize, isize), Vec<usize>> = HashMap::new();
    for places in windows.values() {
        for (k, &(ga, ia)) in places.iter().enumerate() {
            for &(gb, ib) in &places[k + 1..] {
                if ga == gb && ia.abs_diff(ib) < t.run_stmts {
                    continue;
                }
                let ((ga, ia), (gb, ib)) = if (ga, ia) <= (gb, ib) {
                    ((ga, ia), (gb, ib))
                } else {
                    ((gb, ib), (ga, ia))
                };
                let shift = isize::try_from(ia).unwrap_or(isize::MAX)
                    - isize::try_from(ib).unwrap_or(isize::MAX);
                aligned.entry((ga, gb, shift)).or_default().push(ia);
            }
        }
    }
    let mut keys: Vec<_> = aligned.keys().copied().collect();
    keys.sort_unstable();
    for key in keys {
        let mut starts = aligned[&key].clone();
        starts.sort_unstable();
        starts.dedup();
        let (ga, gb, shift) = key;
        let (fa, fb) = (blocks[ga], blocks[gb]);
        let func_a = (fa.0, files[fa.0].units.blocks[fa.1].func);
        let func_b = (fb.0, files[fb.0].units.blocks[fb.1].func);
        if fn_pairs.contains(&(func_a, func_b)) || fn_pairs.contains(&(func_b, func_a)) {
            continue;
        }
        for (first, last) in merge_runs(&starts) {
            let end = last + t.run_stmts;
            let tokens: usize = stmts_of(ga)[first..end].iter().map(Vec::len).sum();
            let first_b = usize::try_from(isize::try_from(first).unwrap_or(0) - shift).unwrap_or(0);
            let end_b = first_b + (end - first);
            let what = |func: FnRef, s: usize, e: usize| {
                format!(
                    "fn {}, statements {}..{}",
                    files[func.0].units.fns[func.1].name,
                    s + 1,
                    e
                )
            };
            out.push(Finding {
                kind: "statements",
                a: fn_loc(files, func_a, what(func_a, first, end)),
                b: fn_loc(files, func_b, what(func_b, first_b, end_b)),
                tokens,
                similarity: 1.0,
            });
        }
    }
}

/// Contiguous runs of sorted starts, as (first start, last start).
fn merge_runs(starts: &[usize]) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for &s in starts {
        match runs.last_mut() {
            Some((_, last)) if s == *last + 1 => *last = s,
            _ => runs.push((s, s)),
        }
    }
    runs
}

/// File pairs whose shingle sets share at least `t.min_file_shared`
/// shingles at a Jaccard index of at least `t.file_similarity` (a ported
/// module: many medium matches rather than one exact one, research §1.4
/// step 5).
pub(crate) fn file_pairs(files: &[Parsed], t: &Thresholds, out: &mut Vec<Finding>) {
    let sets: Vec<Vec<u64>> = files
        .iter()
        .map(|file| {
            let mut set: Vec<u64> = file
                .units
                .fns
                .iter()
                .flat_map(|u| shingles(&u.toks, t.shingle))
                .collect();
            set.sort_unstable();
            set.dedup();
            set
        })
        .collect();
    let mut pairs: Vec<(usize, usize)> = candidates(&sets).into_iter().collect();
    pairs.sort_unstable();
    for (a, b) in pairs {
        let common = shared(&sets[a], &sets[b]);
        let sim = jaccard(&sets[a], &sets[b]);
        if common < t.min_file_shared || sim < t.file_similarity {
            continue;
        }
        let loc = |side: usize| Loc {
            file: files[side].path.clone(),
            start: 1,
            end: files[side]
                .units
                .fns
                .iter()
                .map(|u| u.end)
                .max()
                .unwrap_or(1),
            what: format!("{} shingles", sets[side].len()),
        };
        out.push(Finding {
            kind: "file",
            a: loc(a),
            b: loc(b),
            tokens: common,
            similarity: sim,
        });
    }
}
