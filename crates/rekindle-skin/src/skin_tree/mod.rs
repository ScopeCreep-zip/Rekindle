//! The pack's component tree (plan SK2, design `sk2/design.md` §5): the
//! name → role table, the importer that resolves a pack's tiles into a
//! [`rekindle_types::dto::skin_tree::SkinTree`], the Host's validation
//! (rules 1–10, run by [`crate::shape::check`] before a package is served)
//! and the stylesheet's one rule per node. The shell renders it in SK2-S2.

mod anchor;
mod anchor_client;
mod anchor_share;
pub mod brand;
pub mod chat_roles;
pub mod chat_slots;
mod clip;
pub mod community_row;
pub mod compose;
pub mod content;
pub mod cover;
pub mod css;
pub mod dim;
pub mod disabled;
mod fit;
pub mod import;
pub mod inset;
mod keep;
pub mod label_style;
mod layer;
mod outdent;
mod passes;
pub mod pieces;
pub mod place;
pub mod popup;
pub mod roles;
pub mod row_pages;
mod settle;
mod span_css;
mod tab_step;
pub mod target;
pub mod text_box;
pub mod titles;
mod triggers;
pub mod validate;

#[cfg(test)]
mod anchor_tests;
#[cfg(test)]
mod chat_tests;
#[cfg(test)]
mod community_row_tests;
#[cfg(test)]
mod community_tests;
#[cfg(test)]
mod compose_tests;
#[cfg(test)]
mod content_tests;
#[cfg(test)]
mod disabled_tests;
#[cfg(test)]
mod fit_tests;
#[cfg(test)]
mod golden_tests;
#[cfg(test)]
mod grip_menu_tests;
#[cfg(test)]
mod grip_tests;
#[cfg(test)]
mod label_style_tests;
#[cfg(test)]
mod overlap_tests;
#[cfg(test)]
mod place_tests;
#[cfg(test)]
mod rail_tests;
#[cfg(test)]
mod row_pages_tests;
#[cfg(test)]
mod settle_tests;
#[cfg(test)]
mod stack_tests;
#[cfg(test)]
mod target_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod text_box_tests;
#[cfg(test)]
mod triggers_tests;
