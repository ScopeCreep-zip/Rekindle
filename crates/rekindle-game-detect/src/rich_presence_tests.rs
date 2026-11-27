use super::*;

#[test]
fn parse_plus_connect() {
    let args: Vec<String> = vec!["cs2".into(), "+connect".into(), "192.168.1.1:27015".into()];
    let result = parse_connect_args(&args);
    assert_eq!(result, Some(("192.168.1.1".into(), 27015)));
}

#[test]
fn parse_dash_connect() {
    let args: Vec<String> = vec![
        "game.exe".into(),
        "-connect".into(),
        "10.0.0.1:27960".into(),
    ];
    let result = parse_connect_args(&args);
    assert_eq!(result, Some(("10.0.0.1".into(), 27960)));
}

#[test]
fn parse_equals_form() {
    let args: Vec<String> = vec!["game.exe".into(), "+connect=1.2.3.4:28000".into()];
    let result = parse_connect_args(&args);
    assert_eq!(result, Some(("1.2.3.4".into(), 28000)));
}

#[test]
fn parse_no_match() {
    let args: Vec<String> = vec!["game.exe".into(), "--fullscreen".into()];
    let result = parse_connect_args(&args);
    assert_eq!(result, None);
}

#[test]
fn parse_bare_ip_defaults_port() {
    let args: Vec<String> = vec!["game.exe".into(), "+connect".into(), "192.168.1.1".into()];
    let result = parse_connect_args(&args);
    assert_eq!(result, Some(("192.168.1.1".into(), 27015)));
}
