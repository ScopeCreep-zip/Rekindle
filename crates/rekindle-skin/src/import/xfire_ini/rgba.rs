//! A theme's `ColorGroup.<NAME>=` value as Xfire read it (SKC2; ruling 3
//! of `design-skin-catalogue-2026-11-26.md`): [`parse_rgba`], after a
//! normalisation only where every reading of the written value gives one
//! colour, each one made described in the `NOTICE`:
//!
//! * a trailing empty field after the fourth, `RGBA(255,255,255,0,)`:
//!   every reader of four integers reads `(255,255,255,0)`;
//! * a channel written with a zero fraction, `RGBA(159,171,205.0)`: it is
//!   205 whether `.0` is a fraction or a mistyped `,0` alpha byte;
//! * the tuple without its `RGBA` name, `(0,0,0,0)`;
//! * a non-zero fourth (alpha) byte, `RGBA(214,214,214,255)` (FluidX's
//!   `ScrollbarColor`) or `-255` (msn's `StatusBackgroundCG`): the byte is
//!   unused, COLORREF-style (GDI ignores a COLORREF's high byte), so it is
//!   read as 0 (lead ruling, 2026-11-26, replacing the refusal). A fourth
//!   field outside a channel's −255..=255 is no byte and still refuses.
//!
//! A negative channel on an absolute colour (a COLORREF byte cast wraps
//! −230 to 26, a clamp makes it 0) has readings that differ, and nothing
//! shows how Xfire read it, so it still refuses.

use super::super::xfire_xml::colours::parse_rgba;
use super::super::ImportError;

/// `value` read as [`parse_rgba`] reads it after the normalisations above;
/// each one made is pushed to `notes`.
///
/// # Errors
/// What [`parse_rgba`] returns, naming the value as written.
pub(super) fn read(
    name: &str,
    value: &str,
    notes: &mut Vec<String>,
) -> Result<[i16; 3], ImportError> {
    let written = value.trim();
    let mut what = Vec::new();
    let inner = match written.strip_prefix("RGBA(") {
        Some(rest) => rest,
        None => match written.strip_prefix('(') {
            Some(rest) => {
                what.push("the RGBA name added");
                rest
            }
            None => return parse_rgba(name, value),
        },
    };
    let Some(inner) = inner.strip_suffix(')') else {
        return parse_rgba(name, value);
    };
    let mut fields: Vec<&str> = inner.split(',').map(str::trim).collect();
    if fields.len() == 5 && fields[4].is_empty() {
        fields.pop();
        what.push("a trailing empty field dropped");
    }
    for f in &mut fields {
        if let Some((int, frac)) = f.split_once('.') {
            if !int.is_empty() && !frac.is_empty() && frac.bytes().all(|b| b == b'0') {
                *f = int;
                what.push("a zero fraction dropped");
            }
        }
    }
    if let Some(a) = fields.get_mut(3) {
        if a.parse::<i32>()
            .is_ok_and(|v| v != 0 && (-255..=255).contains(&v))
        {
            *a = "0";
            what.push("a non-zero alpha byte ignored (unused, COLORREF-style)");
        }
    }
    let canonical = format!("RGBA({})", fields.join(","));
    let rgb = parse_rgba(name, &canonical).map_err(|e| match e {
        ImportError::BadColour { .. } => ImportError::BadColour {
            name: name.to_owned(),
            value: value.to_owned(),
        },
        e => e,
    })?;
    if !what.is_empty() {
        notes.push(format!(
            "ColorGroup.{name}={written} read as {canonical} ({})",
            what.join("; ")
        ));
    }
    Ok(rgb)
}
