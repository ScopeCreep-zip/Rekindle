import { type Component } from "solid-js";
import FormField from "../../components/common/FormField";
import SettingsSelect, { type SettingsSelectOption } from "../../components/common/SettingsSelect";
import FavouriteServersSection from "../../components/settings/FavouriteServersSection";
import { settingsState } from "../../stores/settings.store";
import { handleSaveSettings, handleSetAutoAway, handleSetGameDetection } from "../../actions/settings.actions";

/** Auto-away after these minutes of inactivity; 0 turns it off. */
const AUTO_AWAY_OPTIONS: SettingsSelectOption[] = [
  { value: "0", label: "Disabled" },
  { value: "5", label: "5 minutes" },
  { value: "10", label: "10 minutes" },
  { value: "15", label: "15 minutes" },
  { value: "30", label: "30 minutes" },
  { value: "60", label: "1 hour" },
];

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
      <FormField label="Go away after inactivity" labelId="settings-auto-away-label">
        <SettingsSelect
          labelledBy="settings-auto-away-label"
          value={String(settingsState.autoAwayMinutes)}
          onChange={(value) => handleSetAutoAway(parseInt(value))}
          options={AUTO_AWAY_OPTIONS}
        />
      </FormField>
    </>
  );
};

export default ApplicationTab;
