//! Plan SK15: XGlow's composed Communities tab serves its source tab's
//! word style and lays its words like the sibling words.

use rekindle_types::dto::skin_tree::LabelAlign;

use super::golden_tests::main_layout;
use super::label_style::{fits, ink, size_for, word_units, BOLD_WEIGHT, REGULAR_WEIGHT};
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
    // Its stems are 1 px: Tahoma Regular.
    assert_eq!(source.stem, 1);

    let main = main_layout("xglow");
    let tab = find(&main.tree.as_ref().unwrap().root, "Downloads.Communities").unwrap();
    let label = tab.label.as_ref().unwrap();
    let style = label.style.as_ref().expect("served style");
    let ih = u16::try_from(source.bounds[3]).unwrap();
    assert_eq!(style.size_px, size_for(ih));
    assert_eq!(style.size_px, 10);
    assert_eq!(style.weight, REGULAR_WEIGHT);
    assert_eq!(style.color, "#5E90A8");
    assert_eq!(style.align, LabelAlign::Start);
    // The cover is the word's ink (the art fills the 65 x 18 node).
    let [cx, cy, cw, ch] = label.cover.unwrap();
    assert_eq!([cx, ch], [2, 7]);
    assert_eq!(usize::from(cy), source.bounds[1]);
    assert_eq!(usize::from(cw), source.bounds[2]);
    // Its words run from the ink start across the node, not the old
    // 53 px box, and fit there at the measured Regular 10 px (57.0 px).
    let [tx, _, tw, _] = style.text_box;
    assert_eq!([tx, tw], [2, 63]);
    assert!(fits("Communities", REGULAR_WEIGHT, 10, tw));
}

/// Tahoma's hinted device advances at 10 ppem (its `hdmx` record for 10
/// ppem), Regular then Bold, for the letters of the packs' tab words: the
/// widths GDI drew the art with.
fn gdi_width(word: &str, weight: u16) -> usize {
    word.chars()
        .map(|c| {
            let (regular, bold) = match c {
                'D' => (7, 8),
                'F' | 'S' | 'd' | 'n' | 'o' | 'v' => (6, 6),
                'a' | 'e' => (5, 6),
                'i' | 'l' => (2, 3),
                'r' | 's' => (4, 5),
                'w' => (8, 9),
                other => panic!("no device width for {other:?}"),
            };
            if weight == BOLD_WEIGHT {
                bold
            } else {
                regular
            }
        })
        .sum()
}

/// The ground truth: each pack's tab words, measured from the art, are
/// the words GDI draws at the measured weight and Tahoma 10 px, within 1
/// px of ink width; the other weight is not. XGlow's 1 px stems are
/// Regular (34, 34, 51 px of ink against 33, 34, 50) and its 7 px of
/// capitals measure 10 px; xslimfire's 2 px stems are Bold (36, 38
/// against 37, 39), its ink 8 px tall with the Bold "d" rising 1 px over
/// its capitals in "Friends" (measured 11 px; its Communities fit
/// steps to 8).
#[test]
fn measured_weight_and_size_reproduce_the_arts_own_words() {
    let words = [
        ("xglow", "node-friends.png", "Friends", REGULAR_WEIGHT, 7),
        ("xglow", "node-servers.png", "Servers", REGULAR_WEIGHT, 7),
        (
            "xglow",
            "node-downloads.png",
            "Downloads",
            REGULAR_WEIGHT,
            7,
        ),
        (
            "xslimfire",
            "node-tabnormalbutton.png",
            "Friends",
            BOLD_WEIGHT,
            8,
        ),
        (
            "xslimfire",
            "node-tabserverbutton.png",
            "Servers",
            BOLD_WEIGHT,
            7,
        ),
    ];
    assert_eq!(size_for(7), 10);
    assert_eq!(size_for(8), 11);
    for (pack, file, word, weight, tall) in words {
        let source = source_ink(pack, &format!("assets/frame/main/{file}"));
        let measured = super::label_style::weight_for(source.stem);
        assert_eq!(measured, weight, "{pack} {word} stem {}", source.stem);
        assert_eq!(source.bounds[3], tall, "{pack} {word} ink height");
        let inked = source.bounds[2];
        let drawn = gdi_width(word, measured);
        assert!(
            inked.abs_diff(drawn) <= 1,
            "{pack} {word}: ink {inked} px, GDI {drawn} px"
        );
        let other = if measured == BOLD_WEIGHT {
            REGULAR_WEIGHT
        } else {
            BOLD_WEIGHT
        };
        assert!(inked.abs_diff(gdi_width(word, other)) > 1, "{pack} {word}");
    }
}

/// xslimfire's tab word box is 53 px: "Communities" in Tahoma Bold 8 px
/// is 52.8 px (13516 of 2048 units each px) and fits it exactly; 9 px
/// (59.4) does not. The fit uses the exact advances, never a per-letter
/// round-up that steps it to 7 px.
#[test]
fn xslimfire_communities_tab_fits_the_largest_size_its_box_holds() {
    assert_eq!(word_units("Communities", BOLD_WEIGHT), 13516);
    assert_eq!(word_units("Communities", REGULAR_WEIGHT), 11679);
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
    assert_eq!(style.weight, BOLD_WEIGHT);
    assert!(fits("Communities", BOLD_WEIGHT, 8, 53));
    assert!(!fits("Communities", BOLD_WEIGHT, 9, 53));
}
