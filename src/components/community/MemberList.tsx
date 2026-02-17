import { Component, For, Show, createSignal, createMemo } from "solid-js";
import { Member, Role } from "../../stores/community.store";
import StatusDot from "../status/StatusDot";
import RoleTag from "./RoleTag";
import ContextMenu from "../common/ContextMenu";
import type { ContextMenuItem } from "../common/ContextMenu";
import {
  ICON_SHIELD,
  ICON_TIMEOUT,
  ICON_ACCOUNT_REMOVE,
  ICON_BAN,
  ICON_CHECK,
} from "../../icons";
import {
  handleRemoveCommunityMember,
  handleBanMember,
  handleAssignRole,
  handleUnassignRole,
  handleTimeoutMember,
} from "../../handlers/community.handlers";
import {
  calculateBasePermissions,
  highestPosition,
  hasPermission,
  KICK_MEMBERS,
  BAN_MEMBERS,
  MODERATE_MEMBERS,
  MANAGE_ROLES,
} from "../../ipc/permissions";

interface MemberListProps {
  members: Member[];
  communityId: string;
  myRoleIds: number[];
  roles: Role[];
  myPseudonymKey: string | null;
  isHosted: boolean;
}

const MemberList: Component<MemberListProps> = (props) => {
  const [contextMenu, setContextMenu] = createSignal<{
    x: number;
    y: number;
    member: Member;
  } | null>(null);

  const [rolePickerTarget, setRolePickerTarget] = createSignal<{
    x: number;
    y: number;
    member: Member;
  } | null>(null);

  const [searchQuery, setSearchQuery] = createSignal("");

  function myPerms(): number {
    return calculateBasePermissions(props.myRoleIds, props.roles, props.isHosted);
  }

  function myPosition(): number {
    return highestPosition(props.myRoleIds, props.roles);
  }

  function canManage(member: Member): boolean {
    const memberPos = highestPosition(member.roleIds, props.roles);
    return myPosition() > memberPos;
  }

  function handleMemberContextMenu(e: MouseEvent, member: Member): void {
    e.preventDefault();
    if (member.pseudonymKey === props.myPseudonymKey) return;
    if (!canManage(member)) return;
    setContextMenu({ x: e.clientX, y: e.clientY, member });
    setRolePickerTarget(null);
  }

  function handleCloseContextMenu(): void {
    setContextMenu(null);
    setRolePickerTarget(null);
  }

  function contextMenuItems(): ContextMenuItem[] {
    const ctx = contextMenu();
    if (!ctx) return [];
    const member = ctx.member;
    const perms = myPerms();
    const items: ContextMenuItem[] = [];

    if (hasPermission(perms, MANAGE_ROLES)) {
      items.push({
        label: "Manage Roles",
        icon: ICON_SHIELD,
        action: () => {
          setRolePickerTarget({ x: ctx.x, y: ctx.y + 30, member });
          setContextMenu(null);
        },
      });
    }

    if (hasPermission(perms, MODERATE_MEMBERS)) {
      items.push({
        label: "Timeout (10min)",
        icon: ICON_TIMEOUT,
        action: () => {
          handleTimeoutMember(props.communityId, member.pseudonymKey, 600, null);
        },
      });
    }

    if (hasPermission(perms, KICK_MEMBERS)) {
      items.push({
        label: "Kick Member",
        icon: ICON_ACCOUNT_REMOVE,
        action: () => {
          handleRemoveCommunityMember(props.communityId, member.pseudonymKey);
        },
        danger: true,
      });
    }

    if (hasPermission(perms, BAN_MEMBERS)) {
      items.push({
        label: "Ban Member",
        icon: ICON_BAN,
        action: () => {
          handleBanMember(props.communityId, member.pseudonymKey);
        },
        danger: true,
      });
    }

    return items;
  }

  function memberRoles(member: Member): { name: string; color: number }[] {
    const resolved = member.roleIds
      .map((id) => props.roles.find((r) => r.id === id))
      .filter((r): r is Role => r != null && r.name !== "@everyone")
      .sort((a, b) => b.position - a.position);
    return resolved.length > 0
      ? resolved.map((r) => ({ name: r.name, color: r.color }))
      : member.displayRole && member.displayRole !== "@everyone"
        ? [{ name: member.displayRole, color: 0 }]
        : [];
  }

  function toggleRole(pseudonymKey: string, roleId: number, hasRole: boolean): void {
    if (hasRole) {
      handleUnassignRole(props.communityId, pseudonymKey, roleId);
    } else {
      handleAssignRole(props.communityId, pseudonymKey, roleId);
    }
  }

  const filteredMembers = createMemo(() => {
    const q = searchQuery().toLowerCase();
    return q
      ? props.members.filter((m) => m.displayName.toLowerCase().includes(q))
      : props.members;
  });

  const groupedMembers = createMemo(() => {
    const hoistedRoles = props.roles
      .filter((r) => r.hoist && r.id !== 0)
      .sort((a, b) => b.position - a.position);

    const groups: { name: string; members: Member[] }[] = [];
    const placed = new Set<string>();

    for (const role of hoistedRoles) {
      const members = filteredMembers().filter(
        (m) => m.roleIds.includes(role.id) && !placed.has(m.pseudonymKey),
      );
      if (members.length > 0) {
        groups.push({ name: `${role.name} — ${members.length}`, members });
        members.forEach((m) => placed.add(m.pseudonymKey));
      }
    }

    const remaining = filteredMembers().filter((m) => !placed.has(m.pseudonymKey));
    if (remaining.length > 0) {
      groups.push({ name: `Online — ${remaining.length}`, members: remaining });
    }

    return groups;
  });

  return (
    <div class="member-list">
      <div class="member-list-header">
        Members — {props.members.length}
      </div>
      <div class="member-search-wrapper">
        <input
          class="member-search-input"
          type="text"
          placeholder="Search..."
          value={searchQuery()}
          onInput={(e) => setSearchQuery(e.currentTarget.value)}
        />
      </div>
      <For each={groupedMembers()}>
        {(group) => (
          <>
            <div class="member-group-header">{group.name}</div>
            <For each={group.members}>
              {(member) => {
                const roles = () => memberRoles(member);
                return (
                  <div
                    class="member-item"
                    onContextMenu={(e) => handleMemberContextMenu(e, member)}
                  >
                    <StatusDot status={member.status || "online"} />
                    <div class="member-name-info">
                      <span class="member-name">{member.displayName}</span>
                      <div class="member-roles-row">
                        <For each={roles()}>
                          {(role) => <RoleTag name={role.name} color={role.color} />}
                        </For>
                        <Show when={member.timeoutUntil}>
                          <span class="nf-icon timeout-indicator" title="Timed out">{ICON_TIMEOUT}</span>
                        </Show>
                      </div>
                    </div>
                  </div>
                );
              }}
            </For>
          </>
        )}
      </For>
      <Show when={contextMenu()}>
        {(menu) => (
          <ContextMenu
            items={contextMenuItems()}
            x={menu().x}
            y={menu().y}
            onClose={handleCloseContextMenu}
          />
        )}
      </Show>
      <Show when={rolePickerTarget()}>
        {(target) => (
          <div
            class="context-menu"
            style={{
              left: `${target().x}px`,
              top: `${target().y}px`,
            }}
          >
            <div class="context-menu-header">Manage Roles</div>
            <For each={props.roles.filter((r) => r.id !== 0).sort((a, b) => b.position - a.position)}>
              {(role) => {
                const hasRole = () => target().member.roleIds.includes(role.id);
                return (
                  <div
                    class={`role-picker-item ${hasRole() ? "role-picker-item-active" : ""}`}
                    onClick={() => toggleRole(target().member.pseudonymKey, role.id, hasRole())}
                  >
                    <input type="checkbox" class="role-picker-checkbox" checked={hasRole()} readOnly />
                    {role.name}
                  </div>
                );
              }}
            </For>
            <div class="context-menu-separator" />
            <div
              class="context-menu-item"
              onClick={() => setRolePickerTarget(null)}
            >
              <span class="nf-icon context-menu-icon">{ICON_CHECK}</span>
              Done
            </div>
          </div>
        )}
      </Show>
    </div>
  );
};

export default MemberList;
