//! Deliver an AppKit key event to the selected WKWebView's content responder.

use std::time::Duration;

use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType, NSView};
use objc2_foundation::{NSPoint, NSProcessInfo, NSString};
use objc2_web_kit::WKWebView;

use tauri::{Runtime, WebviewWindow};
use tokio::sync::oneshot;

use super::{ModifierState, NativeKey};
use crate::server::response::WebDriverErrorResponse;

pub(super) async fn dispatch_to_window<R: Runtime>(
    window: &WebviewWindow<R>,
    key: NativeKey,
    is_down: bool,
    modifiers: ModifierState,
    timeout_ms: u64,
) -> Result<(), WebDriverErrorResponse> {
    let (tx, rx) = oneshot::channel();
    window
        .with_webview(move |webview| {
            if tx.is_closed() {
                return;
            }
            // SAFETY: this callback runs on AppKit's thread with the live WKWebView.
            let accepted = unsafe {
                dispatch(
                    &*webview.inner().cast::<WKWebView>(),
                    key,
                    is_down,
                    modifiers,
                )
            };
            let _ = tx.send(accepted);
        })
        .map_err(|error| WebDriverErrorResponse::unknown_error(&error.to_string()))?;
    match tokio::time::timeout(Duration::from_millis(timeout_ms), rx).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(error))) => Err(WebDriverErrorResponse::element_not_interactable(error)),
        Ok(Err(_)) => Err(WebDriverErrorResponse::unknown_error(
            "Native key dispatch channel closed",
        )),
        Err(_) => Err(WebDriverErrorResponse::script_timeout()),
    }
}

fn dispatch(
    webview: &WKWebView,
    key: NativeKey,
    down: bool,
    modifiers: ModifierState,
) -> Result<(), &'static str> {
    let window = webview
        .window()
        .ok_or("The selected WKWebView has no native window")?;
    let responder = window
        .firstResponder()
        .ok_or("The selected AppKit window has no keyboard responder")?;
    if !responder
        .downcast_ref::<NSView>()
        .is_some_and(|view| view.isDescendantOf(webview))
    {
        return Err("The selected WKWebView does not own the keyboard responder");
    }
    let (code, characters) = key_binding(key, modifiers.shift);
    let is_modifier = matches!(key, NativeKey::ShiftLeft | NativeKey::ShiftRight);
    let kind = if is_modifier {
        NSEventType::FlagsChanged
    } else if down {
        NSEventType::KeyDown
    } else {
        NSEventType::KeyUp
    };
    let text = NSString::from_str(characters);
    let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        kind,
        NSPoint::new(0.0, 0.0),
        modifier_flags(modifiers),
        NSProcessInfo::processInfo().systemUptime(),
        window.windowNumber(),
        None,
        &text,
        &text,
        false,
        code,
    ).ok_or("AppKit did not construct the key event")?;
    if is_modifier {
        responder.flagsChanged(&event);
    } else if down {
        responder.keyDown(&event);
    } else {
        responder.keyUp(&event);
    }
    Ok(())
}

fn key_binding(key: NativeKey, shift: bool) -> (u16, &'static str) {
    match key {
        NativeKey::Tab if shift => (0x30, "\u{19}"),
        NativeKey::Tab => (0x30, "\t"),
        NativeKey::Enter => (0x24, "\r"),
        NativeKey::NumpadEnter => (0x4c, "\u{3}"),
        NativeKey::Escape => (0x35, "\u{1b}"),
        NativeKey::Space => (0x31, " "),
        NativeKey::Left => (0x7b, "\u{f702}"),
        NativeKey::Right => (0x7c, "\u{f703}"),
        NativeKey::Up => (0x7e, "\u{f700}"),
        NativeKey::Down => (0x7d, "\u{f701}"),
        NativeKey::ShiftLeft => (0x38, ""),
        NativeKey::ShiftRight => (0x3c, ""),
    }
}

fn modifier_flags(modifiers: ModifierState) -> NSEventModifierFlags {
    let mut flags = NSEventModifierFlags::empty();
    flags.set(NSEventModifierFlags::Shift, modifiers.shift);
    flags.set(NSEventModifierFlags::Control, modifiers.ctrl);
    flags.set(NSEventModifierFlags::Option, modifiers.alt);
    flags.set(NSEventModifierFlags::Command, modifiers.meta);
    flags
}
