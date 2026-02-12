import { Component, For, Show, createSignal } from "solid-js";
import { Member } from "../../stores/community.store";
import { authState } from "../../stores/auth.store";
import StatusDot from "../status/StatusDot";
import RoleTag from "./RoleTag";
import ContextMenu from "../common/ContextMenu";
import type { ContextMenuItem } from "../common/ContextMenu";
import {
  handleRemoveCommunityMember,
  handleUpdateMemberRole,
} from "../../handlers/community.handlers";

interface MemberListProps {
  members: Member[];
  communityId: string;
  myRole: string | null;
}

const AVAILABLE_ROLES = ["admin", "moderator", "member"];

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

  function isOwnerOrAdmin(): boolean {
    const role = props.myRole;
    return role === "owner" || role === "admin";
  }

  function handleMemberContextMenu(e: MouseEvent, member: Member): void {
    e.preventDefault();
    // Don't show context menu for yourself
    if (member.publicKey === authState.publicKey) return;
    if (!isOwnerOrAdmin()) return;
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
    return [
      {
        label: "Change Role",
        action: () => {
          setRolePickerTarget({ x: ctx.x, y: ctx.y + 30, member });
          setContextMenu(null);
        },
      },
      {
        label: "Remove Member",
        action: () => {
          handleRemoveCommunityMember(props.communityId, member.publicKey);
        },
        danger: true,
      },
    ];
  }

  function handleSelectRole(publicKey: string, role: string): void {
    handleUpdateMemberRole(props.communityId, publicKey, role);
    setRolePickerTarget(null);
  }

  return (
    <div class="member-list">
      <div class="member-list-header">
        Members — {props.members.length}
      </div>
      <For each={props.members}>
        {(member) => (
          <div
            class="member-item"
            onContextMenu={(e) => handleMemberContextMenu(e, member)}
          >
            <StatusDot status={member.status || "online"} />
            <div class="member-name-info">
              <span class="member-name">{member.displayName}</span>
              {member.role && <RoleTag name={member.role} />}
            </div>
          </div>
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
            <div class="context-menu-header">Set Role</div>
            <For each={AVAILABLE_ROLES}>
              {(role) => (
                <div
                  class={`role-picker-item ${target().member.role === role ? "role-picker-item-active" : ""}`}
                  onClick={() => handleSelectRole(target().member.publicKey, role)}
                >
                  {role.charAt(0).toUpperCase() + role.slice(1)}
                </div>
              )}
            </For>
            <div class="context-menu-separator" />
            <div
              class="context-menu-item"
              onClick={() => setRolePickerTarget(null)}
            >
              Cancel
            </div>
          </div>
        )}
      </Show>
    </div>
  );
};

export default MemberList;
