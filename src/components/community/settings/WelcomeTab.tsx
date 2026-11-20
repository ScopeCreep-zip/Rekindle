import { Component, For, createEffect, createSignal } from "solid-js";
import type { Community } from "../../../stores/community.store";
import { commands } from "../../../ipc/commands";
import type { OnboardingConfig, OnboardingConfigEdit, WelcomeChannelEntry, WelcomeScreen } from "../../../ipc/commands/dto";
import { handleLoadOnboardingConfig, handleLoadWelcomeScreen } from "../../../actions/community/profile";
import { addToast } from "../../../stores/toast.store";
import OnboardingQuestionsEditor from "./OnboardingQuestionsEditor";

interface WelcomeTabProps {
  community: Community;
}

const EMPTY_SCREEN: WelcomeScreen = { description: "", channels: [] };

const EMPTY_FLOW: OnboardingConfig = {
  enabled: false,
  mode: "default",
  defaultChannels: [],
  questions: [],
  welcomeMessage: null,
  guideSteps: [],
};

/**
 * The welcome screen and onboarding a joiner sees (plan E7.15, D3's Host
 * commands). Each save is the Host's `WelcomeScreenSet` / `OnboardingConfigSet`:
 * reviewed by the intent gate, checked for `MANAGE_COMMUNITY` and the bounds
 * there; this tab only edits the fields and shows the Host's refusal.
 */
const WelcomeTab: Component<WelcomeTabProps> = (props) => {
  const [screen, setScreen] = createSignal<WelcomeScreen>(EMPTY_SCREEN);
  const [flow, setFlow] = createSignal<OnboardingConfigEdit>(EMPTY_FLOW);
  const [saving, setSaving] = createSignal(false);

  createEffect(() => {
    const id = props.community.id;
    if (!id) return;
    void commands.getWelcomeScreen(id).then(setScreen)
      .catch((e) => console.error("Failed to load welcome screen:", e));
    void commands.getOnboardingConfig(id).then(setFlow)
      .catch((e) => console.error("Failed to load onboarding:", e));
  });

  const channels = () => props.community.channels.filter((c) => c.type !== "voice");

  function setEntry(i: number, patch: Partial<WelcomeChannelEntry>): void {
    setScreen((s) => ({
      ...s,
      channels: s.channels.map((e, j) => (j === i ? { ...e, ...patch } : e)),
    }));
  }

  function addEntry(): void {
    const first = channels()[0];
    if (!first) return;
    setScreen((s) => ({
      ...s,
      channels: [...s.channels, { channelId: first.id, description: "", emoji: null }],
    }));
  }

  function toggleDefault(channelId: string, on: boolean): void {
    setFlow((f) => ({
      ...f,
      defaultChannels: on
        ? [...f.defaultChannels, channelId]
        : f.defaultChannels.filter((c) => c !== channelId),
    }));
  }

  async function save(what: string, write: () => Promise<void>, reload: () => Promise<void>): Promise<void> {
    setSaving(true);
    try {
      await write();
      await reload();
      addToast(`${what} saved`, "success");
    } catch (e) {
      console.error(`Failed to save ${what}:`, e);
      addToast(typeof e === "string" ? e : `Failed to save ${what.toLowerCase()}`, "error");
    } finally {
      setSaving(false);
    }
  }

  const id = () => props.community.id;

  return (
    <div class="settings-section">
      <h3 class="settings-subheading">Welcome screen</h3>
      <div class="settings-hint">Shown to a member when they first open the community.</div>
      <label class="welcome-field">
        <span class="form-field-label">Description</span>
        <textarea
          class="form-textarea"
          value={screen().description}
          onInput={(e) => setScreen((s) => ({ ...s, description: e.currentTarget.value }))}
        />
      </label>
      <For each={screen().channels}>
        {(entry, i) => (
          <div class="welcome-channel-row">
            <label class="welcome-field">
              <span class="form-field-label">Channel</span>
              <select
                class="form-select"
                value={entry.channelId}
                onChange={(e) => setEntry(i(), { channelId: e.currentTarget.value })}
              >
                <For each={channels()}>{(c) => <option value={c.id}>#{c.name}</option>}</For>
              </select>
            </label>
            <label class="welcome-field">
              <span class="form-field-label">Emoji</span>
              <input
                class="form-input welcome-emoji"
                value={entry.emoji ?? ""}
                onInput={(e) => setEntry(i(), { emoji: e.currentTarget.value === "" ? null : e.currentTarget.value })}
              />
            </label>
            <label class="welcome-field">
              <span class="form-field-label">What it is for</span>
              <input
                class="form-input"
                value={entry.description}
                onInput={(e) => setEntry(i(), { description: e.currentTarget.value })}
              />
            </label>
            <button
              class="form-btn-secondary"
              onClick={() => setScreen((s) => ({ ...s, channels: s.channels.filter((_, j) => j !== i()) }))}
            >
              Remove
            </button>
          </div>
        )}
      </For>
      <div class="settings-list-actions">
        <button class="form-btn-secondary" onClick={addEntry} disabled={channels().length === 0}>
          Feature a channel
        </button>
        <button
          class="form-btn-primary"
          disabled={saving()}
          onClick={() => void save(
            "Welcome screen",
            () => commands.setWelcomeScreen(id(), screen()),
            () => handleLoadWelcomeScreen(id()),
          )}
        >
          Save welcome screen
        </button>
      </div>

      <h3 class="settings-subheading">Onboarding</h3>
      <label class="automod-enabled-toggle">
        <input
          type="checkbox"
          checked={flow().enabled}
          onChange={(e) => setFlow((f) => ({ ...f, enabled: e.currentTarget.checked }))}
        />
        <span>Onboarding for new members</span>
      </label>
      <label class="welcome-field">
        <span class="form-field-label">Mode</span>
        <select
          class="form-select"
          value={flow().mode}
          onChange={(e) => setFlow((f) => ({ ...f, mode: e.currentTarget.value as OnboardingConfig["mode"] }))}
        >
          <option value="default">Default: roles at once</option>
          <option value="guided">Guided: questions first</option>
          <option value="gated">Gated: agree to the rules first</option>
        </select>
      </label>
      <label class="welcome-field">
        <span class="form-field-label">Welcome message</span>
        <textarea
          class="form-textarea"
          value={flow().welcomeMessage ?? ""}
          onInput={(e) => setFlow((f) => ({
            ...f,
            welcomeMessage: e.currentTarget.value === "" ? null : e.currentTarget.value,
          }))}
        />
      </label>
      <fieldset class="welcome-defaults">
        <legend class="form-field-label">Channels every new member sees</legend>
        <For each={channels()}>
          {(c) => (
            <label class="automod-enabled-toggle">
              <input
                type="checkbox"
                checked={flow().defaultChannels.includes(c.id)}
                onChange={(e) => toggleDefault(c.id, e.currentTarget.checked)}
              />
              <span>#{c.name}</span>
            </label>
          )}
        </For>
      </fieldset>
      <OnboardingQuestionsEditor
        flow={flow()}
        setFlow={setFlow}
        channels={channels()}
        roles={props.community.roles}
      />
      <div class="settings-list-actions">
        <button
          class="form-btn-primary"
          disabled={saving()}
          onClick={() => void save(
            "Onboarding",
            () => commands.setOnboardingConfig(id(), flow()),
            // The Host minted the new items' ids: edit on from its flow, so a
            // second save keeps them instead of sending new items again.
            async () => {
              setFlow(await commands.getOnboardingConfig(id()));
              await handleLoadOnboardingConfig(id());
            },
          )}
        >
          Save onboarding
        </button>
      </div>
    </div>
  );
};

export default WelcomeTab;
