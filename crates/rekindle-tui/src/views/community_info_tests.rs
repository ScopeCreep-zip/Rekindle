use rekindle_types::display::RoleDisplay;
use rekindle_types::dto::community::RoleTagColours;
use rekindle_types::dto::permission_catalogue::{
    PermissionCatalogue, PermissionEntry, PermissionGroup,
};

use super::role_lines;

#[test]
fn role_lines_use_the_hosts_labels_for_assigned_bits() {
    let role = RoleDisplay {
        id: "01".repeat(16),
        name: "Helper".into(),
        color: 0,
        tag_colours: RoleTagColours { dark: 0, light: 0 },
        permissions: 1 << 59,
        position: 1,
        position_label: "1".into(),
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
    };
    let catalogue = PermissionCatalogue {
        groups: vec![PermissionGroup {
            name: "Test group".into(),
            permissions: vec![
                PermissionEntry {
                    key: "ASSIGNED".into(),
                    label: "Assigned by the Host".into(),
                    bit: (1_u64 << 59).to_string(),
                },
                PermissionEntry {
                    key: "UNASSIGNED".into(),
                    label: "Unassigned by the Host".into(),
                    bit: (1_u64 << 58).to_string(),
                },
            ],
        }],
    };
    let roles = [role];
    let lines = role_lines(&roles, Some(&catalogue)).unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1].to_string(), "    Assigned by the Host");
}
