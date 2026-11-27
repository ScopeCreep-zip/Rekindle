use super::*;

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn the_three_sources_parse() {
    let invoke = "tauri::generate_handler![\n commands::auth::login,\n #[cfg(debug_assertions)]\n commands::auth::pqxdh_bundle_info,\n]";
    assert_eq!(
        tauri_commands(invoke),
        names(&["login", "pqxdh_bundle_info"])
    );
    let ipc = "pub enum IpcRequest { Unlock { passphrase: String }, \
             Community(CommunityRequest), Transfer(TransferRequest), \
             Servers(ServersRequest), Lock }";
    let community = "pub enum CommunityRequest { Moderation(ModerationRequest) } \
             pub enum ModerationRequest { Kick { target: String }, BanList }";
    let transfer = r#"pub enum TransferRequest { #[serde(rename = "TransferList")] List, Stop { id: String } }"#;
    let servers = r#"pub enum ServersRequest { #[serde(rename = "ServersView")] View, #[serde(rename = "RecentServerRemove")] RecentRemove { game_id: u32 } }"#;
    let carriers = [
        (
            "Transfer",
            "TransferRequest",
            "transfer.rs",
            transfer.to_string(),
        ),
        (
            "Servers",
            "ServersRequest",
            "servers.rs",
            servers.to_string(),
        ),
    ];
    assert_eq!(
        ipc_requests(ipc, &[community.to_string()], &carriers).unwrap(),
        names(&[
            "Unlock",
            "Lock",
            "Kick",
            "BanList",
            "TransferList",
            "Stop",
            "ServersView",
            "RecentServerRemove"
        ])
    );
    let table = r#"pub(crate) const CORE_ROWS: &[(&str, Option<IntentKind>)] = &[("login", Some(IntentKind::Login)), ("Lock", None)]; pub const COMMAND_TABLE: &[&[(&str, Option<IntentKind>)]] = &[CORE_ROWS];"#;
    assert_eq!(table_names(table).unwrap(), names(&["login", "Lock"]));
}

/// Every carrier the list names must exist on `IpcRequest`, and each
/// expands to its own sub-enum's leaves, never the carrier's name.
#[test]
fn a_second_carrier_expands_and_a_missing_one_fails() {
    let community = "pub enum CommunityRequest { Moderation(ModerationRequest) } \
             pub enum ModerationRequest { Kick { target: String } }";
    let servers = r#"pub enum ServersRequest { #[serde(rename = "ServersView")] View }"#;
    let carriers = [(
        "Servers",
        "ServersRequest",
        "servers.rs",
        servers.to_string(),
    )];
    let with = "pub enum IpcRequest { Community(CommunityRequest), Servers(ServersRequest) }";
    let leaves = ipc_requests(with, &[community.to_string()], &carriers).unwrap();
    assert_eq!(leaves, names(&["Kick", "ServersView"]));
    let without = "pub enum IpcRequest { Community(CommunityRequest), Lock }";
    let missing = ipc_requests(without, &[community.to_string()], &carriers).unwrap_err();
    assert!(missing.to_string().contains("no `IpcRequest::Servers`"));
}

/// Mutations: an unclassified command, an unclassified request, a
/// stale row and a duplicate row each fail.
#[test]
fn every_mismatch_is_reported() {
    let ok = compare(
        &names(&["login"]),
        &names(&["Lock"]),
        &names(&["login", "Lock"]),
    );
    assert!(ok.is_empty());
    let new_command = compare(
        &names(&["login", "kick"]),
        &names(&["Lock"]),
        &names(&["login", "Lock"]),
    );
    assert_eq!(new_command.len(), 1);
    assert!(new_command[0].contains("`kick` is not classified"));
    let new_request = compare(
        &names(&["login"]),
        &names(&["Lock", "Ban"]),
        &names(&["login", "Lock"]),
    );
    assert!(new_request[0].contains("`Ban`"));
    let stale = compare(
        &names(&["login"]),
        &names(&["Lock"]),
        &names(&["login", "Lock", "gone"]),
    );
    assert!(stale[0].contains("`gone`"));
    let twice = compare(
        &names(&["login"]),
        &names(&["Lock"]),
        &names(&["login", "Lock", "login"]),
    );
    assert!(twice[0].contains("listed twice"));
}
