use super::*;

#[test]
fn every_base_has_the_token_skin_grounds() {
    assert_eq!(grounds(SkinBase::Dark).len(), GROUNDS.len());
    assert_eq!(grounds(SkinBase::Light).len(), GROUNDS.len());
}

#[test]
fn a_colour_that_reads_is_kept() {
    assert_eq!(readable(0x00ff_ffff, SkinBase::Dark, None), 0x00ff_ffff);
    assert_eq!(readable(0x0000_0000, SkinBase::Light, None), 0);
}

#[test]
fn the_shown_skin_grounds_join_its_base_only() {
    // A shown dark skin with a lighter ground (#404040; the token
    // skin's are #0a-#1e): #c0c0c0 reads on the token skin's grounds,
    // and a pack the device does not show (XGlow's #494949 buddy list)
    // does not move it, but it is 3.5:1 on its tag over #404040, so it
    // moves; the light base's colour is not touched by a dark skin.
    let colour = 0x00c0_c0c0;
    assert_eq!(readable(colour, SkinBase::Dark, None), colour);
    let shown = SkinGrounds {
        base: SkinBase::Dark,
        grounds: vec![[0x40; 3]],
    };
    let out = readable(colour, SkinBase::Dark, Some(&shown));
    assert_ne!(out, colour);
    let ground = tag(unpack(colour), [0x40; 3]);
    assert!(colour::contrast(unpack(out), ground) >= TEXT_RATIO);
    assert_eq!(
        readable(colour, SkinBase::Light, Some(&shown)),
        readable(colour, SkinBase::Light, None)
    );
}

#[test]
fn a_colour_that_does_not_read_moves_until_it_does_on_every_ground() {
    // #3498db on white is 3.2:1 (the axe finding on the light base).
    for (base, colour) in [
        (SkinBase::Light, 0x0034_98db),
        (SkinBase::Dark, 0x0020_2080),
    ] {
        let out = readable(colour, base, None);
        assert_ne!(out, colour, "{base:?}");
        for g in grounds(base) {
            let ground = tag(unpack(colour), *g);
            assert!(
                colour::contrast(unpack(out), ground) >= TEXT_RATIO,
                "{base:?} {g:?}"
            );
        }
    }
}
