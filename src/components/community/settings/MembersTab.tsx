import { Component, For, Show, createSignal } from "solid-js";
import SkinGlyph from "../../common/SkinGlyph";
import StatusDot from "../../status/StatusDot";
import RoleTag from "../RoleTag";
import GovernancePublicationStatus from "../GovernancePublicationStatus";
import type { Community, Member, Role } from "../../../stores/community.store";
import { EVERYONE_ROLE_ID } from "../../../stores/community.store";
import {
  handleRemoveCommunityMember,
  handleBanMember,
  handleAssignRole,
  handleUnassignRole,
  handleTimeoutMember,
  handleRemoveTimeout,
  handleExpandCommunitySegment,
} from "../../../actions/community.actions";

interface MembersTabProps {
  community: Community;
  /** Why we may not expand the community, in the Host's words, or null (never-hide). */
  manageDenied: string | null;
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

  function toggleMemberRole(pseudonymKey: string, roleId: string, has: boolean): void {
    if (has) {
      handleUnassignRole(props.community.id, pseudonymKey, roleId);
    } else {
      handleAssignRole(props.community.id, pseudonymKey, roleId);
    }
  }

  function memberAllRoles(member: Member): Role[] {
    return member.roleIds
      .map((id) => props.community.roles.find((r) => r.id === id))
      .filter((r): r is Role => r != null && r.name !== "@everyone")
      .sort((a, b) => b.position - a.position);
  }

  // What we may do to a member is the Host's per-member verdict (plan
  // E3.2 S9): the validator's rank rule, never a rank compared here.
  function kickMember(member: Member): void {
    void handleRemoveCommunityMember(props.community.id, member.pseudonymKey);
  }

  function banMember(member: Member): void {
    void handleBanMember(props.community.id, member.pseudonymKey);
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
    if (!canExpandMore()) return null;
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
      <GovernancePublicationStatus communityId={props.community.id} />
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
            disabled={expanding() || props.manageDenied !== null}
            title={props.manageDenied ?? undefined}
            aria-description={props.manageDenied ?? undefined}
            onClick={() => void onClickExpand()}
          >
            {expanding() ? "Expanding…" : "Expand community"}
          </button>
        </div>
      </Show>
      <Show when={(props.community.segments?.length ?? 0) > 0}>
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
                  {(role) => <RoleTag name={role.name} color={role.color} tagColours={role.tagColours} />}
                </For>
              </div>
              <Show when={member.timeoutUntil}>
                <SkinGlyph name="timeout" class="timeout-indicator" title="Timed out" label="Timed out" />
              </Show>
            </div>
            {/* Never-hide: every action is drawn for every member, each disabled
                with the Host's reason for this member (rank, permission, self). */}
            <div class="settings-member-actions">
              <button
                class="form-btn-secondary"
                disabled={member.moderation.manageRoles !== null}
                title={member.moderation.manageRoles ?? undefined}
                aria-description={member.moderation.manageRoles ?? undefined}
                onClick={() => setRolePickerTarget(
                  rolePickerTarget() === member.pseudonymKey ? null : member.pseudonymKey
                )}
              >
                <SkinGlyph name="shield" /> Roles
              </button>
              <Show when={!member.timeoutUntil} fallback={
                <button
                  class="form-btn-secondary"
                  disabled={member.moderation.timeout !== null}
                  title={member.moderation.timeout ?? undefined}
                  aria-description={member.moderation.timeout ?? undefined}
                  onClick={() => handleRemoveTimeout(props.community.id, member.pseudonymKey)}
                >
                  <SkinGlyph name="timeout" /> Untimeout
                </button>
              }>
                <select
                  class="timeout-select"
                  disabled={member.moderation.timeout !== null}
                  title={member.moderation.timeout ?? undefined}
                  aria-description={member.moderation.timeout ?? undefined}
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
              <button
                class="form-btn-secondary"
                disabled={member.moderation.kick !== null}
                title={member.moderation.kick ?? undefined}
                aria-description={member.moderation.kick ?? undefined}
                onClick={() => kickMember(member)}
              >
                <SkinGlyph name="account-remove" /> Kick
              </button>
              <button
                class="form-btn-danger"
                disabled={member.moderation.ban !== null}
                title={member.moderation.ban ?? undefined}
                aria-description={member.moderation.ban ?? undefined}
                onClick={() => banMember(member)}
              >
                <SkinGlyph name="ban" /> Ban
              </button>
            </div>
            <Show when={rolePickerTarget() === member.pseudonymKey}>
              <div class="role-picker-list settings-role-picker">
                <For each={props.community.roles.filter((r) => r.id !== EVERYONE_ROLE_ID).sort((a, b) => b.position - a.position)}>
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
