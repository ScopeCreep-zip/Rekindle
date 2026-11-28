//! The builders of [`super::SLOTS`]' entries.

use rekindle_types::dto::skin::RegionState;

use super::{Extent, RegionSlot, RegionText};

pub(super) const fn normal(token: &'static str) -> RegionText {
    RegionText {
        token,
        large: false,
    }
}

pub(super) const fn slot(
    id: &'static str,
    width: Extent,
    height: Extent,
    text: &'static [RegionText],
) -> RegionSlot {
    RegionSlot {
        id,
        width,
        height,
        text,
        states: &[],
        css: false,
        label_inset: (0, 0),
        grounds: &[],
        labelled: false,
        sized: false,
        framed: false,
    }
}

impl RegionSlot {
    pub(super) const fn states(self, states: &'static [RegionState]) -> Self {
        Self { states, ..self }
    }

    pub(super) const fn painted(self, states: &'static [RegionState]) -> Self {
        Self {
            states,
            css: true,
            ..self
        }
    }

    pub(super) const fn label_inset(self, label_inset: (u32, u32)) -> Self {
        Self {
            label_inset,
            ..self
        }
    }

    pub(super) const fn grounds(self, grounds: &'static [&'static str]) -> Self {
        Self { grounds, ..self }
    }

    /// Painted, its text on the children `regions.toml` names.
    pub(super) const fn labelled(
        self,
        states: &'static [RegionState],
        grounds: &'static [&'static str],
    ) -> Self {
        Self {
            states,
            css: true,
            grounds,
            labelled: true,
            ..self
        }
    }

    pub(super) const fn sized(self) -> Self {
        Self {
            sized: true,
            ..self
        }
    }

    /// Its element's border box is the image's nine-slice insets.
    pub(super) const fn framed(self) -> Self {
        Self {
            framed: true,
            ..self
        }
    }

    /// Whether the slot has `state` (normal is the slot's image).
    #[must_use]
    pub fn has(&self, state: RegionState) -> bool {
        self.states.contains(&state)
    }
}
