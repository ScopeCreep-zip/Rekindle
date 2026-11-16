//! Synthetic sound packs only: every archive and WAV is built here.

use super::*;
use rekindle_skin::package;

/// A mono 8-bit 8 kHz PCM WAV of `n` samples.
fn wav(n: u32) -> Vec<u8> {
    let mut w = b"RIFF".to_vec();
    w.extend_from_slice(&(36 + n).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&[1, 0, 1, 0]); // PCM, mono
    w.extend_from_slice(&8000u32.to_le_bytes());
    w.extend_from_slice(&8000u32.to_le_bytes());
    w.extend_from_slice(&[1, 0, 8, 0]); // block align 1, 8 bits
    w.extend_from_slice(b"data");
    w.extend_from_slice(&n.to_le_bytes());
    w.extend((0..n).map(|i| u8::try_from(i % 256).unwrap()));
    w
}

/// A stored zip of `files`.
fn zip(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, data) in files {
        let offset = u32::try_from(out.len()).unwrap();
        let crc = crc32fast::hash(data);
        let size = u32::try_from(data.len()).unwrap();
        let name_len = u16::try_from(name.len()).unwrap();
        let fields = |v: &mut Vec<u8>| {
            v.extend_from_slice(&[0; 8]); // flags, method, time, date
            v.extend_from_slice(&crc.to_le_bytes());
            v.extend_from_slice(&size.to_le_bytes());
            v.extend_from_slice(&size.to_le_bytes());
            v.extend_from_slice(&name_len.to_le_bytes());
        };
        out.extend_from_slice(&[0x50, 0x4b, 3, 4, 20, 0]);
        fields(&mut out);
        out.extend_from_slice(&[0; 2]);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(&[0x50, 0x4b, 1, 2, 20, 0, 20, 0]);
        fields(&mut central);
        central.extend_from_slice(&[0; 12]); // extra, comment, disk, attributes
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let count = u16::try_from(files.len()).unwrap();
    let cd_at = u32::try_from(out.len()).unwrap();
    let cd_len = u32::try_from(central.len()).unwrap();
    out.extend_from_slice(&central);
    out.extend_from_slice(&[0x50, 0x4b, 5, 6, 0, 0, 0, 0]);
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&cd_len.to_le_bytes());
    out.extend_from_slice(&cd_at.to_le_bytes());
    out.extend_from_slice(&[0; 2]);
    out
}

const INFO: &str = "<?xml version=\"1.0\"?><!DOCTYPE SkinDescription SYSTEM \"x.dtd\">\
    <SkinDescription><Name>t</Name><LongName>Test sound pack</LongName></SkinDescription>";

fn pack(sounds_xml: &str) -> Vec<u8> {
    zip(&[
        ("sounds.xml", format!("\u{feff}{sounds_xml}").into_bytes()),
        ("soundsinfo.xml", INFO.as_bytes().to_vec()),
        ("recv.wav", wav(800)),
        ("on.wav", wav(900)),
        ("off.wav", wav(1000)),
        ("ring.wav", wav(1100)),
        ("back.wav", wav(1200)),
        ("end.wav", wav(1300)),
    ])
}

const SOUNDS: &str = "<!DOCTYPE XfireSounds SYSTEM \"xfiresounds.dtd\"><Sounds>\
    <Sound type=\"Receive Message\">recv.wav</Sound><Sound type=\"Send Message\"></Sound>\
    <Sound type=\"Online\">on.wav</Sound><Sound type=\"Offline\">off.wav</Sound>\
    <Sound type=\"Incoming Call\">ring.wav</Sound><Sound type=\"Outgoing Call\">back.wav</Sound>\
    <Sound type=\"Call End\">end.wav</Sound><Sound type=\"Screenshot\">shot.wav</Sound></Sounds>";

#[test]
fn a_pack_becomes_a_validated_bank_with_every_cue() {
    let bank = import("t", &pack(SOUNDS)).unwrap();
    assert_eq!(bank.long_name, "Test sound pack");
    let p = package::read(&bank.canonical.bytes).unwrap();
    for cue in SoundCue::ALL {
        assert!(
            p.get(&format!("sounds/{}.wav", cue.id())).is_some(),
            "{}",
            cue.id()
        );
    }
    assert_eq!(p.get("sounds/mention.wav"), p.get("sounds/message.wav"));
    assert_eq!(
        bank.dropped,
        [
            ("Send Message".to_owned(), String::new()),
            ("Screenshot".to_owned(), "shot.wav".to_owned())
        ]
    );
    assert_eq!(
        import("t", &pack(SOUNDS)).unwrap().canonical,
        bank.canonical
    );
}

#[test]
fn a_bad_pack_is_refused() {
    let entity = SOUNDS.replace("recv.wav</Sound>", "&ext;</Sound>");
    assert!(import("t", &pack(&entity)).is_err(), "entity reference");
    let missing = SOUNDS.replace("on.wav", "gone.wav");
    assert!(import("t", &pack(&missing)).is_err(), "missing file");
    let no_call = SOUNDS.replace("Incoming Call", "Nothing");
    assert!(import("t", &pack(&no_call)).is_err(), "no Incoming Call");
    let mut corrupt = pack(SOUNDS);
    // Only the CRC tells: the changed byte is in soundsinfo.xml's unread <Name>.
    let at = corrupt.windows(9).position(|w| w == b"<Name>t</").unwrap();
    corrupt[at + 6] = b'u';
    assert!(import("t", &corrupt).is_err(), "CRC mismatch");
}
