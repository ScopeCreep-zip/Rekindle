//! The INI era's chat window (plan SK17-3): the file the skin file's
//! `ChatSkin=` names (`combat.ini:9` `ChatSkin=CombatChatSkin`,
//! `Alloy.ini:11`, `SimpleX.ini`, `Xglow.ini`), read as the XML era's tile
//! tree by [`super::title::tree`] from its `[MainWindow]`, then laid as a
//! fan pack's chat window: the frame from the nine tiles that root lays,
//! fitted to the pack's chat content region
//! ([`super::super::xfire_xml::chat_frame::from_ini_tiles`]), and the
//! component tree with the chat window's roles (plan SK17-2,
//! [`crate::skin_tree::chat_roles`]: title bar, logo, chat title, window
//! buttons, menu row, slots). A file without a `[MainWindow]`, or whose
//! root lays no complete frame, serves no chat window: the shell keeps
//! its own layout.

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{SkinWindow, WindowLayout};

use super::super::tree::Node;
use super::super::xfire_xml::chat_frame;
use super::super::xfire_xml::controls::StateTally;
use super::super::xfire_xml::refs::Ref;
use super::super::xfire_xml::surfaces::Window;
use super::super::{Control, ImportError, Tree};
use super::ini::{Ini, Section};
use super::{controls, count_dropped, file_name, ini_refs, lookup, parse_file, popups, title};

/// The chat file's root section.
const ROOT: &str = "MainWindow";

/// A window's layout and its package images.
pub type Built = (WindowLayout, BTreeMap<String, Vec<u8>>);

/// The chat window `chat` (a chat skin file) lays, with `popups` (the popup
/// files its launchers open, [`super::popups`]); `None` without a root or
/// a complete frame.
///
/// # Errors
/// What `decode` returns, a tile the role tables lack, an anchor that does
/// not resolve.
pub fn window(
    chat: &Ini,
    popups: Vec<Node>,
    decode: impl Fn(&str, Option<&str>) -> Result<RgbaImage, ImportError>,
) -> Result<Option<Built>, ImportError> {
    let Some((layout, components)) = title::tree(chat, ROOT) else {
        return Ok(None);
    };
    let mut all = vec![components];
    all.extend(popups);
    let Some(mut built) = chat_frame::from_ini_tiles((&layout, &all), ROOT, &decode)? else {
        return Ok(None);
    };
    crate::skin_tree::import::add(SkinWindow::Chat, &layout, &all, &decode, &[], &mut built)?;
    Ok(Some(built))
}

/// What reading the chat files adds to the skin's read.
pub struct Sink<'s, 't> {
    pub controls: &'s mut Vec<Control>,
    pub tally: &'s mut StateTally,
    /// The files read, for the `NOTICE`'s unread list.
    pub read_files: &'s mut Vec<&'t str>,
    /// Every colour reference ([`ini_refs`]).
    pub found: &'s mut Vec<Ref>,
    pub dropped: &'s mut BTreeMap<&'static str, usize>,
}

/// Reads the chat and group chat skin files `main` (the skin file
/// `skin_key`'s `[MainWindow]`) names by `ChatSkin=`/`GroupChatSkin=`:
/// their colour references, their painted control slots, the popups the
/// chat file launches, and the chat window it lays ([`window`]). A skin
/// that names a file it does not ship has no chat surfaces; a shipped file
/// is read like the others (a malformed one refuses).
///
/// # Errors
/// A malformed chat file, a launched popup file the pack lacks, what
/// [`window`] or [`controls::read_file`] refuses.
pub fn read_files<'t>(
    tree: &'t Tree,
    (skin_key, main): (&str, &Section),
    images: &controls::Images<'_>,
    decode: &impl Fn(&str, Option<&str>) -> Result<RgbaImage, ImportError>,
    sink: &mut Sink<'_, 't>,
) -> Result<Option<Built>, ImportError> {
    let mut chat_window = None;
    for (key, window_kind) in [
        ("ChatSkin", Window::Chat),
        ("GroupChatSkin", Window::GroupChat),
    ] {
        let Some(name) = main.get(key) else { continue };
        let file = file_name(skin_key, key, name)?;
        if lookup(tree, &file).is_err() {
            continue;
        }
        let (chat_key, chat) = parse_file(tree, &file)?;
        ini_refs(&chat, sink.found);
        let mut laid = false;
        if window_kind == Window::Chat {
            let (nodes, keys) = popups(tree, chat_key, &chat)?;
            sink.read_files.extend(keys);
            chat_window = window(&chat, nodes, decode)?;
            laid = chat_window.is_some();
        }
        let sections = controls::read_file(
            &chat,
            images,
            window_kind,
            (&mut *sink.controls, &mut *sink.tally),
        )?;
        if !sections.is_empty() {
            count_dropped(&chat, (None, &sections), sink.dropped);
        }
        // A file that lays the chat window or fills a slot was read.
        if laid || !sections.is_empty() {
            sink.read_files.push(chat_key);
        }
    }
    Ok(chat_window)
}
