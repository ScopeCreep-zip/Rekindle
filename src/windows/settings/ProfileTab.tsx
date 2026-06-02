import { Component, createSignal, createEffect } from "solid-js";
import Avatar from "../../components/common/Avatar";
import FormField from "../../components/common/FormField";
import { authState, setAuthState } from "../../stores/auth.store";
import { handleSetAvatar } from "../../handlers/settings.handlers";
import { commands } from "../../ipc/commands";
import { fetchAvatarUrl } from "../../ipc/avatar";

const ProfileTab: Component = () => {
  const [nameInput, setNameInput] = createSignal("");
  const [statusMsgInput, setStatusMsgInput] = createSignal("");

  // Seed the name input from the hydrated identity. `displayName` only
  // changes on explicit save (or initial hydrate), so this never clobbers
  // an unsaved edit.
  createEffect(() => setNameInput(authState.displayName ?? ""));

  function handleSaveName(): void {
    const name = nameInput().trim();
    if (name && name !== authState.displayName) {
      setAuthState("displayName", name);
      commands.setNickname(name).catch((e) => {
        console.error("Failed to set nickname:", e);
      });
    }
  }

  function handleSaveStatusMessage(): void {
    const msg = statusMsgInput().trim();
    commands.setStatusMessage(msg).catch((e) => {
      console.error("Failed to set status message:", e);
    });
  }

  async function handleAvatarUpload(): Promise<void> {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/png, image/jpeg, image/gif";
    input.style.display = "none";
    document.body.appendChild(input);
    input.onchange = async () => {
      const file = input.files?.[0];
      document.body.removeChild(input);
      if (!file) return;
      const arrayBuffer = await file.arrayBuffer();
      const bytes = Array.from(new Uint8Array(arrayBuffer));
      await handleSetAvatar(bytes);
      // Refresh avatar in store after upload
      if (authState.publicKey) {
        const avatarUrl = await fetchAvatarUrl(authState.publicKey);
        setAuthState("avatarUrl", avatarUrl);
      }
    };
    input.click();
  }

  return (
    <>
      <div class="settings-section-title">Avatar</div>
      <div class="avatar-upload-section">
        <Avatar displayName={authState.displayName ?? "?"} size={64} avatarUrl={authState.avatarUrl ?? undefined} />
        <button class="avatar-upload-btn" onClick={handleAvatarUpload}>
          Change Avatar
        </button>
        <span class="avatar-upload-hint">PNG, JPEG, or GIF (max 256KB)</span>
      </div>
      <div class="settings-section-title">Display Name</div>
      <FormField>
        <div class="form-field-row">
          <input
            class="form-input"
            type="text"
            value={nameInput()}
            onInput={(e: InputEvent) => setNameInput((e.target as HTMLInputElement).value)}
            onKeyDown={(e: KeyboardEvent) => { if (e.key === "Enter") handleSaveName(); }}
          />
          <button class="form-btn-primary" onClick={handleSaveName}>Save</button>
        </div>
      </FormField>
      <div class="settings-section-title">Status Message</div>
      <FormField>
        <div class="form-field-row">
          <input
            class="form-input"
            type="text"
            placeholder="What's on your mind?"
            value={statusMsgInput()}
            onInput={(e: InputEvent) => setStatusMsgInput((e.target as HTMLInputElement).value)}
            onKeyDown={(e: KeyboardEvent) => { if (e.key === "Enter") handleSaveStatusMessage(); }}
          />
          <button class="form-btn-primary" onClick={handleSaveStatusMessage}>Save</button>
        </div>
      </FormField>
    </>
  );
};

export default ProfileTab;
