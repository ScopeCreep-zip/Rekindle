//! Steam library fixtures in each OS's layout (plan E7.15 IV3). Nothing is
//! launched and no real Steam directory is read.

use super::*;

const LIBRARY_FOLDERS: &str = r#""libraryfolders"
{
	"0"
	{
		"path"		"LIBRARY_A"
		"label"		""
		"apps"
		{
			"570"		"123"
		}
	}
	"1"
	{
		"path"		"LIBRARY_B"
	}
}
"#;

fn manifest(app_id: u32) -> String {
    format!(
        "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app_id}\"\n\t\"name\"\t\t\"Game {app_id}\"\n\t\"StateFlags\"\t\t\"4\"\n}}\n"
    )
}

fn db() -> GameDatabase {
    GameDatabase::from_json(
        r#"{"games":[
            {"id":10,"name":"A","process_names":["a"],"steam_app_id":570},
            {"id":11,"name":"B","process_names":["b"],"steam_app_id":440},
            {"id":12,"name":"B2","process_names":["b2"],"steam_app_id":440},
            {"id":13,"name":"Played","process_names":["p"]}
        ]}"#,
    )
    .unwrap()
}

/// Build Steam under `layout`'s first root inside `home`, with one more
/// library elsewhere; return the roots to scan.
fn fixture(home: &Path, layout: SteamLayout) -> Vec<PathBuf> {
    let program_files = home.join("Program Files (x86)");
    let roots = layout.roots(home, Some(&program_files));
    let root = &roots[0];
    let extra = home.join("Games/SteamLibrary");
    std::fs::create_dir_all(root.join("steamapps")).unwrap();
    std::fs::create_dir_all(extra.join("steamapps")).unwrap();
    // A Windows path in the file is escaped (`D:\\Games`): the parser
    // unescapes it, so write the escaped form of every path.
    let escape = |p: &Path| p.to_string_lossy().replace('\\', "\\\\");
    let vdf = LIBRARY_FOLDERS
        .replace("LIBRARY_A", &escape(root))
        .replace("LIBRARY_B", &escape(&extra));
    std::fs::write(root.join("steamapps/libraryfolders.vdf"), vdf).unwrap();
    std::fs::write(root.join("steamapps/appmanifest_570.acf"), manifest(570)).unwrap();
    std::fs::write(extra.join("steamapps/appmanifest_440.acf"), manifest(440)).unwrap();
    std::fs::write(extra.join("steamapps/notes.txt"), manifest(999)).unwrap();
    roots
}

#[test]
fn every_layout_finds_both_libraries() {
    for layout in [SteamLayout::MacOs, SteamLayout::Linux, SteamLayout::Windows] {
        let home = tempfile::tempdir().unwrap();
        let roots = fixture(home.path(), layout);
        let ids = steam_app_ids(&roots);
        assert_eq!(ids, BTreeSet::from([440, 570]), "{layout:?}");
    }
}

#[test]
fn layouts_name_each_os_steam_root() {
    let home = Path::new("/home/u");
    assert_eq!(
        SteamLayout::MacOs.roots(home, None),
        vec![PathBuf::from("/home/u/Library/Application Support/Steam")]
    );
    assert!(SteamLayout::Linux
        .roots(home, None)
        .contains(&PathBuf::from("/home/u/.local/share/Steam")));
    assert!(SteamLayout::Windows.roots(home, None).is_empty());
    assert_eq!(
        SteamLayout::Windows.roots(home, Some(Path::new("C:/PF86"))),
        vec![PathBuf::from("C:/PF86/Steam")]
    );
}

#[test]
fn keyvalues_values_are_unescaped() {
    let vdf = "\"libraryfolders\" { \"0\" { \"path\" \"D:\\\\Games\\\\Steam\" } }";
    assert_eq!(
        library_folders(vdf),
        vec![PathBuf::from("D:\\Games\\Steam")]
    );
    assert_eq!(manifest_app_id(&manifest(730)), Some(730));
    assert_eq!(manifest_app_id("\"AppState\" { \"appid\" \"x\" }"), None);
}

#[test]
fn installed_games_join_steam_and_played_minus_hidden() {
    let steam = BTreeSet::from([440, 570, 7]);
    assert_eq!(
        installed_games(&db(), &steam, &BTreeSet::new(), &[13], &[]),
        vec![10, 11, 12, 13]
    );
    assert_eq!(
        installed_games(&db(), &steam, &BTreeSet::new(), &[13], &[11, 13]),
        vec![10, 12]
    );
    assert!(installed_games(&db(), &BTreeSet::new(), &BTreeSet::new(), &[], &[]).is_empty());
}

#[test]
fn a_missing_steam_root_is_empty() {
    let home = tempfile::tempdir().unwrap();
    assert!(steam_app_ids(&SteamLayout::Linux.roots(home.path(), None)).is_empty());
}

/// The OS's own Chess (plan E7.15 CH1, id 6200) is in the installed list
/// when its fixed path exists under the Host's system root, and not
/// before; Windows has no native copy.
#[test]
fn native_chess_is_installed_when_its_path_exists() {
    let db = GameDatabase::bundled();
    let root = tempfile::tempdir().unwrap();
    let apps = AppRoots {
        home: root.path().join("home"),
        program_files_x86: None,
        system: root.path().to_path_buf(),
    };
    for layout in [SteamLayout::MacOs, SteamLayout::Linux, SteamLayout::Windows] {
        assert!(
            native_installed(&db, layout, &apps).is_empty(),
            "{layout:?}"
        );
    }
    let contents = root.path().join("System/Applications/Chess.app/Contents");
    std::fs::create_dir_all(&contents).unwrap();
    std::fs::write(contents.join("Info.plist"), "com.apple.Chess").unwrap();
    std::fs::create_dir_all(root.path().join("usr/games")).unwrap();
    std::fs::write(root.path().join("usr/games/gnome-chess"), b"").unwrap();
    for layout in [SteamLayout::MacOs, SteamLayout::Linux] {
        let native = native_installed(&db, layout, &apps);
        assert_eq!(native, BTreeSet::from([6200]), "{layout:?}");
        assert_eq!(
            installed_games(&db, &BTreeSet::new(), &native, &[], &[]),
            vec![6200]
        );
    }
    assert!(native_installed(&db, SteamLayout::Windows, &apps).is_empty());
}

/// Chess is detected from each OS's process name: `Chess` (macOS) and
/// `gnome-chess` (Linux); GNOME Chess's engine child `hoichess` is not a
/// game of its own.
#[test]
fn chess_is_detected_from_each_os_process_name() {
    let db = GameDatabase::bundled();
    assert_eq!(db.lookup_by_process("Chess").map(|e| e.id), Some(6200));
    assert_eq!(
        db.lookup_by_process("gnome-chess").map(|e| e.id),
        Some(6200)
    );
    assert!(db.lookup_by_process("hoichess").is_none());
}
