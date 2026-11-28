use rekindle_types::dto::skin::WindowLayout;
use rekindle_types::dto::skin_action::SkinAction;

use super::{png, read, skin_ini, tree, FOLDER};

fn background_at(z: i16) -> WindowLayout {
    let mut files = tree();
    let ini = skin_ini().replacen(
        "[Top]\r\n",
        &format!(
            "Window2.Name=MAINBG\r\nWindow2.Z={z}\r\nWindow2.X=5\r\nWindow2.Y=5\r\n\
         Window2.ResizeX=100\r\nWindow2.ResizeY=100\r\nWindow2.IndentRight=5\r\n\
         Window2.IndentBottom=5\r\n[Top]\r\n"
        ),
        1,
    );
    files.insert(
        "Fixture.ini".into(),
        format!("{ini}[MAINBG]\r\nClass=Tile\r\nGraphics.middle=main\\background.png\r\nAction.lbtndown=AppDragWindow\r\n").into_bytes(),
    );
    files.insert("main/background.png".into(), png(false));
    let imported = read(&files, FOLDER).unwrap();
    imported.main.unwrap().0
}

#[test]
fn a_stretched_background_draws_once_and_retains_its_drag_target() {
    let layout = background_at(-1);
    assert!(layout
        .tiles
        .iter()
        .any(|tile| tile.image.ends_with("/bg-mainbg.png")));
    let tree = layout.tree.unwrap();
    let background = tree.root.iter().find(|node| node.id == "MAINBG").unwrap();
    assert!(
        background.art.is_none(),
        "the underlay is already painted in the frame"
    );
    assert!(background
        .triggers
        .iter()
        .any(|trigger| trigger.run == SkinAction::DragWindow));
}

#[test]
fn a_stretched_background_at_zero_z_keeps_its_authored_tree_layer() {
    let layout = background_at(0);
    assert!(!layout
        .tiles
        .iter()
        .any(|tile| tile.image.ends_with("/bg-mainbg.png")));
    let tree = layout.tree.unwrap();
    let background = tree.root.iter().find(|node| node.id == "MAINBG").unwrap();
    assert!(background.art.is_some());
    assert!(background
        .triggers
        .iter()
        .any(|trigger| trigger.run == SkinAction::DragWindow));
}
