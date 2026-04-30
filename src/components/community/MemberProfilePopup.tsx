import { Component, Show, For, onMount, onCleanup, createSignal } from "solid-js";
import type { Member, Role } from "../../stores/community.store";
import StatusDot from "../status/StatusDot";
import RoleTag from "./RoleTag";
import { truncateKey } from "../../utils/formatting";
import { commands } from "../../ipc/commands";
import { handleSelfAssignRole, handleSelfUnassignRole } from "../../handlers/community.handlers";
import { addToast } from "../../stores/toast.store";

interface MemberProfilePopupProps {
  communityId: string;
  member: Member;
  roles: Role[];
  x: number;
  y: number;
  onClose: () => void;
  myPseudonymKey?: string | null;
}

const MemberProfilePopup: Component<MemberProfilePopupProps> = (props) => {
  let popupRef: HTMLDivElement | undefined;
  const [clampedLeft, setClampedLeft] = createSignal(props.x);
  const [clampedTop, setClampedTop] = createSignal(props.y);
  const [updatingRoleId, setUpdatingRoleId] = createSignal<number | null>(null);

  function handleClickOutside(e: MouseEvent): void {
    if (popupRef && !popupRef.contains(e.target as Node)) {
      props.onClose();
    }
  }

  onMount(() => {
    // Defer listener so the opening click doesn't immediately close
    requestAnimationFrame(() => {
      document.addEventListener("mousedown", handleClickOutside);
      // Clamp position after first render when dimensions are known
      if (popupRef) {
        const rect = popupRef.getBoundingClientRect();
        setClampedLeft(Math.min(props.x, window.innerWidth - rect.width - 8));
        setClampedTop(Math.min(props.y, window.innerHeight - rect.height - 8));
      }
    });
  });

  onCleanup(() => {
    document.removeEventListener("mousedown", handleClickOutside);
  });

  const memberRoles = () =>
    props.roles
      .filter((r) => props.member.roleIds.includes(r.id) && r.id !== 0 && r.name !== "@everyone")
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
      .filter((role) => role.selfAssignable && role.id !== 0 && role.name !== "@everyone")
      .sort((a, b) => b.position - a.position);

  function handleMessage(): void {
    commands.openChatWindow(props.member.pseudonymKey, props.member.displayName);
    props.onClose();
  }

  function handleAddFriend(): void {
    commands.addFriend(props.member.pseudonymKey, props.member.displayName, "");
    props.onClose();
  }

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

  return (
    <div
      ref={popupRef}
      class="profile-popup"
      style={{ left: `${clampedLeft()}px`, top: `${clampedTop()}px` }}
    >
      <div class="profile-popup-name">{props.member.displayName}</div>
      <div class="profile-popup-key">{truncateKey(props.member.pseudonymKey)}</div>
      <div class="profile-popup-status">
        <StatusDot status={props.member.status || "online"} />
        <span>{props.member.status || "online"}</span>
      </div>
      <Show when={props.member.gameInfo}>
        {(info) => (
          <div class="profile-popup-game">
            <span class="profile-popup-game-name">{info().gameName}</span>
            <Show when={info().serverAddress}>
              <span class="profile-popup-game-server">{info().serverAddress}</span>
              <button
                class="profile-popup-join-btn"
                onClick={() => commands.launchGameToServer(info().gameId!, info().serverAddress!)}
              >
                Join Game
              </button>
            </Show>
          </div>
        )}
      </Show>
      <Show when={memberRoles().length > 0}>
        <div class="profile-popup-roles">
          <For each={memberRoles()}>
            {(role) => <RoleTag name={role.name} color={role.color} />}
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
      <Show when={!isSelf()}>
        <div class="profile-popup-actions">
          <button class="profile-popup-action-btn" onClick={handleMessage}>Message</button>
          <button class="profile-popup-action-btn" onClick={handleAddFriend}>Add Friend</button>
          <button class="profile-popup-action-btn" onClick={handleCopyKey}>Copy Key</button>
        </div>
      </Show>
    </div>
  );
};

export default MemberProfilePopup;
