use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::dto::permission_catalogue::PermissionCatalogue;
use rekindle_types::permissions::PERMISSION_CATALOGUE;

use crate::harness::start_with;
use crate::shell_node::SimMode;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn both_locked_host_modes_serve_the_types_permission_catalogue() {
    let (_sim, nodes) = start_with(2, 584, &|index| {
        ([SimMode::Daemon, SimMode::Shell][index], None)
    })
    .await;

    for node in nodes {
        let response = node.answer(IpcRequest::PermissionCatalogue).await;
        let IpcResponse::Ok {
            data, undo: None, ..
        } = response
        else {
            panic!("expected a static catalogue, got {response:?}");
        };
        let catalogue: PermissionCatalogue = serde_json::from_value(data).unwrap();
        assert_eq!(catalogue.groups.len(), PERMISSION_CATALOGUE.len());
        for (actual, expected) in catalogue.groups.iter().zip(PERMISSION_CATALOGUE) {
            assert_eq!(actual.name, expected.name);
            assert_eq!(actual.permissions.len(), expected.permissions.len());
            for (actual, expected) in actual.permissions.iter().zip(expected.permissions) {
                assert_eq!(actual.key, expected.key);
                assert_eq!(actual.label, expected.label);
                assert_eq!(actual.bit, expected.bit.to_string());
            }
        }
    }
}
