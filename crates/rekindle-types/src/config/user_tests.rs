use super::*;

#[test]
fn the_default_file_is_valid() {
    assert!(UserConfig::default().validate().is_ok());
    let parsed: UserConfig = toml::from_str("").unwrap();
    assert!(parsed.validate().is_ok());
}

#[test]
fn unknown_sections_and_keys_are_errors() {
    assert!(toml::from_str::<UserConfig>("[global]\nnamespace = \"x\"").is_err());
    assert!(toml::from_str::<UserConfig>("[tui]\nthme = \"x\"").is_err());
    assert!(toml::from_str::<UserConfig>("[network]\ngossip = 3").is_err());
}

#[test]
fn sections_validate() {
    let mut cfg = UserConfig::default();
    cfg.tui.frame_rate = 0.0;
    assert_eq!(cfg.validate().unwrap_err().key(), "tui.frame_rate");
    let cfg = UserConfig {
        config_version: 2,
        ..UserConfig::default()
    };
    assert_eq!(cfg.validate().unwrap_err().key(), "config_version");
}
