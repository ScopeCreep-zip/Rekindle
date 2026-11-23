//! Xfire's sound packs (`sounds/defaults.zip`, `sounds/classic.zip`) as the
//! built-in banks `builtin:defaults` and `builtin:classic` (plan E7.7.1
//! "Built-in packs", E7.7.4).
//!
//! A bank is a `rekindle-skin/1` package like any other: the manifest's
//! `sounds` maps each [`SoundCue`] id to `sounds/<cue>.wav` (the entry the
//! E7.7.4 cue player fetches by cue name), each cue re-encoded by
//! `rekindle_skin::sounds::check`. The format requires one variant; a bank
//! carries one dark standard variant with an empty token file, so it
//! renders as the registry's defaults and is chosen for its sounds alone.
//! The package is built by `rekindle_skin::output::package` and then run
//! through `rekindle_skin::validate::run`, which must return the same bytes.
//!
//! `sounds.xml` (`<Sounds><Sound type="…">file.wav</Sound>…`) is read with
//! `quick-xml`: the DOCTYPE is skipped and never resolved, an entity
//! reference refuses the file.

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, ensure, Context, Result};
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use rekindle_skin::output::{self, Canonical};
use rekindle_skin::{sounds, validate};
use rekindle_types::dto::skin::{
    SkinBase, SkinContrast, SkinLicense, SkinManifest, SkinSource, SkinVariant, SoundCue,
    SKIN_FORMAT_MAJOR, SKIN_FORMAT_PREFIX,
};

/// Each cue and the Xfire sound type that voices it. Xfire had no mention
/// or busy sound: a mention plays its "Receive Message" sound (Xfire played
/// it for every incoming message, a message naming you included), and a
/// busy or declined call its "Call End" sound.
pub const CUES: [(SoundCue, &str); 7] = [
    (SoundCue::Message, "Receive Message"),
    (SoundCue::Mention, "Receive Message"),
    (SoundCue::SignOn, "Online"),
    (SoundCue::SignOff, "Offline"),
    (SoundCue::Call, "Incoming Call"),
    (SoundCue::Ringback, "Outgoing Call"),
    (SoundCue::Busy, "Call End"),
];

/// The token file of the bank's one variant.
const TOKENS: &str = "tokens.json";
/// `source.format` of a bank imported from an Xfire sound pack.
const SOURCE_FORMAT: &str = "xfire-sounds";

/// One imported bank.
pub struct Bank {
    pub canonical: Canonical,
    /// Each cue and the archive file it came from.
    pub cues: Vec<(SoundCue, String)>,
    /// Each Xfire sound type with no cue, and its file (`""` for none).
    pub dropped: Vec<(String, String)>,
    /// `soundsinfo.xml`'s `LongName`.
    pub long_name: String,
}

/// Each `element` of the XML `bytes` in document order: the value of its
/// attribute `attr` (`""` without one) and its trimmed text.
fn elements(
    file: &str,
    bytes: &[u8],
    element: &str,
    attr: Option<&str>,
) -> Result<Vec<(String, String)>> {
    let text = std::str::from_utf8(bytes).with_context(|| format!("{file}: not UTF-8"))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut reader = Reader::from_str(text);
    let mut out: Vec<(String, String)> = Vec::new();
    let mut inside = false;
    loop {
        match reader.read_event().map_err(|e| anyhow!("{file}: {e}"))? {
            Event::Start(e) if e.name().as_ref() == element => {
                let key = match attr {
                    Some(a) => e
                        .try_get_attribute(a)
                        .map_err(|x| anyhow!("{file}: {x}"))?
                        .with_context(|| format!("{file}: <{element}> without {a}"))?
                        .normalized_value(XmlVersion::Implicit1_0)
                        .map_err(|x| anyhow!("{file}: {x}"))?
                        .into_owned(),
                    None => String::new(),
                };
                out.push((key, String::new()));
                inside = true;
                ensure!(out.len() <= 256, "{file}: too many elements");
            }
            Event::Text(t) if inside => {
                let s = t.xml_content(XmlVersion::Implicit1_0);
                if let Some(last) = out.last_mut() {
                    last.1.push_str(s.trim());
                }
            }
            Event::End(e) if e.name().as_ref() == element => inside = false,
            Event::GeneralRef(_) => bail!("{file}: entity references are refused"),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

fn manifest(id: &str, name: &str, hash: String, cues: &[(SoundCue, String)]) -> SkinManifest {
    SkinManifest {
        format: format!("{SKIN_FORMAT_PREFIX}{SKIN_FORMAT_MAJOR}"),
        id: id.to_owned(),
        name: name.to_owned(),
        description: "Xfire's own sound pack, imported as a built-in sound bank; see the NOTICE \
                      beside the package."
            .to_owned(),
        license: SkinLicense::Text(
            "Xfire, Inc. sound pack (Xfire is defunct), used under the project owner's \
             2026-10-15 ruling. Not covered by Rekindle's MIT licence; provenance is in the \
             NOTICE beside the package."
                .to_owned(),
        ),
        source: Some(SkinSource {
            format: SOURCE_FORMAT.to_owned(),
            hash,
        }),
        credits: None,
        author: None,
        signature: None,
        variants: vec![SkinVariant {
            base: SkinBase::Dark,
            contrast: SkinContrast::Standard,
            tokens: TOKENS.to_owned(),
        }],
        regions: BTreeMap::new(),
        icons: BTreeMap::new(),
        sounds: cues
            .iter()
            .map(|(c, _)| {
                (
                    c.id().to_owned(),
                    format!("{}{}.wav", sounds::SOUND_DIR, c.id()),
                )
            })
            .collect(),
        windows: BTreeMap::new(),
    }
}

/// The archive `zip` (an Xfire sound pack) as the bank `id`.
///
/// # Errors
/// A malformed archive or XML, a missing or refused sound, or a package the
/// validator refuses or changes.
pub fn import(id: &str, zip: &[u8]) -> Result<Bank> {
    let files = super::zip::unpack(zip)?;
    let get = |f: &str| {
        files
            .get(f)
            .with_context(|| format!("{f} missing from the pack"))
    };
    let pairs = elements("sounds.xml", get("sounds.xml")?, "Sound", Some("type"))?;
    let types: BTreeMap<&str, &str> = pairs
        .iter()
        .map(|(t, f)| (t.as_str(), f.as_str()))
        .collect();
    ensure!(types.len() == pairs.len(), "sounds.xml names a type twice");
    let long_name = elements("soundsinfo.xml", get("soundsinfo.xml")?, "LongName", None)?
        .into_iter()
        .next()
        .map(|(_, v)| v)
        .context("soundsinfo.xml without LongName")?;
    let mut cues = Vec::new();
    let mut wavs = BTreeMap::new();
    for (cue, ty) in CUES {
        let file = types
            .get(ty)
            .with_context(|| format!("sounds.xml has no {ty:?}"))?;
        ensure!(!file.is_empty(), "sounds.xml gives {ty:?} no file");
        let entry = format!("{}{}.wav", sounds::SOUND_DIR, cue.id());
        let wav = sounds::check(&entry, get(file)?).map_err(|r| anyhow!("{file}: {r:?}"))?;
        wavs.insert(entry, wav);
        cues.push((cue, (*file).to_owned()));
    }
    let dropped = pairs
        .iter()
        .filter(|(t, _)| !CUES.iter().any(|(_, ty)| ty == t))
        .cloned()
        .collect();
    // Served names never say Xfire (design `br`): "Classic sound pack".
    let name = long_name;
    let m = manifest(id, &name, blake3::hash(zip).to_hex().to_string(), &cues);
    let tokens = BTreeMap::from([(TOKENS.to_owned(), BTreeMap::new())]);
    let built = output::package(&m, &tokens, &BTreeMap::new(), &wavs)
        .map_err(|e| anyhow!("{id}: package: {e:?}"))?;
    let canonical = validate::run(&built.bytes).map_err(|e| anyhow!("{id}: refused: {e:?}"))?;
    ensure!(
        canonical == built,
        "{id}: the validator changed the package"
    );
    Ok(Bank {
        canonical,
        cues,
        dropped,
        long_name,
    })
}

#[cfg(test)]
#[path = "sounds_tests.rs"]
mod tests;
