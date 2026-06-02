import { Component, createSignal, Show } from "solid-js";
import { handleCheckForUpdates } from "../../handlers/settings.handlers";

const AboutTab: Component = () => {
  const [checkingUpdates, setCheckingUpdates] = createSignal(false);
  const [updateResult, setUpdateResult] = createSignal<string | null>(null);

  async function handleCheckUpdates(): Promise<void> {
    setCheckingUpdates(true);
    setUpdateResult(null);
    const available = await handleCheckForUpdates();
    if (available) {
      setUpdateResult("Update available! Restart to apply.");
    } else {
      setUpdateResult("You are on the latest version.");
    }
    setCheckingUpdates(false);
  }

  return (
    <>
      <div class="settings-section-title">Rekindle</div>
      <div class="settings-about-text">
        A faithful recreation of the classic Xfire gaming chat client,
        rebuilt with modern technology for the decentralized era.
      </div>
      <div class="settings-about-row">
        <span class="form-field-label">Version</span>
        <span class="settings-about-value">0.1.0-dev</span>
      </div>
      <div class="settings-about-row">
        <span class="form-field-label">Stack</span>
        <span class="settings-about-value">Tauri 2 + SolidJS + Veilid</span>
      </div>
      <div class="settings-about-row">
        <span class="form-field-label">License</span>
        <span class="settings-about-value">MIT</span>
      </div>
      <div class="settings-section-title">Updates</div>
      <div class="update-check-row">
        <button
          class="form-btn-secondary"
          onClick={handleCheckUpdates}
          disabled={checkingUpdates()}
        >
          {checkingUpdates() ? "Checking..." : "Check for Updates"}
        </button>
        <Show when={updateResult()}>
          {(result) => (
            <span class={result().includes("available") ? "update-status-available" : "update-status"}>
              {result()}
            </span>
          )}
        </Show>
      </div>
    </>
  );
};

export default AboutTab;
