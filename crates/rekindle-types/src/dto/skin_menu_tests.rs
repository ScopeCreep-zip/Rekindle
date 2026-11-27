use super::{EngineMenu, MenuRegistry};

/// Every engine menu has Rekindle items or is a recorded feature gap.
#[test]
fn every_engine_menu_is_populated_or_a_recorded_gap() {
    let reg = MenuRegistry::rekindle();
    for menu in EngineMenu::ALL {
        let gap = MenuRegistry::FEATURE_GAPS.contains(&menu);
        assert_eq!(reg.items(menu).is_empty(), gap, "{menu:?}");
        assert_eq!(reg.menus.iter().filter(|m| m.menu == menu).count(), 1);
    }
}

/// Plan SK10: a pack that declares a link gets "<pack>'s website" last
/// in Help, running that link; a pack without one gets Rekindle's menus.
#[test]
fn a_packs_link_is_served_in_the_help_menu() {
    use super::{PackUrl, SkinAction};
    let site = PackUrl {
        url: "http://www.totalchaos.org.uk/".to_owned(),
    };
    let reg = MenuRegistry::for_pack("Total Chaos", Some(&site));
    let last = reg.items(EngineMenu::Help).last().unwrap();
    assert_eq!(last.label, "Total Chaos's website");
    assert_eq!(last.run, SkinAction::LaunchUrl(site));
    let plain = MenuRegistry::rekindle();
    assert_eq!(
        reg.items(EngineMenu::Help).len(),
        plain.items(EngineMenu::Help).len() + 1
    );
    assert_eq!(MenuRegistry::for_pack("Onyx", None), plain);
}

/// Lead decision 7: only the file menu is renamed ("Xfire" → "Rekindle").
#[test]
fn only_the_file_menu_title_is_renamed() {
    for menu in EngineMenu::ALL {
        assert_eq!(menu.title().renamed(), menu == EngineMenu::File, "{menu:?}");
    }
    assert_eq!(EngineMenu::File.title().ours, "Rekindle");
}
