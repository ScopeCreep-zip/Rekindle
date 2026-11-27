import type { Component } from "solid-js";
import FormField from "../../components/common/FormField";
import FavouriteServersSection from "../../components/settings/FavouriteServersSection";
import { settingsState } from "../../stores/settings.store";
import { handleSaveSettings, handleSetAutoAway, handleSetGameDetection } from "../../actions/settings.actions";

const ApplicationTab: Component = () => {
  function handleToggle(key: keyof typeof settingsState): void {
    handleSaveSettings({ [key]: !settingsState[key] });
  }

  return (
    <>
      <div class="settings-section-title">Startup</div>
      <label class="settings-option">
        <input
          type="checkbox"
          checked={settingsState.autoStart}
          onChange={() => handleToggle("autoStart")}
        />
        <span class="buddy-name">Start with System</span>
      </label>
      <label class="settings-option">
        <input
          type="checkbox"
          checked={settingsState.startMinimized}
          onChange={() => handleToggle("startMinimized")}
        />
        <span class="buddy-name">Start Minimized</span>
      </label>
      <div class="settings-section-title">Game Detection</div>
      <label class="settings-option">
        <input
          type="checkbox"
          checked={settingsState.showGameActivity}
          onChange={() => void handleSetGameDetection(!settingsState.showGameActivity)}
        />
        <span class="buddy-name">Detect running games</span>
      </label>
      <div class="settings-hint">
        Records your game time on this computer. Whether friends see the
        game you play is under Privacy.
      </div>
      <FavouriteServersSection />
      <div class="settings-section-title">Auto-Away</div>
      <FormField label="Go away after inactivity">
        <select
          class="form-select"
          value={settingsState.autoAwayMinutes}
          onChange={(e) =>
            handleSetAutoAway(parseInt(e.currentTarget.value))
          }
        >
          <option value={0}>Disabled</option>
          <option value={5}>5 minutes</option>
          <option value={10}>10 minutes</option>
          <option value={15}>15 minutes</option>
          <option value={30}>30 minutes</option>
          <option value={60}>1 hour</option>
        </select>
      </FormField>
    </>
  );
};

export default ApplicationTab;
