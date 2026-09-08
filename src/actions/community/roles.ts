import { commands } from "../../ipc/commands";
import { setCommunityState, communityState } from "../../stores/community.store";
import { addToast } from "../../stores/toast.store";
import { computeDisplayRoleName } from "./shared";

export async function handleAssignRole(
  communityId: string,
  pseudonymKey: string,
  roleId: number,
): Promise<void> {
  try {
    await commands.assignRole(communityId, pseudonymKey, roleId);
    // Update local state — add roleId to member
    const community = communityState.communities[communityId];
    const memberIdx = community?.members.findIndex((member) => member.pseudonymKey === pseudonymKey) ?? -1;
    if (community && memberIdx >= 0) {
      const nextRoleIds = community.members[memberIdx].roleIds.includes(roleId)
        ? community.members[memberIdx].roleIds
        : [...community.members[memberIdx].roleIds, roleId];
      setCommunityState("communities", communityId, "members", memberIdx, "roleIds", nextRoleIds);
      setCommunityState(
        "communities",
        communityId,
        "members",
        memberIdx,
        "displayRole",
        computeDisplayRoleName(nextRoleIds, community.roles),
      );
    }
  } catch (e) {
    console.error("Failed to assign role:", e);
    addToast("Failed to assign role", "error");
  }
}

export async function handleUnassignRole(
  communityId: string,
  pseudonymKey: string,
  roleId: number,
): Promise<void> {
  try {
    await commands.unassignRole(communityId, pseudonymKey, roleId);
    // Update local state — remove roleId from member
    const community = communityState.communities[communityId];
    const memberIdx = community?.members.findIndex((member) => member.pseudonymKey === pseudonymKey) ?? -1;
    if (community && memberIdx >= 0) {
      const nextRoleIds = community.members[memberIdx].roleIds.filter((id) => id !== roleId);
      setCommunityState("communities", communityId, "members", memberIdx, "roleIds", nextRoleIds);
      setCommunityState(
        "communities",
        communityId,
        "members",
        memberIdx,
        "displayRole",
        computeDisplayRoleName(nextRoleIds, community.roles),
      );
    }
  } catch (e) {
    console.error("Failed to unassign role:", e);
    addToast("Failed to unassign role", "error");
  }
}

export async function handleCreateRole(
  communityId: string,
  name: string,
  color: number,
  permissions: string,
  hoist: boolean,
  mentionable: boolean,
  selfAssignable: boolean,
): Promise<number | null> {
  try {
    const roleId = await commands.createRole(communityId, name, color, permissions, hoist, mentionable, selfAssignable);
    // Optimistic update — add the new role to the store
    const community = communityState.communities[communityId];
    if (community) {
      const newRole = { id: roleId, name, color, permissions, position: 0, hoist, mentionable, selfAssignable };
      setCommunityState("communities", communityId, "roles", [...community.roles, newRole]);
    }
    return roleId;
  } catch (e) {
    console.error("Failed to create role:", e);
    addToast("Failed to create role", "error");
    return null;
  }
}

export async function handleEditRole(
  communityId: string,
  roleId: number,
  name: string | null,
  color: number | null,
  permissions: string | null,
  position: number | null,
  hoist: boolean | null,
  mentionable: boolean | null,
  selfAssignable: boolean | null,
): Promise<void> {
  try {
    await commands.editRole(communityId, roleId, name, color, permissions, position, hoist, mentionable, selfAssignable);
    // Optimistic update — patch the role in the store
    const community = communityState.communities[communityId];
    if (community) {
      const idx = community.roles.findIndex((r) => r.id === roleId);
      if (idx >= 0) {
        const updated = { ...community.roles[idx] };
        if (name !== null) updated.name = name;
        if (color !== null) updated.color = color;
        if (permissions !== null) updated.permissions = permissions;
        if (position !== null) updated.position = position;
        if (hoist !== null) updated.hoist = hoist;
        if (mentionable !== null) updated.mentionable = mentionable;
        if (selfAssignable !== null) updated.selfAssignable = selfAssignable;
        setCommunityState("communities", communityId, "roles", idx, updated);
      }
    }
  } catch (e) {
    console.error("Failed to edit role:", e);
    addToast("Failed to edit role", "error");
  }
}

export async function handleDeleteRole(
  communityId: string,
  roleId: number,
): Promise<void> {
  try {
    await commands.deleteRole(communityId, roleId);
    // Optimistic update — remove role from store and scrub from members
    const community = communityState.communities[communityId];
    if (community) {
      setCommunityState("communities", communityId, "roles",
        community.roles.filter((r) => r.id !== roleId),
      );
      // Scrub the deleted roleId from all members
      community.members.forEach((member, idx) => {
        if (member.roleIds.includes(roleId)) {
          setCommunityState("communities", communityId, "members", idx, "roleIds",
            member.roleIds.filter((id) => id !== roleId),
          );
        }
      });
      // Scrub from myRoleIds
      if (community.myRoleIds.includes(roleId)) {
        setCommunityState("communities", communityId, "myRoleIds",
          community.myRoleIds.filter((id) => id !== roleId),
        );
      }
    }
  } catch (e) {
    console.error("Failed to delete role:", e);
    addToast("Failed to delete role", "error");
  }
}

export async function handleSelfAssignRole(
  communityId: string,
  roleId: number,
): Promise<void> {
  try {
    await commands.selfAssignRole(communityId, roleId);
    const community = communityState.communities[communityId];
    const myPseudonymKey = community?.myPseudonymKey;
    if (!community || !myPseudonymKey) return;
    if (!community.myRoleIds.includes(roleId)) {
      setCommunityState("communities", communityId, "myRoleIds", [...community.myRoleIds, roleId]);
    }
    const memberIdx = community.members.findIndex((member) => member.pseudonymKey === myPseudonymKey);
    if (memberIdx >= 0 && !community.members[memberIdx].roleIds.includes(roleId)) {
      const nextRoleIds = [...community.members[memberIdx].roleIds, roleId];
      setCommunityState(
        "communities",
        communityId,
        "members",
        memberIdx,
        "roleIds",
        nextRoleIds,
      );
      setCommunityState(
        "communities",
        communityId,
        "members",
        memberIdx,
        "displayRole",
        computeDisplayRoleName(nextRoleIds, community.roles),
      );
    }
  } catch (e) {
    console.error("Failed to self-assign role:", e);
    addToast("Failed to assign role", "error");
  }
}

export async function handleSelfUnassignRole(
  communityId: string,
  roleId: number,
): Promise<void> {
  try {
    await commands.selfUnassignRole(communityId, roleId);
    const community = communityState.communities[communityId];
    const myPseudonymKey = community?.myPseudonymKey;
    if (!community || !myPseudonymKey) return;
    setCommunityState(
      "communities",
      communityId,
      "myRoleIds",
      community.myRoleIds.filter((id) => id !== roleId),
    );
    const memberIdx = community.members.findIndex((member) => member.pseudonymKey === myPseudonymKey);
    if (memberIdx >= 0) {
      const nextRoleIds = community.members[memberIdx].roleIds.filter((id) => id !== roleId);
      setCommunityState(
        "communities",
        communityId,
        "members",
        memberIdx,
        "roleIds",
        nextRoleIds,
      );
      setCommunityState(
        "communities",
        communityId,
        "members",
        memberIdx,
        "displayRole",
        computeDisplayRoleName(nextRoleIds, community.roles),
      );
    }
  } catch (e) {
    console.error("Failed to self-unassign role:", e);
    addToast("Failed to remove role", "error");
  }
}
