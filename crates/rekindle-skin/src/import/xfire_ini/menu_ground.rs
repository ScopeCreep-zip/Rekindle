//! The ground an INI main window draws its menu text on (ST1 ruling 3:
//! a text role is held against the ground the pack drew it on; lead round
//! 4 d). An INI skin has no menu-bar control: `[MainWindow]` lays its menu
//! titles (`Text.color=MenuTextCG`) straight on the window's frame art, so
//! `menubar.bg` is the alpha-weighted mean of the frame's Normal image
//! under that text: the text's line (`Text.size` rows from its `Y`), from
//! its `X` to the frame's right end piece. Combat lays `[FileMenuText]` at
//! 54,174 on the grey bar between two black rules of `c_top_left.gif` and
//! `c_top_middle.gif`, not on its white window ground (`bg.dark`).

use rekindle_types::dto::skin::RegionState;

use super::frame::Frame;
use super::ini::Ini;
use super::title;
use crate::import::dialog_roles::region_mean;

/// The colour group Xfire draws its menu titles in.
const MENU_TEXT: &str = "MenuTextCG";

/// The mean of the frame art under the first menu title `[MainWindow]`
/// lays at a numeric position, or `None` when it lays none there.
pub(super) fn of(skin: &Ini, frame: Option<&Frame>) -> Option<[u8; 3]> {
    let frame = frame?;
    let img = frame.images.get(&RegionState::Normal)?;
    let end = img.width().saturating_sub(u32::from(frame.insets.right));
    let main = skin.section("MainWindow")?;
    title::windows(main).values().find_map(|w| {
        let text = skin.section(w.get("Name")?)?;
        if !text
            .get("Text.color")?
            .trim()
            .eq_ignore_ascii_case(MENU_TEXT)
        {
            return None;
        }
        let x: u32 = w.get("X")?.trim().parse().ok()?;
        let y: u32 = w.get("Y")?.trim().parse().ok()?;
        let size: u32 = text.get("Text.size")?.trim().parse().ok()?;
        let bottom = y.saturating_add(size).min(img.height());
        (x < end && y < bottom)
            .then(|| region_mean(img, x..end, y..bottom))
            .flatten()
    })
}
