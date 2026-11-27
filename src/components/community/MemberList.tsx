import { type Component, For, Show, createEffect, createSignal, createMemo, type JSX } from "solid-js";
import SkinGlyph from "../common/SkinGlyph";
import { ContextMenu } from "@kobalte/core/context-menu";
import { Popover } from "@kobalte/core/popover";
import { EVERYONE_ROLE_ID, type Member, type Role, type RoleTagColours, communityState } from "../../stores/community.store";
import StatusDot from "../status/StatusDot";
import type { MemberGroup } from "../../ipc/commands/dto";
import RoleTag from "./RoleTag";
import { rovingList } from "../../utils/roving_list";
import MemberProfilePopup from "./MemberProfilePopup";
import { useMembersOnScreen } from "../../hooks/useMembersOnScreen";
import {
  handleRemoveCommunityMember,
  handleBanMember,
  handleAssignRole,
  handleUnassignRole,
  handleTimeoutMember,
  handleServerMuteMember,
  handleServerDeafenMember,
  handleSetCommunityBlock,
  loadCommunityBlocks,
} from "../../actions/community.actions";

interface MemberListProps {
  members: Member[];
  communityId: string;
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

function groupKey(group: MemberGroup): string {
  return group.kind === "role" ? `role:${group.id}` : group.kind;
}

function groupLabel(group: MemberGroup): string {
  if (group.kind === "role") return group.name;
  return group.kind === "online" ? "Online" : "Offline";
}

const MemberList: Component<MemberListProps> = (props) => {
  const [searchQuery, setSearchQuery] = createSignal("");
  const [openProfileFor, setOpenProfileFor] = createSignal<string | null>(null);
  const [blocked, setBlocked] = createSignal<Set<string>>(new Set());
  const onScreen = useMembersOnScreen(() => props.communityId);

  createEffect(() => {
    const communityId = props.communityId;
    void loadCommunityBlocks(communityId).then((set) => {
      if (props.communityId === communityId) setBlocked(set);
    });
  });

  async function toggleBlock(pseudonymKey: string): Promise<void> {
    const key = pseudonymKey.toLowerCase();
    const next = !blocked().has(key);
    if (!(await handleSetCommunityBlock(props.communityId, pseudonymKey, next))) return;
    const set = new Set(blocked());
    if (next) set.add(key);
    else set.delete(key);
    setBlocked(set);
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

  function memberRoles(member: Member): { name: string; color: number; tagColours?: RoleTagColours }[] {
    const resolved = member.roleIds
      .map((id) => props.roles.find((r) => r.id === id))
      .filter((r): r is Role => r != null && r.name !== "@everyone")
      .sort((a, b) => b.position - a.position);
    return resolved.length > 0
      ? resolved.map((r) => ({ name: r.name, color: r.color, tagColours: r.tagColours }))
      : member.displayRole && member.displayRole !== "@everyone"
        ? [{ name: member.displayRole, color: 0 }]
        : [];
  }

  function toggleRole(pseudonymKey: string, roleId: string, hasRole: boolean): void {
    if (hasRole) {
      handleUnassignRole(props.communityId, pseudonymKey, roleId);
    } else {
      handleAssignRole(props.communityId, pseudonymKey, roleId);
    }
  }

  /** The status a member's row draws; an unset status is shown online. */
  function memberStatus(member: Member): string {
    return member.status || "online";
  }

  const filteredMembers = createMemo(() => {
    const q = searchQuery().toLowerCase();
    return q
      ? props.members.filter((m) => m.displayName.toLowerCase().includes(q))
      : props.members;
  });

  // The groups are the Host's (`Member.group`, Discord's member list rules
  // decided once for every frontend): the list keeps the Host's order and
  // starts a group wherever the group changes. The rows are one keyed list
  // of the store's members (MM1): a group header is drawn before the first
  // member of its group, so a re-read that moves or relabels a group never
  // rebuilds the rows (and never closes a menu open on one).
  const groupHeads = createMemo(() => {
    const heads = new Map<string, string>();
    let first: Member | null = null;
    let label = "";
    let count = 0;
    let currentKey = "";
    const close = () => {
      if (first) heads.set(first.pseudonymKey, `${label} — ${count}`);
    };
    for (const m of filteredMembers()) {
      const key = groupKey(m.group);
      if (first === null || key !== currentKey) {
        close();
        first = m;
        label = groupLabel(m.group);
        count = 0;
        currentKey = key;
      }
      count += 1;
    }
    close();
    return heads;
  });

  function memberMenu(member: Member): JSX.Element {
    // Never-hide: every action is drawn; one the Host would refuse for
    // this member is disabled with its reason (`member.moderation`: rank,
    // permission, self, not in a voice channel), never a rank compared here.
    // Every read is an accessor (MM1): the row outlives the Host's
    // re-reads, so the menu follows the verdicts as they change.
    const mod = () => member.moderation;
    const voiceChannelId = () => memberVoiceChannelId(member);
    const assignableRoles = () =>
      props.roles
        .filter((r) => r.id !== EVERYONE_ROLE_ID)
        .sort((a, b) => b.position - a.position);

    return (
      <ContextMenu.Portal>
        <ContextMenu.Content class="context-menu">
          {/* Never-hide: drawn always, disabled with this member's reason. */}
          <ContextMenu.Sub>
            <ContextMenu.SubTrigger class="context-menu-item" disabled={mod().manageRoles !== null}
            title={mod().manageRoles ?? undefined}>
              <SkinGlyph name="shield" class="context-menu-icon" />
              Manage Roles
            </ContextMenu.SubTrigger>
            <ContextMenu.Portal>
              <ContextMenu.SubContent class="context-menu">
                <For each={assignableRoles()}>
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

          {/* Never-hide. */}
          <ContextMenu.Sub>
            <ContextMenu.SubTrigger class="context-menu-item" disabled={mod().timeout !== null}
            title={mod().timeout ?? undefined}>
              <SkinGlyph name="timeout" class="context-menu-icon" />
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

          {/* Never-hide. */}
          <ContextMenu.Item
            class="context-menu-item"
            disabled={mod().mute !== null}
            title={mod().mute ?? undefined}
            onSelect={() =>
              // No known channel goes as null: the Host's refusal says why.
              handleServerMuteMember(props.communityId, voiceChannelId(), member.pseudonymKey, true)
            }
          >
            <SkinGlyph name="mic.off" class="context-menu-icon" />
            Server Mute
          </ContextMenu.Item>
          {/* Never-hide. */}
          <ContextMenu.Item
            class="context-menu-item"
            disabled={mod().deafen !== null}
            title={mod().deafen ?? undefined}
            onSelect={() =>
              // No known channel goes as null: the Host's refusal says why.
              handleServerDeafenMember(props.communityId, voiceChannelId(), member.pseudonymKey, true)
            }
          >
            <SkinGlyph name="speaker.off" class="context-menu-icon" />
            Server Deafen
          </ContextMenu.Item>

          {/* Never-hide. */}
          <ContextMenu.Item
            class="context-menu-item context-menu-item-danger"
            disabled={mod().kick !== null}
            title={mod().kick ?? undefined}
            onSelect={() =>
              void handleRemoveCommunityMember(props.communityId, member.pseudonymKey)
            }
          >
            <SkinGlyph name="account-remove" class="context-menu-icon" />
            Kick Member
          </ContextMenu.Item>

          {/* Never-hide. */}
          <ContextMenu.Item
            class="context-menu-item context-menu-item-danger"
            disabled={mod().ban !== null}
            title={mod().ban ?? undefined}
            onSelect={() =>
              void handleBanMember(props.communityId, member.pseudonymKey)
            }
          >
            <SkinGlyph name="ban" class="context-menu-icon" />
            Ban Member
          </ContextMenu.Item>

          <ContextMenu.Item
            class="context-menu-item context-menu-item-danger"
            onSelect={() => void toggleBlock(member.pseudonymKey)}
          >
            <SkinGlyph name="ban" class="context-menu-icon" />
            {blocked().has(member.pseudonymKey.toLowerCase()) ? "Unblock" : "Block"}
          </ContextMenu.Item>
        </ContextMenu.Content>
      </ContextMenu.Portal>
    );
  }

  function renderMember(member: Member): JSX.Element {
    const isSelf = () => member.pseudonymKey === props.myPseudonymKey;
    const roles = () => memberRoles(member);
    const isOpen = () => openProfileFor() === member.pseudonymKey;

    const popover = () => (
      <Popover
        open={isOpen()}
        onOpenChange={(open) =>
          setOpenProfileFor(open ? member.pseudonymKey : null)
        }
      >
        <Popover.Trigger as="div" class="member-item" data-pseudonym={member.pseudonymKey}>
          <StatusDot status={memberStatus(member)} />
          <div class="member-name-info">
            <span class="member-name" title={member.fingerprint}>
              {member.displayName}
            </span>
            <Show when={member.customStatus}>
              {(text) => <span class="member-custom-status">{text()}</span>}
            </Show>
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
                {(role) => <RoleTag name={role.name} color={role.color} tagColours={role.tagColours} />}
              </For>
              <Show when={member.timeoutUntil}>
                <SkinGlyph name="timeout" class="timeout-indicator" title="Timed out" label="Timed out" />
              </Show>
            </div>
          </div>
        </Popover.Trigger>
        <MemberProfilePopup
          communityId={props.communityId}
          member={member}
          roles={props.roles}
          myPseudonymKey={props.myPseudonymKey}
        />
      </Popover>
    );

    return (
      <Show when={!isSelf()} fallback={popover()}>
        <ContextMenu>
          <ContextMenu.Trigger as="div">{popover()}</ContextMenu.Trigger>
          {memberMenu(member)}
        </ContextMenu>
      </Show>
    );
  }

  return (
    <div
      class="member-list"
      ref={(list) => {
        rovingList(".member-item")(list);
        onScreen(list);
      }}
    >
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
      <For each={filteredMembers()}>
        {(member) => (
          <>
            <Show when={groupHeads().get(member.pseudonymKey)}>
              {(name) => (
                <div class="member-group-header">
                  <span class="member-group-name">{name()}</span>
                </div>
              )}
            </Show>
            {renderMember(member)}
          </>
        )}
      </For>
    </div>
  );
};

export default MemberList;
