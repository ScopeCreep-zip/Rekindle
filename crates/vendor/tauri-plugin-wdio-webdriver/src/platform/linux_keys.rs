//! Route a test key to the selected GTK webview through the normal GTK event path.

use glib::translate::{ToGlibPtr, ToGlibPtrMut};
use gtk::gdk::{self, keys::constants, EventType, ModifierType};
use gtk::prelude::*;

use super::{ModifierState, NativeKey};

pub(super) fn dispatch(
    webview: &webkit2gtk::WebView,
    key: NativeKey,
    down: bool,
    modifiers: ModifierState,
) -> Result<(), &'static str> {
    if !webview.has_focus() {
        return Err("The selected GTK webview does not have keyboard focus");
    }
    let window = webview
        .window()
        .ok_or("The selected GTK webview has no native window")?;
    let display = window.display();
    let keyboard = display
        .default_seat()
        .and_then(|seat| seat.keyboard())
        .ok_or("The selected GTK display has no keyboard")?;
    // GTK can reroute window events while another widget owns a keyboard grab.
    if display.device_is_grabbed(&keyboard) || webview.device_is_shadowed(&keyboard) {
        return Err("A GTK keyboard grab prevents delivery to the selected webview");
    }
    let base = *match key {
        NativeKey::Tab => constants::Tab,
        NativeKey::Enter => constants::Return,
        NativeKey::NumpadEnter => constants::KP_Enter,
        NativeKey::Escape => constants::Escape,
        NativeKey::Space => constants::space,
        NativeKey::Left => constants::Left,
        NativeKey::Right => constants::Right,
        NativeKey::Up => constants::Up,
        NativeKey::Down => constants::Down,
        NativeKey::ShiftLeft => constants::Shift_L,
        NativeKey::ShiftRight => constants::Shift_R,
    };
    let keymap = gdk::Keymap::for_display(&display).ok_or("The GTK keyboard has no keymap")?;
    let entries = keymap.entries_for_keyval(base);
    let entry = entries
        .first()
        .ok_or("The GTK keymap has no entry for this key")?;
    let keyval = if key == NativeKey::Tab && modifiers.shift {
        *constants::ISO_Left_Tab
    } else {
        base
    };
    let hardware =
        u16::try_from(entry.keycode()).map_err(|_| "The GDK keycode exceeds the event field")?;
    let group =
        u8::try_from(entry.group()).map_err(|_| "The GDK key group exceeds the event field")?;
    let time = (glib::monotonic_time() / 1_000).rem_euclid(i64::from(u32::MAX) + 1);
    let time = u32::try_from(time).map_err(|_| "The GDK event timestamp is invalid")?;
    let kind = if down {
        EventType::KeyPress
    } else {
        EventType::KeyRelease
    };
    let mut event = gdk::Event::new(kind);
    event.set_device(Some(&keyboard));
    // SAFETY: GDK allocated this union with the matching key-event variant. It
    // owns one reference to the attached window and releases it with the event.
    unsafe {
        let event_key = &mut (*event.to_glib_none_mut().0).key;
        event_key.window = window.to_glib_full();
        event_key.send_event = 1;
        event_key.time = time;
        event_key.state = modifier_mask(modifiers).bits();
        event_key.keyval = keyval;
        event_key.hardware_keycode = hardware;
        event_key.group = group;
        event_key.is_modifier =
            u32::from(matches!(key, NativeKey::ShiftLeft | NativeKey::ShiftRight));
    }
    gtk::main_do_event(&mut event);
    Ok(())
}

fn modifier_mask(modifiers: ModifierState) -> ModifierType {
    let mut mask = ModifierType::empty();
    mask.set(ModifierType::SHIFT_MASK, modifiers.shift);
    mask.set(ModifierType::CONTROL_MASK, modifiers.ctrl);
    mask.set(ModifierType::MOD1_MASK, modifiers.alt);
    mask.set(ModifierType::META_MASK, modifiers.meta);
    mask
}
