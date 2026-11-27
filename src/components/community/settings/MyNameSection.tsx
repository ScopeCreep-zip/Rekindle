import { Component, Show, createEffect, createSignal } from "solid-js";
import { commands } from "../../../ipc/commands";
import { isCommandError } from "../../../ipc/commands/dto";
import { communityState } from "../../../stores/community.store";
import { errorMessage } from "../../../utils/error";
import FormField from "../../common/FormField";

/**
 * Our name in this community (plan E3.4 S6): set through the Host's review
 * (`set_community_display_name`, SetCommunityDisplayName), its bound the
 * Host's. Empty clears it, and members then see our short fingerprint. The
 * field starts from our card name as set (`myCardName`); how we show now is
 * the Host's `myDisplayName`. Our identity name is never used here.
 */
const MyNameSection: Component<{ communityId: string }> = (props) => {
  const community = () => communityState.communities[props.communityId];
  const [draft, setDraft] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  createEffect(() => setDraft(community()?.myCardName ?? ""));

  const changed = (): boolean => draft().trim() !== (community()?.myCardName ?? "");

  async function save(e: Event): Promise<void> {
    e.preventDefault();
    if (!changed() || busy()) return;
    setBusy(true);
    setError(null);
    try {
      // The Host publishes our MemberCardChanged; the window takes the name from it.
      // An emptied name is an explicit clear (PF2): the fingerprint shows.
      const name = draft().trim();
      await commands.setCommunityDisplayName(props.communityId, name === "" ? { op: "clear" } : { op: "set", value: name });
    } catch (err) {
      if (!(isCommandError(err) && err.type === "cancelled")) setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form class="my-name-section" onSubmit={(e) => void save(e)}>
      <FormField label="Your name in this community" error={error()}>
        <div class="form-field-row my-name-row">
          <input
            class="form-input"
            type="text"
            aria-describedby="my-name-hint"
            value={draft()}
            onInput={(e) => setDraft(e.currentTarget.value)}
            placeholder="Leave empty to show your fingerprint"
          />
          <button type="submit" class="form-btn-secondary" disabled={!changed() || busy()}>
            {busy() ? "Saving…" : "Save"}
          </button>
        </div>
        <Show when={community()}>
          {(c) => (
            <div class="settings-hint" id="my-name-hint">
              Members here see you as {c().myDisplayName}.
            </div>
          )}
        </Show>
      </FormField>
    </form>
  );
};

export default MyNameSection;
