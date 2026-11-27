import { type Component, Show, createEffect, createSignal } from "solid-js";
import { commands } from "../../ipc/commands";
import { addToast } from "../../stores/toast.store";

interface FriendNicknameFieldProps {
  publicKey: string;
  /** The alias the Host holds for this friend, or null. */
  nickname: string | null;
}

/**
 * Our private alias for one friend (plan E7.15): the Host's
 * `FriendSetNickname`, which validates the text and is reviewed by the
 * intent gate; the buddy list then shows the alias from the Host's friend
 * update. Clear sends no alias.
 */
const FriendNicknameField: Component<FriendNicknameFieldProps> = (props) => {
  const [value, setValue] = createSignal("");
  const [saving, setSaving] = createSignal(false);

  createEffect(() => setValue(props.nickname ?? ""));

  async function write(nickname: string | null): Promise<void> {
    setSaving(true);
    try {
      await commands.setFriendNickname(props.publicKey, nickname);
      addToast(nickname === null ? "Nickname cleared" : "Nickname saved", "success");
    } catch (e) {
      console.error("Failed to set nickname:", e);
      addToast(typeof e === "string" ? e : "Failed to set nickname", "error");
    } finally {
      setSaving(false);
    }
  }

  return (
    <div class="profile-section">
      <label class="profile-section-label" for="friend-nickname">Nickname (only you see it)</label>
      <div class="settings-list-actions">
        <input
          id="friend-nickname"
          class="form-input"
          value={value()}
          onInput={(e) => setValue(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && e.currentTarget.value !== "") void write(e.currentTarget.value);
          }}
        />
        <button
          type="button"
          class="form-btn-primary"
          disabled={saving() || value() === ""}
          onClick={() => void write(value())}
        >
          Save
        </button>
        <Show when={props.nickname !== null}>
          <button type="button" class="form-btn-secondary" disabled={saving()} onClick={() => void write(null)}>
            Clear
          </button>
        </Show>
      </div>
    </div>
  );
};

export default FriendNicknameField;
