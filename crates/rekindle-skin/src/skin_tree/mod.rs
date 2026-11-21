//! The pack's component tree (plan SK2, design `sk2/design.md` §5): the
//! name → role table, the importer that resolves a pack's tiles into a
//! [`rekindle_types::dto::skin_tree::SkinTree`], the Host's validation
//! (rules 1–10, run by [`crate::shape::check`] before a package is served)
//! and the stylesheet's one rule per node. The shell renders it in SK2-S2.

mod anchor;
pub mod css;
pub mod import;
mod passes;
pub mod roles;
pub mod validate;

#[cfg(test)]
mod fit_tests;
#[cfg(test)]
mod golden_tests;
#[cfg(test)]
mod stack_tests;
#[cfg(test)]
mod tests;
