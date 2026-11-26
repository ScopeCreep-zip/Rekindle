use super::*;
use crate::{builtin, validate};

const H: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn only_a_package_address_and_one_file_parse() {
    assert_eq!(
        parse_path(&format!("/{H}/tokens.css")),
        Some((H, File::Stylesheet))
    );
    assert_eq!(
        parse_path(&format!("/{H}/{H}.gif")),
        Some((
            H,
            File::Image {
                hash: H,
                ext: "gif"
            }
        ))
    );
    for bad in [
        format!("/{H}/../{H}/tokens.css"),
        format!("/{H}/x/tokens.css"),
        format!("/../{H}/tokens.css"),
        format!("/{H}/%2e%2e/tokens.css"),
        format!("/{H}/{H}.svg"),
        format!("/{H}/{H}.png.png"),
        format!("/{}/tokens.css", H.to_uppercase()),
        format!("{H}/tokens.css"),
        format!("/{H}/manifest.json"),
        format!("/{H}"),
        String::new(),
    ] {
        assert_eq!(parse_path(&bad), None, "{bad}");
    }
}

#[test]
fn every_builtin_lists_each_image_its_stylesheet_draws_once() {
    for pack in &builtin::PACKS {
        let c = validate::run(pack.bytes).unwrap();
        let a = c.address();
        let applied = stylesheet(&c.bytes, &a).unwrap();
        assert!(!applied.images.is_empty(), "{}", pack.id);
        let shared: std::collections::BTreeSet<&str> =
            applied.images.iter().map(String::as_str).collect();
        assert_eq!(shared.len(), applied.images.len(), "{} twice", pack.id);
        let mut listed = shared.clone();
        for shell in &applied.shells {
            let own: std::collections::BTreeSet<&str> =
                shell.images.iter().map(String::as_str).collect();
            assert_eq!(own.len(), shell.images.len(), "{} twice", pack.id);
            assert!(own.is_disjoint(&shared), "{} shares tree art", pack.id);
            listed.extend(own);
        }
        for href in &listed {
            let path = href.strip_prefix(origin()).expect("under the origin");
            assert!(parse_path(path).is_some(), "{href} is no served file");
            assert!(applied.css.contains(&format!("url(\"{href}\")")), "{href}");
        }
        for drawn in applied.css.split("url(\"").skip(1) {
            let href = drawn.split('"').next().unwrap();
            assert!(listed.contains(href), "{} draws {href} unlisted", pack.id);
        }
    }
}

/// Plan SK4: the shared list a window without a tree (Settings)
/// decodes holds no tree art, each tree's art is on its own shell, and
/// no image is lost (the union is the whole sheet's list).
#[test]
fn a_trees_art_is_served_on_its_shell_and_none_is_lost() {
    let symbiosis = builtin::PACKS.iter().find(|p| p.id == "symbiosis").unwrap();
    for pack in &builtin::PACKS {
        let c = validate::run(pack.bytes).unwrap();
        let a = c.address();
        let applied = stylesheet(&c.bytes, &a).unwrap();
        let whole: std::collections::BTreeSet<String> =
            sheet_images(&applied.css, &format!("{}/{a}", origin()))
                .into_iter()
                .collect();
        let mut union: std::collections::BTreeSet<String> =
            applied.images.iter().cloned().collect();
        for shell in &applied.shells {
            union.extend(shell.images.iter().cloned());
        }
        assert_eq!(union, whole, "{}", pack.id);
        let p = package::read(&c.bytes).unwrap();
        let m = manifest::served(&p).unwrap();
        let base = format!("{}/{a}", origin());
        let (_, trees) = shape::frame_parts(&m, &files(&p), &base).unwrap();
        for href in &applied.images {
            let outside = applied.css.matches(&format!("url(\"{href}\")")).count()
                > trees
                    .iter()
                    .map(|t| t.css.matches(&format!("url(\"{href}\")")).count())
                    .sum::<usize>();
            assert!(outside, "{}: {href} is tree art only", pack.id);
        }
        if pack.id == symbiosis.id {
            let art: usize = applied.shells.iter().map(|s| s.images.len()).sum();
            assert!(art > applied.images.len(), "{}", pack.id);
        }
    }
}

/// Plan SK4: a tree-laid window's shell serves its minimum with the
/// frame's outsets, the value the desktop's fit applies
/// ([`shape::min_window`]); Symbiosis's main window is 368 wide.
#[test]
fn a_shell_serves_its_windows_minimum_with_the_outsets() {
    let pack = builtin::PACKS.iter().find(|p| p.id == "symbiosis").unwrap();
    let c = validate::run(pack.bytes).unwrap();
    let applied = stylesheet(&c.bytes, &c.address()).unwrap();
    let p = package::read(&c.bytes).unwrap();
    let m = manifest::served(&p).unwrap();
    let main = &m.windows[&rekindle_types::dto::skin::SkinWindow::Main];
    let shell = applied
        .shells
        .iter()
        .find(|s| s.window == rekindle_types::dto::skin::SkinWindow::Main && s.popup.is_none())
        .unwrap();
    assert_eq!(shell.min_width, 368);
    assert_eq!(
        Some((shell.min_width, shell.min_height)),
        shape::min_window(main)
    );
    let o = main.outsets;
    assert_eq!(shell.min_width, main.min_width + o.left + o.right);
    assert_eq!(shell.min_height, main.min_height + o.top + o.bottom);
}

#[test]
fn the_builtin_serves_its_stylesheet_and_its_images_by_address() {
    let c = validate::run(builtin::PACKS[0].bytes).unwrap();
    let a = c.address();
    let applied = stylesheet(&c.bytes, &a).unwrap();
    assert!(applied.css.starts_with("@layer skin {"));
    let prefix = format!("{}/{a}/", origin());
    let href = applied
        .css
        .split("url(\"")
        .nth(1)
        .and_then(|r| r.split('"').next())
        .expect("a region image");
    let name = href
        .strip_prefix(&prefix)
        .expect("served under the package");
    let path = format!("/{a}/{name}");
    let (_, file) = parse_path(&path).unwrap();
    let body = body(&c.bytes, &a, file).unwrap().expect("the image");
    assert!(body.content_type.starts_with("image/"));
    let missing = File::Image {
        hash: H,
        ext: "png",
    };
    assert_eq!(super::body(&c.bytes, &a, missing).unwrap(), None);
}

/// Every built-in serves a stylesheet with balanced braces,
/// parentheses and strings and no comment. Its time is held in its
/// own test binary (`tests/serve_budget.rs`), and the title's column
/// work, the regression that budget caught, by an operation count
/// (`regions/title_tests.rs`).
#[test]
fn every_builtin_validates_and_serves_a_balanced_sheet() {
    for pack in &builtin::PACKS {
        let c = validate::run(pack.bytes).unwrap_or_else(|e| panic!("{}: {e:?}", pack.id));
        let sheet = stylesheet(&c.bytes, &c.address()).expect("a stylesheet");
        let count = |ch: char| sheet.css.matches(ch).count();
        assert_eq!(count('{'), count('}'), "{}: braces", pack.id);
        assert_eq!(count('('), count(')'), "{}: parentheses", pack.id);
        assert_eq!(count('"') % 2, 0, "{}: an open string", pack.id);
        assert!(!sheet.css.contains("/*"), "{}: a comment", pack.id);
    }
}

/// Design §2.6: a pressed menu's items come from the served shell, so
/// every shell a built-in serves carries Rekindle's menu registry for
/// that pack (plan SK10: with "<pack>'s website" in Help when the pack
/// declares a link, as Total Chaos's logo does; Rekindle's menus
/// unchanged otherwise).
#[test]
fn a_served_shell_carries_the_registrys_menus() {
    use rekindle_types::dto::skin_action::EngineMenu;
    let registry = MenuRegistry::rekindle();
    assert!(!registry.menus.is_empty());
    let mut linked = Vec::new();
    for pack in &builtin::PACKS {
        let c = validate::run(pack.bytes).unwrap();
        let m = manifest::served(&package::read(&c.bytes).unwrap()).unwrap();
        let want = MenuRegistry::for_pack(&m.name, pack_link(&m).as_ref());
        let sheet = stylesheet(&c.bytes, &c.address()).unwrap();
        assert!(!sheet.shells.is_empty(), "{}", pack.id);
        for shell in &sheet.shells {
            assert_eq!(shell.menus, want, "{} {:?}", pack.id, shell.window);
        }
        let help = want.items(EngineMenu::Help);
        let site = help.iter().find(|i| i.label.ends_with("'s website"));
        if let Some(site) = site {
            linked.push((pack.id, site.label.clone(), site.run.clone()));
        } else {
            assert_eq!(want, registry, "{}: a menu without a link", pack.id);
        }
    }
    // The real set (design `br`): Total Chaos's `TCLink` and Space
    // Cowboy's `SCLogo`; the packs whose logo ran xfire.com serve none.
    let mut ids: Vec<&str> = linked.iter().map(|(id, ..)| *id).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["spacecowboy", "total-chaos"]);
    for (id, _, run) in &linked {
        let SkinAction::LaunchUrl(u) = run else {
            panic!("{id}: the website item runs {run:?}");
        };
        assert!(!crate::skin_tree::brand::is_xfire_url(&u.url), "{id}");
    }
    let chaos = linked.iter().find(|(id, ..)| *id == "total-chaos");
    let (_, label, run) = chaos.expect("Total Chaos's Help menu serves its website");
    assert_eq!(label, "Total Chaos's website");
    assert_eq!(
        *run,
        SkinAction::LaunchUrl(PackUrl {
            url: "http://www.totalchaos.org.uk/".to_owned()
        })
    );
}

#[test]
fn only_built_ins_are_immutable() {
    assert!(Cache::Immutable.header().contains("immutable"));
    assert_eq!(Cache::NoStore.header(), "no-store");
}
