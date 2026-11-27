use super::super::ini::parse;
use super::views_insets;

fn main_of(text: &str) -> Option<[u32; 4]> {
    let ini = parse("skin.ini", text.as_bytes()).unwrap();
    views_insets(&ini, ini.section("MainWindow").unwrap())
}

const VIEWS: &str = "[Views]\nClass=TabControl\nWindow1.Name=BuddyView\n[BuddyView]\nClass=BuddyListView\n[Back]\nClass=Tile\n";

/// Plan SK0-L1, [`super::Frame::views`]: the window holding the buddy
/// list (any name: XGlow's `TabbedControl`, Combat's `MainTabbedArea`)
/// stretched both ways gives `X` + `IndentLeft`, `Y`, `IndentRight`
/// and `IndentBottom` (XGlow 28/64/31/159); one justified right or
/// bottom, not stretched both ways, placed by a non-number, or holding
/// no `BuddyListView` reads none.
#[test]
fn views_are_the_stretched_window_holding_the_buddy_list() {
    let base = format!("[MainWindow]\nWindow1.Name=Back\nWindow1.ResizeX=100\nWindow1.ResizeY=100\nWindow2.Name=Views\nWindow2.ResizeX=100\nWindow2.ResizeY=100\nWindow2.X=20\nWindow2.IndentLeft=8\nWindow2.Y=64\nWindow2.IndentRight=31\nWindow2.IndentBottom=159\n{VIEWS}");
    assert_eq!(main_of(&base), Some([28, 64, 31, 159]));
    let named = base.replace("Views", "MainTabbedArea");
    assert_eq!(main_of(&named), Some([28, 64, 31, 159]));
    let right = base.replace("Window2.Y=64", "Window2.Y=64\nWindow2.JustX=RIGHT");
    assert_eq!(main_of(&right), None);
    let bottom = base.replace("Window2.Y=64", "Window2.Y=64\nWindow2.JustY=BOTTOM");
    assert_eq!(main_of(&bottom), None);
    assert_eq!(
        main_of(&base.replace("Window2.ResizeY=100", "Window2.ResizeY=0")),
        None
    );
    assert_eq!(main_of(&base.replace("Y=64", "Y=Back.BOTTOM")), None);
    assert_eq!(
        main_of(&base.replace("Class=BuddyListView", "Class=ServerView")),
        None
    );
}
