import { Component, Show, For, createSignal } from "solid-js";
import { colorIntToHex } from "../../utils/color";
import { Popover } from "@kobalte/core/popover";
import type { Member, Role } from "../../stores/community.store";
import type { GameInfo } from "../../ipc/commands/dto";
import { EVERYONE_ROLE_ID, communityState } from "../../stores/community.store";
import StatusDot from "../status/StatusDot";
import RoleTag from "./RoleTag";
import { handleLaunchGame } from "../../actions/game.actions";
import { notJoinableText } from "../../utils/joinable_text";
import { handleSelfAssignRole, handleSelfUnassignRole } from "../../actions/community.actions";
import { addToast } from "../../stores/toast.store";

interface MemberProfilePopupProps {
  communityId: string;
  member: Member;
  roles: Role[];
  myPseudonymKey?: string | null;
  /** Open our community profile editor (its own dialog; the popout never grows). */
  onEditProfile: () => void;
}

/** This popup falls back to the accent blue rather than black. */
function formatColorHex(value: number | null | undefined): string {
  return colorIntToHex(value, "#3a8ee6");
}

/** Coarse relative last-seen. The value is already bucketed by the peer's
 * sharing policy (hour granularity unless they opted into exact), so this
 * only renders it relative — never finer than the data they shared. */
function formatLastSeen(secs: number): string {
  const delta = Math.floor(Date.now() / 1000) - secs;
  if (delta < 90) return "just now";
  if (delta < 3600) return `${Math.round(delta / 60)} minutes ago`;
  if (delta < 86400) return `${Math.round(delta / 3600)} hours ago`;
  if (delta < 604800) return `${Math.round(delta / 86400)} days ago`;
  return new Date(secs * 1000).toLocaleDateString();
}

/**
 * Architecture §32 Phase 5 W15 — per-community profile popover. Renders
 * inside a Kobalte `<Popover>` that the parent (MemberList) wraps around
 * the member row. Floating UI handles flip / shift / collision detection;
 * Esc + click-outside dismissal are built in.
 */
/** Join a member's game server through the Host's review; a refusal is shown. */
async function joinMemberGame(info: GameInfo): Promise<void> {
  if (!info.joinable || info.gameId === null || info.serverAddress === null) return;
  await handleLaunchGame(info.gameId, info.serverAddress, "friendPresence");
}

const MemberProfilePopup: Component<MemberProfilePopupProps> = (props) => {
  const [updatingRoleId, setUpdatingRoleId] = createSignal<string | null>(null);

  const myCommunity = () => communityState.communities[props.communityId];
  const memberRoles = () =>
    props.roles
      .filter((r) => props.member.roleIds.includes(r.id) && r.id !== EVERYONE_ROLE_ID && r.name !== "@everyone")
      .sort((a, b) => b.position - a.position);

  function formatTimeout(timestamp: number | null): string | null {
    if (!timestamp) return null;
    const d = new Date(timestamp * 1000);
    if (d.getTime() < Date.now()) return null;
    return `Timed out until ${d.toLocaleString()}`;
  }

  const isSelf = () => props.myPseudonymKey === props.member.pseudonymKey;
  const selfAssignableRoles = () =>
    props.roles
      .filter((role) => role.selfAssignable && role.id !== EVERYONE_ROLE_ID && role.name !== "@everyone")
      .sort((a, b) => b.position - a.position);

  function handleCopyKey(): void {
    navigator.clipboard.writeText(props.member.pseudonymKey);
  }

  async function handleToggleSelfRole(role: Role): Promise<void> {
    const hasRole = props.member.roleIds.includes(role.id);
    setUpdatingRoleId(role.id);
    try {
      if (hasRole) {
        await handleSelfUnassignRole(props.communityId, role.id);
        addToast(`Removed ${role.name}`, "success");
      } else {
        await handleSelfAssignRole(props.communityId, role.id);
        addToast(`Assigned ${role.name}`, "success");
      }
    } finally {
      setUpdatingRoleId(null);
    }
  }

  const accentStyle = () =>
    props.member.themeColor != null
      ? { "--profile-accent": formatColorHex(props.member.themeColor) }
      : {};

  return (
    <Popover.Portal>
      <Popover.Content class="profile-popup" style={accentStyle()}>
        <div class="profile-popup-name">{props.member.displayName}</div>
        <div class="profile-popup-key">{props.member.fingerprint}</div>
        <div class="profile-popup-status">
          <StatusDot status={props.member.status || "online"} />
          <span>{props.member.status || "online"}</span>
        </div>
        <Show when={props.member.status === "offline" && (props.member.lastActive ?? 0) > 0}>
          <div class="profile-popup-lastseen">Last seen {formatLastSeen(props.member.lastActive!)}</div>
        </Show>
        <Show when={props.member.customStatus}>
          <div class="profile-popup-custom-status">{props.member.customStatus}</div>
        </Show>
        <Show when={props.member.pronouns}>
          <div class="profile-popup-pronouns">{props.member.pronouns}</div>
        </Show>
        <Show when={props.member.bio}>
          <div class="profile-popup-bio">{props.member.bio}</div>
        </Show>
        <Show when={(props.member.badges?.length ?? 0) > 0}>
          <div class="profile-popup-badges">
            <For each={props.member.badges}>
              {(badge) => <span class="profile-popup-badge">{badge}</span>}
            </For>
          </div>
        </Show>
        {/* The member's game, and Join Game always listed (owner: no button
         *  is hidden): enabled only while the Host states the game joinable,
         *  otherwise saying why, "Not in a game" included. */}
        <div class="profile-popup-game">
          <Show when={props.member.gameInfo}>
            {(info) => (
              <>
                <span class="profile-popup-game-name">{info().gameName}</span>
                <Show when={info().serverAddress}>
                  <span class="profile-popup-game-server">{info().serverAddress}</span>
                </Show>
              </>
            )}
          </Show>
          <button
            class="profile-popup-join-btn"
            disabled={props.member.gameInfo?.joinable !== true}
            title={props.member.gameInfo?.joinable === true ? undefined : notJoinableText(props.member.gameInfo)}
            aria-describedby={props.member.gameInfo?.joinable === true ? undefined : "profile-popup-join-reason"}
            onClick={() => {
              const info = props.member.gameInfo;
              if (info) void joinMemberGame(info);
            }}
          >
            Join Game
          </button>
          <Show when={props.member.gameInfo?.joinable !== true}>
            <span class="profile-popup-join-reason" id="profile-popup-join-reason">
              {notJoinableText(props.member.gameInfo)}
            </span>
          </Show>
        </div>
        <Show when={memberRoles().length > 0}>
          <div class="profile-popup-roles">
            <For each={memberRoles()}>
              {(role) => <RoleTag name={role.name} color={role.color} tagColours={role.tagColours} />}
            </For>
          </div>
        </Show>
        <Show when={isSelf() && selfAssignableRoles().length > 0}>
          <div class="profile-popup-self-roles">
            <div class="profile-popup-self-roles-title">Self-assignable roles</div>
            <For each={selfAssignableRoles()}>
              {(role) => {
                const hasRole = () => props.member.roleIds.includes(role.id);
                return (
                  <button
                    class={`profile-popup-self-role-btn ${hasRole() ? "profile-popup-self-role-btn-active" : ""}`}
                    onClick={() => void handleToggleSelfRole(role)}
                    disabled={updatingRoleId() === role.id}
                  >
                    <span>{hasRole() ? "Remove" : "Get"} {role.name}</span>
                    <Show when={updatingRoleId() === role.id}>
                      <span>...</span>
                    </Show>
                  </button>
                );
              }}
            </For>
          </div>
        </Show>
        <Show when={formatTimeout(props.member.timeoutUntil)}>
          {(msg) => <div class="profile-popup-timeout">{msg()}</div>}
        </Show>
        <Show when={isSelf()}>
          <div class="profile-popup-actions">
            <Show
              when={
                (myCommunity()?.myBio?.trim().length ?? 0) === 0
                && (myCommunity()?.myPronouns?.trim().length ?? 0) === 0
                && !myCommunity()?.myAvatarRef
                && !myCommunity()?.myBannerRef
              }
            >
              <div class="profile-popup-empty-hint">
                Add a bio so peers in this community know who you are. Your
                per-community profile is independent of your global identity.
              </div>
            </Show>
            <button class="profile-popup-action-btn" onClick={() => props.onEditProfile()}>
              Edit profile
            </button>
          </div>
        </Show>
        <Show when={!isSelf()}>
          <div class="profile-popup-actions">
            <button class="profile-popup-action-btn" onClick={handleCopyKey}>Copy Key</button>
          </div>
        </Show>
      </Popover.Content>
    </Popover.Portal>
  );
};

export default MemberProfilePopup;
