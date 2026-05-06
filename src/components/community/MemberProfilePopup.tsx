import { Component, Show, For, createEffect, createSignal } from "solid-js";
import { Popover } from "@kobalte/core/popover";
import LoadingButton from "../common/LoadingButton";
import type { Member, Role } from "../../stores/community.store";
import { communityState } from "../../stores/community.store";
import StatusDot from "../status/StatusDot";
import RoleTag from "./RoleTag";
import { truncateKey } from "../../utils/formatting";
import { commands } from "../../ipc/commands";
import {
  handleSelfAssignRole,
  handleSelfUnassignRole,
  handleUpdateCommunityProfile,
} from "../../handlers/community.handlers";
import { addToast } from "../../stores/toast.store";

interface MemberProfilePopupProps {
  communityId: string;
  member: Member;
  roles: Role[];
  onClose: () => void;
  myPseudonymKey?: string | null;
}

const MAX_BIO_LEN = 190;
const MAX_PRONOUNS_LEN = 32;
const MAX_BADGES = 8;
const MAX_BADGE_LEN = 32;

function formatColorHex(value: number | null | undefined): string {
  if (value == null) return "#3a8ee6";
  return `#${(value & 0xffffff).toString(16).padStart(6, "0")}`;
}

function parseColorHex(input: string): number | null {
  const trimmed = input.trim().replace(/^#/, "");
  if (trimmed.length !== 6) return null;
  const parsed = Number.parseInt(trimmed, 16);
  return Number.isFinite(parsed) ? parsed : null;
}

function parseBadges(input: string): string[] {
  return input
    .split(",")
    .map((b) => b.trim())
    .filter((b) => b.length > 0)
    .slice(0, MAX_BADGES);
}

/**
 * Architecture §32 Phase 5 W15 — per-community profile popover. Renders
 * inside a Kobalte `<Popover>` that the parent (MemberList) wraps around
 * the member row. Floating UI handles flip / shift / collision detection;
 * Esc + click-outside dismissal are built in.
 */
const MemberProfilePopup: Component<MemberProfilePopupProps> = (props) => {
  const [updatingRoleId, setUpdatingRoleId] = createSignal<number | null>(null);
  const [editing, setEditing] = createSignal(false);
  const [saving, setSaving] = createSignal(false);

  const myCommunity = () => communityState.communities[props.communityId];
  const initialBio = () => myCommunity()?.myBio ?? "";
  const initialPronouns = () => myCommunity()?.myPronouns ?? "";
  const initialThemeColorHex = () => formatColorHex(myCommunity()?.myThemeColor ?? null);
  const initialBadges = () => (myCommunity()?.myBadges ?? []).join(", ");

  const [bioDraft, setBioDraft] = createSignal(initialBio());
  const [pronounsDraft, setPronounsDraft] = createSignal(initialPronouns());
  const [themeColorDraft, setThemeColorDraft] = createSignal(initialThemeColorHex());
  const [badgesDraft, setBadgesDraft] = createSignal(initialBadges());

  const [avatarRefDraft, setAvatarRefDraft] = createSignal<string | null>(
    myCommunity()?.myAvatarRef ?? null,
  );
  const [bannerRefDraft, setBannerRefDraft] = createSignal<string | null>(
    myCommunity()?.myBannerRef ?? null,
  );
  const [avatarDataUrl, setAvatarDataUrl] = createSignal<string | null>(null);
  const [bannerDataUrl, setBannerDataUrl] = createSignal<string | null>(null);
  const [uploadingImage, setUploadingImage] = createSignal<"avatar" | "banner" | null>(null);

  createEffect(() => {
    const hash = avatarRefDraft();
    if (!hash) {
      setAvatarDataUrl(null);
      return;
    }
    void commands
      .getCommunityAvatarDataUrl(props.communityId, hash)
      .then((url) => setAvatarDataUrl(url))
      .catch(() => setAvatarDataUrl(null));
  });
  createEffect(() => {
    const hash = bannerRefDraft();
    if (!hash) {
      setBannerDataUrl(null);
      return;
    }
    void commands
      .getCommunityAvatarDataUrl(props.communityId, hash)
      .then((url) => setBannerDataUrl(url))
      .catch(() => setBannerDataUrl(null));
  });

  async function pickImage(kind: "avatar" | "banner"): Promise<void> {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/png,image/jpeg,image/webp,image/gif";
    input.style.display = "none";
    document.body.appendChild(input);
    input.onchange = async () => {
      const file = input.files?.[0];
      document.body.removeChild(input);
      if (!file) return;
      const buffer = await file.arrayBuffer();
      const bytes = Array.from(new Uint8Array(buffer));
      setUploadingImage(kind);
      try {
        const hash =
          kind === "avatar"
            ? await commands.setCommunityAvatar(props.communityId, bytes)
            : await commands.setCommunityBanner(props.communityId, bytes);
        if (kind === "avatar") setAvatarRefDraft(hash);
        else setBannerRefDraft(hash);
      } catch (e) {
        addToast(typeof e === "string" ? e : `Failed to upload ${kind}`, "error");
      } finally {
        setUploadingImage(null);
      }
    };
    input.click();
  }

  function clearImage(kind: "avatar" | "banner"): void {
    if (kind === "avatar") setAvatarRefDraft(null);
    else setBannerRefDraft(null);
  }

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

  function startEditing(): void {
    setBioDraft(initialBio());
    setPronounsDraft(initialPronouns());
    setThemeColorDraft(initialThemeColorHex());
    setBadgesDraft(initialBadges());
    setAvatarRefDraft(myCommunity()?.myAvatarRef ?? null);
    setBannerRefDraft(myCommunity()?.myBannerRef ?? null);
    setEditing(true);
  }

  function cancelEditing(): void {
    setEditing(false);
  }

  async function submitProfile(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    if (saving()) return;
    const bio = bioDraft().trim();
    if (bio.length > MAX_BIO_LEN) {
      addToast(`Bio exceeds ${MAX_BIO_LEN} characters`, "error");
      return;
    }
    const pronouns = pronounsDraft().trim();
    if (pronouns.length > MAX_PRONOUNS_LEN) {
      addToast(`Pronouns exceeds ${MAX_PRONOUNS_LEN} characters`, "error");
      return;
    }
    const badges = parseBadges(badgesDraft());
    if (badges.some((b) => b.length > MAX_BADGE_LEN)) {
      addToast(`Badge exceeds ${MAX_BADGE_LEN} characters`, "error");
      return;
    }
    const themeColor = parseColorHex(themeColorDraft());
    setSaving(true);
    try {
      const ok = await handleUpdateCommunityProfile(
        props.communityId,
        bio.length > 0 ? bio : null,
        pronouns.length > 0 ? pronouns : null,
        themeColor,
        badges,
        avatarRefDraft(),
        bannerRefDraft(),
      );
      if (ok) setEditing(false);
    } finally {
      setSaving(false);
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
        <div class="profile-popup-key">{truncateKey(props.member.pseudonymKey)}</div>
        <div class="profile-popup-status">
          <StatusDot status={props.member.status || "online"} />
          <span>{props.member.status || "online"}</span>
        </div>
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
        <Show when={isSelf()}>
          <Show
            when={editing()}
            fallback={
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
                <button class="profile-popup-action-btn" onClick={startEditing}>
                  Edit profile
                </button>
              </div>
            }
          >
            <form class="profile-popup-edit-form" onSubmit={(e) => void submitProfile(e)}>
              <label class="profile-popup-edit-label">
                <span>Avatar</span>
                <div class="profile-popup-edit-image-row">
                  <Show when={avatarDataUrl()} fallback={
                    <div class="profile-popup-edit-image-placeholder">No avatar</div>
                  }>
                    <img class="profile-popup-edit-avatar" src={avatarDataUrl()!} alt="Avatar preview" />
                  </Show>
                  <button
                    type="button"
                    class="profile-popup-edit-btn profile-popup-edit-btn-secondary"
                    onClick={() => void pickImage("avatar")}
                    disabled={uploadingImage() === "avatar"}
                    aria-label="Choose avatar image"
                  >
                    {uploadingImage() === "avatar" ? "Uploading…" : "Choose…"}
                  </button>
                  <Show when={avatarRefDraft()}>
                    <button
                      type="button"
                      class="profile-popup-edit-btn profile-popup-edit-btn-secondary"
                      onClick={() => clearImage("avatar")}
                      aria-label="Remove avatar"
                    >
                      Remove
                    </button>
                  </Show>
                </div>
              </label>
              <label class="profile-popup-edit-label">
                <span>Banner</span>
                <div class="profile-popup-edit-image-row">
                  <Show when={bannerDataUrl()} fallback={
                    <div class="profile-popup-edit-image-placeholder">No banner</div>
                  }>
                    <img class="profile-popup-edit-banner" src={bannerDataUrl()!} alt="Banner preview" />
                  </Show>
                  <button
                    type="button"
                    class="profile-popup-edit-btn profile-popup-edit-btn-secondary"
                    onClick={() => void pickImage("banner")}
                    disabled={uploadingImage() === "banner"}
                    aria-label="Choose banner image"
                  >
                    {uploadingImage() === "banner" ? "Uploading…" : "Choose…"}
                  </button>
                  <Show when={bannerRefDraft()}>
                    <button
                      type="button"
                      class="profile-popup-edit-btn profile-popup-edit-btn-secondary"
                      onClick={() => clearImage("banner")}
                      aria-label="Remove banner"
                    >
                      Remove
                    </button>
                  </Show>
                </div>
              </label>
              <label class="profile-popup-edit-label">
                <span>Pronouns</span>
                <input
                  class="profile-popup-edit-input"
                  type="text"
                  maxLength={MAX_PRONOUNS_LEN}
                  value={pronounsDraft()}
                  onInput={(e) => setPronounsDraft(e.currentTarget.value)}
                  placeholder="they/them"
                />
              </label>
              <label class="profile-popup-edit-label">
                <span>Bio</span>
                <textarea
                  class="profile-popup-edit-textarea"
                  maxLength={MAX_BIO_LEN}
                  rows={3}
                  value={bioDraft()}
                  onInput={(e) => setBioDraft(e.currentTarget.value)}
                  placeholder={`Up to ${MAX_BIO_LEN} characters`}
                />
              </label>
              <label class="profile-popup-edit-label">
                <span>Theme color</span>
                <input
                  class="profile-popup-edit-color"
                  type="color"
                  value={themeColorDraft()}
                  onInput={(e) => setThemeColorDraft(e.currentTarget.value)}
                />
              </label>
              <label class="profile-popup-edit-label">
                <span>Badges (comma-separated, max {MAX_BADGES})</span>
                <input
                  class="profile-popup-edit-input"
                  type="text"
                  value={badgesDraft()}
                  onInput={(e) => setBadgesDraft(e.currentTarget.value)}
                  placeholder="founder, mod, gamer"
                />
              </label>
              <div class="profile-popup-edit-actions">
                <LoadingButton
                  type="submit"
                  loading={saving()}
                  loadingLabel="Saving"
                  class="profile-popup-edit-btn"
                >
                  Save
                </LoadingButton>
                <button
                  type="button"
                  class="profile-popup-edit-btn profile-popup-edit-btn-secondary"
                  onClick={cancelEditing}
                  disabled={saving()}
                >
                  Cancel
                </button>
              </div>
            </form>
          </Show>
        </Show>
        <Show when={!isSelf()}>
          <div class="profile-popup-actions">
            <button class="profile-popup-action-btn" onClick={handleMessage}>Message</button>
            <button class="profile-popup-action-btn" onClick={handleAddFriend}>Add Friend</button>
            <button class="profile-popup-action-btn" onClick={handleCopyKey}>Copy Key</button>
          </div>
        </Show>
      </Popover.Content>
    </Popover.Portal>
  );
};

export default MemberProfilePopup;
