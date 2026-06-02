//! `governance` roles variant encode/decode helpers.

use super::super::sub_types::{
    pseudonym_key_from_capnp, pseudonym_key_to_capnp, role_id_from_capnp, uuid16_to_capnp,
};
use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community_governance_capnp::{self as schema_pkg};
use crate::error::ProtocolError;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{PseudonymKey, RoleId};

pub(super) fn write_role_definition(
    mut p: schema_pkg::role_definition_entry::Builder<'_>,
    role_id: RoleId,
    name: &str,
    permissions: u64,
    position: u32,
    color: u32,
    hoist: bool,
    mentionable: bool,
    self_assignable: bool,
    exclusion_group: Option<&str>,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_role_id(), &role_id.0);
    p.set_name(name);
    p.set_permissions(permissions);
    p.set_position(position);
    p.set_color(color);
    p.set_hoist(hoist);
    p.set_mentionable(mentionable);
    p.set_self_assignable(self_assignable);
    p.set_has_exclusion_group(exclusion_group.is_some());
    if let Some(g) = exclusion_group {
        p.set_exclusion_group(g);
    }
    p.set_lamport(lamport);
}

pub(super) fn write_role_assignment(
    mut p: schema_pkg::role_assignment_entry::Builder<'_>,
    target: &PseudonymKey,
    role_id: RoleId,
    lamport: u64,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    uuid16_to_capnp(p.reborrow().init_role_id(), &role_id.0);
    p.set_lamport(lamport);
}

pub(super) fn write_role_unassignment(
    mut p: schema_pkg::role_unassignment_entry::Builder<'_>,
    target: &PseudonymKey,
    role_id: RoleId,
    lamport: u64,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    uuid16_to_capnp(p.reborrow().init_role_id(), &role_id.0);
    p.set_lamport(lamport);
}

pub(super) fn write_role_archived(
    mut p: schema_pkg::role_archived_entry::Builder<'_>,
    role_id: RoleId,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_role_id(), &role_id.0);
    p.set_lamport(lamport);
}

pub(super) fn read_role_definition(
    p: schema_pkg::role_definition_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::RoleDefinition {
        role_id: role_id_from_capnp(p.get_role_id().map_err(|e| capnp_err(&e))?)?,
        name: text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?,
        permissions: p.get_permissions(),
        position: p.get_position(),
        color: p.get_color(),
        hoist: p.get_hoist(),
        mentionable: p.get_mentionable(),
        self_assignable: p.get_self_assignable(),
        exclusion_group: if p.get_has_exclusion_group() {
            Some(text_to_string(
                p.get_exclusion_group().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_role_assignment(
    p: schema_pkg::role_assignment_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::RoleAssignment {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        role_id: role_id_from_capnp(p.get_role_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_role_unassignment(
    p: schema_pkg::role_unassignment_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::RoleUnassignment {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        role_id: role_id_from_capnp(p.get_role_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_role_archived(
    p: schema_pkg::role_archived_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::RoleArchived {
        role_id: role_id_from_capnp(p.get_role_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}
