import { Component, For, Show, createSignal } from "solid-js";
import StatusDot from "../../status/StatusDot";
import RoleTag from "../RoleTag";
import type { Community, Member, Role } from "../../../stores/community.store";
import type { ConfirmOptions } from "../CommunitySettingsModal";
import {
  handleRemoveCommunityMember,
  handleBanMember,
  handleAssignRole,
  handleUnassignRole,
  handleTimeoutMember,
  handleRemoveTimeout,
  handleExpandCommunitySegment,
} from "../../../handlers/community.handlers";
import { highestPosition } from "../../../ipc/permissions";
import {
  ICON_SHIELD,
  ICON_TIMEOUT,
  ICON_ACCOUNT_REMOVE,
  ICON_BAN,
} from "../../../icons";

interface MembersTabProps {
  community: Community;
  myRoleIds: number[];
  canManageRoles: boolean;
  canKick: boolean;
  canBan: boolean;
  canModerate: boolean;
  canManageCommunity: boolean;
  requestConfirm: (opts: ConfirmOptions) => void;
}

const SLOTS_PER_SEGMENT = 255;
const MAX_SEGMENTS = 8;

const MembersTab: Component<MembersTabProps> = (props) => {
  const [rolePickerTarget, setRolePickerTarget] = createSignal<string | null>(null);
  const [searchQuery, setSearchQuery] = createSignal("");

  const filteredMembers = () => {
    const q = searchQuery().toLowerCase().trim();
    if (!q) return props.community.members;
    return props.community.members.filter((m) =>
      m.displayName.toLowerCase().includes(q),
    );
  };

  function toggleMemberRole(pseudonymKey: string, roleId: number, has: boolean): void {
    if (has) {
      handleUnassignRole(props.community.id, pseudonymKey, roleId);
    } else {
      handleAssignRole(props.community.id, pseudonymKey, roleId);
    }
  }

  function memberAllRoles(member: Member): { name: string; color: number }[] {
    return member.roleIds
      .map((id) => props.community.roles.find((r) => r.id === id))
      .filter((r): r is Role => r != null && r.name !== "@everyone")
      .sort((a, b) => b.position - a.position)
      .map((r) => ({ name: r.name, color: r.color }));
  }

  function canManageMember(member: Member): boolean {
    const myPos = highestPosition(props.myRoleIds, props.community.roles);
    const memberPos = highestPosition(member.roleIds, props.community.roles);
    return myPos > memberPos && member.pseudonymKey !== props.community.myPseudonymKey;
  }

  function confirmKick(member: Member): void {
    props.requestConfirm({
      title: "Kick Member",
      message: `Kick ${member.displayName} from the community?`,
      confirmLabel: "Kick",
      action: () => handleRemoveCommunityMember(props.community.id, member.pseudonymKey),
    });
  }

  function confirmBan(member: Member): void {
    props.requestConfirm({
      title: "Ban Member",
      message: `Ban ${member.displayName}? They will not be able to rejoin.`,
      confirmLabel: "Ban",
      action: () => handleBanMember(props.community.id, member.pseudonymKey),
    });
  }

  // Plate Gate (architecture §15): admins see two-state banner — a
  // proactive `warning` at ≤10 slots remaining ("Approaching segment
  // limit"), and an `alert` once full ("Segment full — expand"). The
  // backend validates the actual condition before writing SegmentAdded;
  // the banner here is purely affordance.
  const PLATE_WARNING_REMAINING = 10;
  const segmentCount = () => 1 + (props.community.segments?.length ?? 0);
  const capacity = () => segmentCount() * SLOTS_PER_SEGMENT;
  const remaining = () => Math.max(0, capacity() - props.community.members.length);
  const canExpandMore = () => segmentCount() < MAX_SEGMENTS;
  const expansionBannerVariant = (): "warning" | "alert" | null => {
    if (!props.canManageCommunity || !canExpandMore()) return null;
    if (remaining() === 0) return "alert";
    if (remaining() <= PLATE_WARNING_REMAINING) return "warning";
    return null;
  };
  const [expanding, setExpanding] = createSignal(false);

  async function onClickExpand(): Promise<void> {
    if (expanding()) return;
    setExpanding(true);
    try {
      await handleExpandCommunitySegment(props.community.id);
    } finally {
      setExpanding(false);
    }
  }

  return (
    <div class="settings-section">
      <Show when={expansionBannerVariant() !== null}>
        <div
          class={`plate-gate-banner plate-gate-banner-${expansionBannerVariant()}`}
          role={expansionBannerVariant() === "alert" ? "alert" : "status"}
        >
          <div class="plate-gate-banner-text">
            <Show
              when={expansionBannerVariant() === "alert"}
              fallback={
                <>
                  <strong>Approaching segment limit ({remaining()} slots left).</strong>{" "}
                  Expand to a new SMPL segment so the community can keep admitting
                  members. Each segment holds up to {SLOTS_PER_SEGMENT} more members.
                </>
              }
            >
              <strong>This community has reached its segment capacity.</strong>{" "}
              Expand to a new SMPL segment to admit more members. Each segment
              holds up to {SLOTS_PER_SEGMENT} additional members.
            </Show>
          </div>
          <button
            class="plate-gate-banner-btn"
            disabled={expanding()}
            onClick={() => void onClickExpand()}
          >
            {expanding() ? "Expanding…" : "Expand community"}
          </button>
        </div>
      </Show>
      <Show when={props.canManageCommunity && (props.community.segments?.length ?? 0) > 0}>
        <div class="plate-gate-status">
          Plate Gate: {segmentCount()} segments · capacity {capacity()} ·
          {" "}{props.community.members.length} members ·{" "}
          {remaining()} slots open
        </div>
      </Show>
      <div class="member-list-header">
        Members — {props.community.members.length}
      </div>
      <input
        class="form-input member-search-input"
        type="text"
        placeholder="Search members..."
        value={searchQuery()}
        onInput={(e) => setSearchQuery(e.currentTarget.value)}
      />
      <For each={filteredMembers()}>
        {(member) => (
          <div class="settings-member-row">
            <StatusDot status={member.status || "online"} />
            <div class="member-name-info">
              <span class="member-name">{member.displayName}</span>
              <div class="member-roles-row">
                <For each={memberAllRoles(member)}>
                  {(role) => <RoleTag name={role.name} color={role.color} />}
                </For>
              </div>
              <Show when={member.timeoutUntil}>
                <span class="nf-icon timeout-indicator" title="Timed out">{ICON_TIMEOUT}</span>
              </Show>
            </div>
            <Show when={canManageMember(member)}>
              <div class="settings-member-actions">
                <Show when={props.canManageRoles}>
                  <button
                    class="form-btn-secondary"
                    onClick={() => setRolePickerTarget(
                      rolePickerTarget() === member.pseudonymKey ? null : member.pseudonymKey
                    )}
                  >
                    <span class="nf-icon">{ICON_SHIELD}</span> Roles
                  </button>
                </Show>
                <Show when={props.canModerate}>
                  <Show when={!member.timeoutUntil} fallback={
                    <button
                      class="form-btn-secondary"
                      onClick={() => handleRemoveTimeout(props.community.id, member.pseudonymKey)}
                    >
                      <span class="nf-icon">{ICON_TIMEOUT}</span> Untimeout
                    </button>
                  }>
                    <select
                      class="timeout-select"
                      onChange={(e) => {
                        const secs = Number(e.currentTarget.value);
                        if (secs > 0) {
                          handleTimeoutMember(props.community.id, member.pseudonymKey, secs, null);
                          e.currentTarget.value = "";
                        }
                      }}
                    >
                      <option value="">Timeout...</option>
                      <option value="60">1 min</option>
                      <option value="300">5 min</option>
                      <option value="600">10 min</option>
                      <option value="1800">30 min</option>
                      <option value="3600">1 hour</option>
                      <option value="86400">1 day</option>
                    </select>
                  </Show>
                </Show>
                <Show when={props.canKick}>
                  <button
                    class="form-btn-secondary"
                    onClick={() => confirmKick(member)}
                  >
                    <span class="nf-icon">{ICON_ACCOUNT_REMOVE}</span> Kick
                  </button>
                </Show>
                <Show when={props.canBan}>
                  <button
                    class="form-btn-danger"
                    onClick={() => confirmBan(member)}
                  >
                    <span class="nf-icon">{ICON_BAN}</span> Ban
                  </button>
                </Show>
              </div>
            </Show>
            <Show when={rolePickerTarget() === member.pseudonymKey}>
              <div class="role-picker-list settings-role-picker">
                <For each={props.community.roles.filter((r) => r.id !== 0).sort((a, b) => b.position - a.position)}>
                  {(role) => {
                    const has = () => member.roleIds.includes(role.id);
                    return (
                      <div
                        class={`role-picker-item ${has() ? "role-picker-item-active" : ""}`}
                        onClick={() => toggleMemberRole(member.pseudonymKey, role.id, has())}
                      >
                        <input type="checkbox" class="role-picker-checkbox" checked={has()} readOnly />
                        {role.name}
                      </div>
                    );
                  }}
                </For>
              </div>
            </Show>
          </div>
        )}
      </For>
    </div>
  );
};

export default MembersTab;
