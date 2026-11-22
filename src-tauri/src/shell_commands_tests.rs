//! The port ledger's test (shell_commands.rs): every Tauri command in
//! `invoke.rs` is in exactly one table.

use std::collections::BTreeMap;

use super::invoke_commands;
use super::*;

/// Every registered command is in exactly one table, every table row is
/// a registered command, and every Host-call request is one the command
/// table classifies.
#[test]
fn every_command_is_in_exactly_one_table() {
    let registered = invoke_commands::parse(include_str!("invoke.rs")).unwrap();
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for name in HOST_CALLS
        .iter()
        .map(|(n, _)| *n)
        .chain(SHELL_LOCAL.iter().copied())
        .chain(PENDING_PORT.iter().map(|(n, _)| *n))
    {
        *seen.entry(name).or_default() += 1;
    }
    for command in &registered {
        assert_eq!(
            seen.remove(command.name()),
            Some(1),
            "{} must be in exactly one table",
            command.name()
        );
    }
    assert!(seen.is_empty(), "not registered in invoke.rs: {seen:?}");

    for (command, HostCall::Request(request)) in HOST_CALLS {
        assert!(
            rekindle_types::intent::kind_of(request).is_some(),
            "{command}: {request} is not in COMMAND_TABLE"
        );
        assert!(
            request.starts_with(|c: char| c.is_ascii_uppercase()),
            "{command}: {request} is not an IpcRequest name"
        );
    }
}
