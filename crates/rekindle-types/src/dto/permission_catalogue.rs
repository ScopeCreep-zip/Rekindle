//! The Host's role-editor catalogue, with bits serialized as decimal strings.

use serde::{Deserialize, Serialize};

use crate::permissions::PERMISSION_CATALOGUE;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionCatalogue {
    pub groups: Vec<PermissionGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionGroup {
    pub name: String,
    pub permissions: Vec<PermissionEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionEntry {
    pub key: String,
    pub label: String,
    pub bit: String,
}

impl PermissionCatalogue {
    /// Build the response from the ordered permission definitions.
    #[must_use]
    pub fn current() -> Self {
        Self {
            groups: PERMISSION_CATALOGUE
                .iter()
                .map(|group| PermissionGroup {
                    name: group.name.into(),
                    permissions: group
                        .permissions
                        .iter()
                        .map(|entry| PermissionEntry {
                            key: entry.key.into(),
                            label: entry.label.into(),
                            bit: entry.bit.to_string(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}
