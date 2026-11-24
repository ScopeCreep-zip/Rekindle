//! Plan SK15: XGlow's composed Communities tab serves its source tab's
//! word style and lays its words like the sibling words.

use rekindle_types::dto::skin_tree::LabelAlign;

use super::golden_tests::main_layout;
use super::label_style::{fits, ink, size_for, word_units};
use super::test_support::find;
use crate::{builtin, package};

/// The pack's own `Downloads` word, decoded from its art
/// (`assets/frame/main/node-downloads.png`, 65 x 18).
fn source_word() -> super::label_style::Ink {
    source_ink("xglow", "assets/frame/main/node-downloads.png")
}

/// The word's ink in `pack`'s art at `path`.
pub(super) fn source_ink(pack: &str, path: &str) -> super::label_style::Ink {
    let (_, b) = builtin::find(&format!("builtin:{pack}")).unwrap();
    let package = package::read(b.bytes).unwrap();
    let frames = crate::regions::frames(path, package.get(path).unwrap()).unwrap();
    ink(&frames[0]).unwrap()
}

#[test]
fn xglow_communities_tab_takes_its_source_tabs_word_style() {
    let source = source_word();
    // The pack's word: from x 2, 7 px of capitals, #5E90A8 at its core.
    assert_eq!(source.bounds[0], 2);
    assert_eq!(source.bounds[3], 7);
    assert_eq!(source.brightest, [0x5E, 0x90, 0xA8]);
    assert!(!source.centred);

    let main = main_layout("xglow");
    let tab = find(&main.tree.as_ref().unwrap().root, "Downloads.Communities").unwrap();
    let label = tab.label.as_ref().unwrap();
    let style = label.style.as_ref().expect("served style");
    let ih = u16::try_from(source.bounds[3]).unwrap();
    assert_eq!(style.size_px, size_for(ih));
    assert_eq!(style.size_px, 9);
    assert_eq!(style.weight, 700);
    assert_eq!(style.color, "#5E90A8");
    assert_eq!(style.align, LabelAlign::Start);
    // The cover is the word's ink (the art fills the 65 x 18 node).
    let [cx, cy, cw, ch] = label.cover.unwrap();
    assert_eq!([cx, ch], [2, 7]);
    assert_eq!(usize::from(cy), source.bounds[1]);
    assert_eq!(usize::from(cw), source.bounds[2]);
    // Its words run from the ink start across the node, not the old
    // 53 px box, and fit there with the shell's label metrics.
    let [tx, _, tw, _] = style.text_box;
    assert_eq!([tx, tw], [2, 63]);
    assert!(fits("Communities", 9, tw));
    assert!(!fits("Communities", 10, tw));
}

/// xslimfire's tab word box is 53 px: "Communities" in Tahoma Bold 8 px
/// is 52.8 px (13516 of 2048 units each px) and fits it exactly; 9 px
/// (59.4) does not. The fit uses the exact advances, never a per-letter
/// round-up that steps it to 7 px.
#[test]
fn xslimfire_communities_tab_fits_the_largest_size_its_box_holds() {
    assert_eq!(word_units("Communities"), 13516);
    let main = main_layout("xslimfire");
    let tab = find(
        &main.tree.as_ref().unwrap().root,
        "TabDownloadButton.Communities",
    )
    .unwrap();
    let style = tab
        .label
        .as_ref()
        .unwrap()
        .style
        .as_ref()
        .expect("served style");
    assert_eq!(style.align, LabelAlign::Centre);
    assert_eq!(style.text_box[2], 53);
    assert_eq!(style.size_px, 8);
    assert!(fits("Communities", 8, 53));
    assert!(!fits("Communities", 9, 53));
}
