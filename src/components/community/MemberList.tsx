import { Component, For, Show, createSignal, createMemo, JSX } from "solid-js";
import { ContextMenu } from "@kobalte/core/context-menu";
import { Popover } from "@kobalte/core/popover";
import { Member, Role, communityState } from "../../stores/community.store";
import StatusDot from "../status/StatusDot";
import RoleTag from "./RoleTag";
import MemberProfilePopup from "./MemberProfilePopup";
import ConfirmDialog from "../common/ConfirmDialog";
import {
  ICON_SHIELD,
  ICON_TIMEOUT,
  ICON_ACCOUNT_REMOVE,
  ICON_BAN,
  ICON_MIC_OFF,
  ICON_HEADPHONES_OFF,
} from "../../icons";
import {
  handleRemoveCommunityMember,
  handleBanMember,
  handleAssignRole,
  handleUnassignRole,
  handleTimeoutMember,
  handleServerMuteMember,
  handleServerDeafenMember,
} from "../../handlers/community.handlers";
import {
  calculateBasePermissions,
  highestPosition,
  hasPermission,
  KICK_MEMBERS,
  BAN_MEMBERS,
  MODERATE_MEMBERS,
  MANAGE_ROLES,
  MUTE_MEMBERS,
  DEAFEN_MEMBERS,
} from "../../ipc/permissions";

interface MemberListProps {
  members: Member[];
  communityId: string;
  myRoleIds: number[];
  roles: Role[];
  myPseudonymKey: string | null;
}

const TIMEOUT_PRESETS: { label: string; seconds: number }[] = [
  { label: "5 minutes", seconds: 300 },
  { label: "10 minutes", seconds: 600 },
  { label: "30 minutes", seconds: 1800 },
  { label: "1 hour", seconds: 3600 },
  { label: "24 hours", seconds: 86400 },
  { label: "1 week", seconds: 604800 },
];

const MemberList: Component<MemberListProps> = (props) => {
  const [searchQuery, setSearchQuery] = createSignal("");
  const [openProfileFor, setOpenProfileFor] = createSignal<string | null>(null);

  const [confirmAction, setConfirmAction] = createSignal<{
    type: "kick" | "ban";
    pseudonymKey: string;
    displayName: string;
  } | null>(null);

  function myPerms(): bigint {
    return calculateBasePermissions(props.myRoleIds, props.roles);
  }

  function myPosition(): number {
    return highestPosition(props.myRoleIds, props.roles);
  }

  function canManage(member: Member): boolean {
    const memberPos = highestPosition(member.roleIds, props.roles);
    return myPosition() > memberPos;
  }

  // Architecture §10 — server-mute / server-deafen apply only when the
  // target is currently sitting in a community voice channel.
  function memberVoiceChannelId(member: Member): string | null {
    const channels = communityState.voiceChannels;
    for (const channelId in channels) {
      const channel = channels[channelId];
      if (channel?.participants?.includes(member.pseudonymKey)) {
        return channelId;
      }
    }
    return null;
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

  function handleConfirm(): void {
    const action = confirmAction();
    if (!action) return;
    if (action.type === "kick") {
      handleRemoveCommunityMember(props.communityId, action.pseudonymKey);
    } else {
      handleBanMember(props.communityId, action.pseudonymKey);
    }
    setConfirmAction(null);
  }

  function memberMenu(member: Member): JSX.Element {
    const perms = myPerms();
    const voiceChannelId = memberVoiceChannelId(member);
    const assignableRoles = props.roles
      .filter((r) => r.id !== 0)
      .sort((a, b) => b.position - a.position);

    return (
      <ContextMenu.Portal>
        <ContextMenu.Content class="context-menu">
          <Show when={hasPermission(perms, MANAGE_ROLES) && assignableRoles.length > 0}>
            <ContextMenu.Sub>
              <ContextMenu.SubTrigger class="context-menu-item">
                <span class="nf-icon context-menu-icon">{ICON_SHIELD}</span>
                Manage Roles
              </ContextMenu.SubTrigger>
              <ContextMenu.Portal>
                <ContextMenu.SubContent class="context-menu">
                  <For each={assignableRoles}>
                    {(role) => {
                      const hasRole = () => member.roleIds.includes(role.id);
                      return (
                        <ContextMenu.CheckboxItem
                          class="context-menu-item"
                          checked={hasRole()}
                          onChange={() =>
                            toggleRole(member.pseudonymKey, role.id, hasRole())
                          }
                        >
                          <ContextMenu.ItemIndicator class="context-menu-indicator">
                            ✓
                          </ContextMenu.ItemIndicator>
                          {role.name}
                        </ContextMenu.CheckboxItem>
                      );
                    }}
                  </For>
                </ContextMenu.SubContent>
              </ContextMenu.Portal>
            </ContextMenu.Sub>
          </Show>

          <Show when={hasPermission(perms, MODERATE_MEMBERS)}>
            <ContextMenu.Sub>
              <ContextMenu.SubTrigger class="context-menu-item">
                <span class="nf-icon context-menu-icon">{ICON_TIMEOUT}</span>
                Timeout
              </ContextMenu.SubTrigger>
              <ContextMenu.Portal>
                <ContextMenu.SubContent class="context-menu">
                  <For each={TIMEOUT_PRESETS}>
                    {(t) => (
                      <ContextMenu.Item
                        class="context-menu-item"
                        onSelect={() =>
                          handleTimeoutMember(
                            props.communityId,
                            member.pseudonymKey,
                            t.seconds,
                            null,
                          )
                        }
                      >
                        {t.label}
                      </ContextMenu.Item>
                    )}
                  </For>
                </ContextMenu.SubContent>
              </ContextMenu.Portal>
            </ContextMenu.Sub>
          </Show>

          <Show when={voiceChannelId && hasPermission(perms, MUTE_MEMBERS)}>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={() =>
                handleServerMuteMember(
                  props.communityId,
                  voiceChannelId!,
                  member.pseudonymKey,
                  true,
                )
              }
            >
              <span class="nf-icon context-menu-icon">{ICON_MIC_OFF}</span>
              Server Mute
            </ContextMenu.Item>
          </Show>
          <Show when={voiceChannelId && hasPermission(perms, DEAFEN_MEMBERS)}>
            <ContextMenu.Item
              class="context-menu-item"
              onSelect={() =>
                handleServerDeafenMember(
                  props.communityId,
                  voiceChannelId!,
                  member.pseudonymKey,
                  true,
                )
              }
            >
              <span class="nf-icon context-menu-icon">{ICON_HEADPHONES_OFF}</span>
              Server Deafen
            </ContextMenu.Item>
          </Show>

          <Show when={hasPermission(perms, KICK_MEMBERS)}>
            <ContextMenu.Item
              class="context-menu-item context-menu-item-danger"
              onSelect={() =>
                setConfirmAction({
                  type: "kick",
                  pseudonymKey: member.pseudonymKey,
                  displayName: member.displayName,
                })
              }
            >
              <span class="nf-icon context-menu-icon">{ICON_ACCOUNT_REMOVE}</span>
              Kick Member
            </ContextMenu.Item>
          </Show>

          <Show when={hasPermission(perms, BAN_MEMBERS)}>
            <ContextMenu.Item
              class="context-menu-item context-menu-item-danger"
              onSelect={() =>
                setConfirmAction({
                  type: "ban",
                  pseudonymKey: member.pseudonymKey,
                  displayName: member.displayName,
                })
              }
            >
              <span class="nf-icon context-menu-icon">{ICON_BAN}</span>
              Ban Member
            </ContextMenu.Item>
          </Show>
        </ContextMenu.Content>
      </ContextMenu.Portal>
    );
  }

  function renderMember(member: Member): JSX.Element {
    const isSelf = member.pseudonymKey === props.myPseudonymKey;
    const allowMenu = !isSelf && canManage(member);
    const roles = () => memberRoles(member);
    const isOpen = () => openProfileFor() === member.pseudonymKey;

    const popoverChildren = (
      <>
        <Popover.Trigger
          as="div"
          class="member-item"
        >
          <StatusDot status={member.status || "online"} />
          <div class="member-name-info">
            <span class="member-name">{member.displayName}</span>
            <Show when={member.gameInfo}>
              {(info) => (
                <span class="member-game-info">
                  {info().gameName}
                  <Show when={info().serverAddress}>
                    <span class="member-game-server"> on {info().serverAddress}</span>
                  </Show>
                </span>
              )}
            </Show>
            <div class="member-roles-row">
              <For each={roles()}>
                {(role) => <RoleTag name={role.name} color={role.color} />}
              </For>
              <Show when={member.timeoutUntil}>
                <span class="nf-icon timeout-indicator" title="Timed out">{ICON_TIMEOUT}</span>
              </Show>
            </div>
          </div>
        </Popover.Trigger>
        <MemberProfilePopup
          communityId={props.communityId}
          member={member}
          roles={props.roles}
          onClose={() => setOpenProfileFor(null)}
          myPseudonymKey={props.myPseudonymKey}
        />
      </>
    );

    const popover = (
      <Popover
        open={isOpen()}
        onOpenChange={(open) =>
          setOpenProfileFor(open ? member.pseudonymKey : null)
        }
      >
        {popoverChildren}
      </Popover>
    );

    if (!allowMenu) return popover;

    return (
      <ContextMenu>
        <ContextMenu.Trigger as="div">{popover}</ContextMenu.Trigger>
        {memberMenu(member)}
      </ContextMenu>
    );
  }

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
              {(member) => renderMember(member)}
            </For>
          </>
        )}
      </For>
      <ConfirmDialog
        isOpen={confirmAction() !== null}
        title={confirmAction()?.type === "kick" ? "Kick Member" : "Ban Member"}
        message={`Are you sure you want to ${confirmAction()?.type ?? "kick"} ${confirmAction()?.displayName ?? "this member"}?${confirmAction()?.type === "ban" ? " They will not be able to rejoin." : ""}`}
        danger
        confirmLabel={confirmAction()?.type === "kick" ? "Kick" : "Ban"}
        onConfirm={handleConfirm}
        onCancel={() => setConfirmAction(null)}
      />
    </div>
  );
};

export default MemberList;
