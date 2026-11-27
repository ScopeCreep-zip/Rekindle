import { Component, createEffect, createSignal, Show, untrack } from "solid-js";
import { handleUpdateCommunityProfile } from "../../actions/community.actions";
import { commands } from "../../ipc/commands";
import { asImageKind } from "../../ipc/commands/system";
import { communityState } from "../../stores/community.store";
import { addToast } from "../../stores/toast.store";
import { colorIntToHex, parseColorHex } from "../../utils/color";
import {
  colourField,
  editedField,
  listField,
  refField,
  textField,
} from "../../utils/profile_field";
import LoadingButton from "../common/LoadingButton";
import Modal from "../common/Modal";

interface CommunityProfileEditorProps {
  communityId: string;
  isOpen: boolean;
  onClose: () => void;
}

const MAX_BIO_LEN = 190;
const MAX_PRONOUNS_LEN = 32;
const MAX_BADGES = 8;
const MAX_BADGE_LEN = 32;

/** The editor falls back to the accent blue rather than black. */
function formatColorHex(value: number | null | undefined): string {
  return colorIntToHex(value, "#3a8ee6");
}

function parseBadges(input: string): string[] {
  return input
    .split(",")
    .map((b) => b.trim())
    .filter((b) => b.length > 0)
    .slice(0, MAX_BADGES);
}

/**
 * Our profile card in one community (plan PF2), in its own dialog: the
 * member popout's "Edit profile" opens it, as Discord's "Edit Server
 * Profile" does, so the popout never grows past the window (Kobalte places a
 * popover only when it opens). Our name here is the community settings'
 * (MyNameSection); this form leaves it unchanged.
 */
const CommunityProfileEditor: Component<CommunityProfileEditorProps> = (props) => {
  const [saving, setSaving] = createSignal(false);

  const myCommunity = () => communityState.communities[props.communityId];
  // The card as editing began (plan PF2): each field's edit is judged
  // against it, so a Host change to a field the user did not touch (a
  // MemberCardChanged mid-edit) survives the save.
  const takeBaseline = () => {
    const c = myCommunity();
    return {
      bio: c?.myBio ?? "",
      pronouns: c?.myPronouns ?? "",
      customStatus: c?.myCustomStatus ?? "",
      themeColor: c?.myThemeColor ?? null,
      badges: c?.myBadges ?? [],
      avatarRef: c?.myAvatarRef ?? null,
      bannerRef: c?.myBannerRef ?? null,
    };
  };
  let baseline = takeBaseline();
  const initialBio = () => baseline.bio;
  const initialPronouns = () => baseline.pronouns;
  const initialCustomStatus = () => baseline.customStatus;
  const initialThemeColorHex = () => formatColorHex(baseline.themeColor);
  const initialBadges = () => baseline.badges.join(", ");

  const [bioDraft, setBioDraft] = createSignal(initialBio());
  const [pronounsDraft, setPronounsDraft] = createSignal(initialPronouns());
  const [customStatusDraft, setCustomStatusDraft] = createSignal(initialCustomStatus());
  const [themeColorDraft, setThemeColorDraft] = createSignal(initialThemeColorHex());
  const [badgesDraft, setBadgesDraft] = createSignal(initialBadges());

  const [avatarRefDraft, setAvatarRefDraft] = createSignal<string | null>(baseline.avatarRef);
  const [bannerRefDraft, setBannerRefDraft] = createSignal<string | null>(baseline.bannerRef);
  const [avatarDataUrl, setAvatarDataUrl] = createSignal<string | null>(null);
  const [bannerDataUrl, setBannerDataUrl] = createSignal<string | null>(null);
  const [uploadingImage, setUploadingImage] = createSignal<"avatar" | "banner" | null>(null);

  // Each opening starts from the card as it is then, and only an opening:
  // the card is read untracked, so a Host change while the dialog is open
  // (a MemberCardChanged) never resets the drafts the user is typing (PF2).
  createEffect(() => {
    if (!props.isOpen) return;
    untrack(() => {
      baseline = takeBaseline();
      setBioDraft(initialBio());
      setPronounsDraft(initialPronouns());
      setCustomStatusDraft(initialCustomStatus());
      setThemeColorDraft(initialThemeColorHex());
      setBadgesDraft(initialBadges());
      setAvatarRefDraft(baseline.avatarRef);
      setBannerRefDraft(baseline.bannerRef);
    });
  });

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
    input.accept = "image/png,image/jpeg,image/gif";
    input.style.display = "none";
    document.body.appendChild(input);
    input.onchange = async () => {
      const file = input.files?.[0];
      document.body.removeChild(input);
      if (!file) return;
      const imageKind = asImageKind(file.type);
      if (!imageKind) {
        addToast("That picture can't be used: choose a PNG, JPEG, or GIF.", "error");
        return;
      }
      const buffer = await file.arrayBuffer();
      const bytes = Array.from(new Uint8Array(buffer));
      setUploadingImage(kind);
      try {
        const hash =
          kind === "avatar"
            ? await commands.setCommunityAvatar(props.communityId, bytes, imageKind)
            : await commands.setCommunityBanner(props.communityId, bytes, imageKind);
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
    // Each field against the card as editing began (PF2): unchanged, cleared
    // or set; the name is the settings', so this form leaves it unchanged.
    setSaving(true);
    try {
      const ok = await handleUpdateCommunityProfile(props.communityId, {
        bio: editedField(bioDraft(), initialBio(), () => textField(bio, baseline.bio)),
        pronouns: editedField(pronounsDraft(), initialPronouns(), () =>
          textField(pronouns, baseline.pronouns)
        ),
        themeColor: editedField(themeColorDraft(), initialThemeColorHex(), () =>
          colourField(themeColor, baseline.themeColor)
        ),
        badges: editedField(badgesDraft(), initialBadges(), () =>
          listField(badges, baseline.badges)
        ),
        avatarRef: refField(avatarRefDraft(), baseline.avatarRef),
        bannerRef: refField(bannerRefDraft(), baseline.bannerRef),
        customStatus: editedField(customStatusDraft(), initialCustomStatus(), () =>
          textField(customStatusDraft().trim(), baseline.customStatus)
        ),
        displayName: { op: "unchanged" },
      });
      if (ok) props.onClose();
    } finally {
      setSaving(false);
    }
  }

  return (
    <Modal isOpen={props.isOpen} title="Edit community profile" onClose={props.onClose} size="md">
      <form class="profile-editor-form" onSubmit={(e) => void submitProfile(e)}>
        <label class="profile-editor-label">
          <span>Avatar</span>
          <div class="profile-editor-image-row">
            <Show
              when={avatarDataUrl()}
              fallback={<div class="profile-editor-image-placeholder">No avatar</div>}
            >
              <img class="profile-editor-avatar" src={avatarDataUrl()!} alt="Avatar preview" />
            </Show>
            <button
              type="button"
              class="profile-editor-btn profile-editor-btn-secondary"
              onClick={() => void pickImage("avatar")}
              disabled={uploadingImage() === "avatar"}
              aria-label="Choose avatar image"
            >
              {uploadingImage() === "avatar" ? "Uploading…" : "Choose…"}
            </button>
            <Show when={avatarRefDraft()}>
              <button
                type="button"
                class="profile-editor-btn profile-editor-btn-secondary"
                onClick={() => clearImage("avatar")}
                aria-label="Remove avatar"
              >
                Remove
              </button>
            </Show>
          </div>
        </label>
        <label class="profile-editor-label">
          <span>Banner</span>
          <div class="profile-editor-image-row">
            <Show
              when={bannerDataUrl()}
              fallback={<div class="profile-editor-image-placeholder">No banner</div>}
            >
              <img class="profile-editor-banner" src={bannerDataUrl()!} alt="Banner preview" />
            </Show>
            <button
              type="button"
              class="profile-editor-btn profile-editor-btn-secondary"
              onClick={() => void pickImage("banner")}
              disabled={uploadingImage() === "banner"}
              aria-label="Choose banner image"
            >
              {uploadingImage() === "banner" ? "Uploading…" : "Choose…"}
            </button>
            <Show when={bannerRefDraft()}>
              <button
                type="button"
                class="profile-editor-btn profile-editor-btn-secondary"
                onClick={() => clearImage("banner")}
                aria-label="Remove banner"
              >
                Remove
              </button>
            </Show>
          </div>
        </label>
        <label class="profile-editor-label">
          <span>Custom status</span>
          <input
            class="profile-editor-input"
            type="text"
            value={customStatusDraft()}
            onInput={(e) => setCustomStatusDraft(e.currentTarget.value)}
            placeholder="What are you up to?"
          />
        </label>
        <label class="profile-editor-label">
          <span>Pronouns</span>
          <input
            class="profile-editor-input"
            type="text"
            maxLength={MAX_PRONOUNS_LEN}
            value={pronounsDraft()}
            onInput={(e) => setPronounsDraft(e.currentTarget.value)}
            placeholder="they/them"
          />
        </label>
        <label class="profile-editor-label">
          <span>Bio</span>
          <textarea
            class="profile-editor-textarea"
            maxLength={MAX_BIO_LEN}
            rows={3}
            value={bioDraft()}
            onInput={(e) => setBioDraft(e.currentTarget.value)}
            placeholder={`Up to ${MAX_BIO_LEN} characters`}
          />
        </label>
        <label class="profile-editor-label">
          <span>Theme color</span>
          <input
            class="profile-editor-color"
            type="color"
            value={themeColorDraft()}
            onInput={(e) => setThemeColorDraft(e.currentTarget.value)}
          />
        </label>
        <label class="profile-editor-label">
          <span>Badges (comma-separated, max {MAX_BADGES})</span>
          <input
            class="profile-editor-input"
            type="text"
            value={badgesDraft()}
            onInput={(e) => setBadgesDraft(e.currentTarget.value)}
            placeholder="founder, mod, gamer"
          />
        </label>
        <div class="profile-editor-actions">
          <LoadingButton
            type="submit"
            loading={saving()}
            loadingLabel="Saving"
            class="profile-editor-btn"
          >
            Save
          </LoadingButton>
          <button
            type="button"
            class="profile-editor-btn profile-editor-btn-secondary"
            onClick={props.onClose}
            disabled={saving()}
          >
            Cancel
          </button>
        </div>
      </form>
    </Modal>
  );
};

export default CommunityProfileEditor;
