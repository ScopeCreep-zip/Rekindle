import { Component, For, createEffect, createSignal } from "solid-js";
import type { Community } from "../../../stores/community.store";
import {
  handleDeleteAutoModRule,
  handleSetAutoModRule,
} from "../../../handlers/community.handlers";
import { addToast } from "../../../stores/toast.store";
import FormField from "../../common/FormField";

interface AutoModTabProps {
  community: Community;
}

const AutoModTab: Component<AutoModTabProps> = (props) => {
  const [editingRuleId, setEditingRuleId] = createSignal<string | null>(null);
  const [name, setName] = createSignal("");
  const [keywords, setKeywords] = createSignal("");
  const [regexPatterns, setRegexPatterns] = createSignal("");
  const [action, setAction] = createSignal<"block_locally" | "blur_content" | "alert_moderators">("block_locally");
  const [enabled, setEnabled] = createSignal(true);
  const [saving, setSaving] = createSignal(false);

  createEffect(() => {
    if (editingRuleId()) return;
    setName("");
    setKeywords("");
    setRegexPatterns("");
    setAction("block_locally");
    setEnabled(true);
  });

  function editRule(ruleId: string): void {
    const rule = props.community.automodRules.find((entry) => entry.ruleId === ruleId);
    if (!rule) return;
    setEditingRuleId(rule.ruleId);
    setName(rule.name);
    setKeywords(rule.keywords.join("\n"));
    setRegexPatterns(rule.regexPatterns.join("\n"));
    setAction(rule.action);
    setEnabled(rule.enabled);
  }

  function resetForm(): void {
    setEditingRuleId(null);
    setName("");
    setKeywords("");
    setRegexPatterns("");
    setAction("block_locally");
    setEnabled(true);
  }

  async function saveRule(): Promise<void> {
    const trimmedName = name().trim();
    if (!trimmedName) {
      addToast("Rule name is required", "error");
      return;
    }
    setSaving(true);
    try {
      await handleSetAutoModRule(props.community.id, {
        ruleId: editingRuleId(),
        name: trimmedName,
        enabled: enabled(),
        keywords: keywords().split("\n").map((entry) => entry.trim()).filter(Boolean),
        regexPatterns: regexPatterns().split("\n").map((entry) => entry.trim()).filter(Boolean),
        action: action(),
      });
      addToast(editingRuleId() ? "AutoMod rule updated" : "AutoMod rule created", "success");
      resetForm();
    } finally {
      setSaving(false);
    }
  }

  async function deleteRule(ruleId: string): Promise<void> {
    await handleDeleteAutoModRule(props.community.id, ruleId);
    addToast("AutoMod rule deleted", "success");
    if (editingRuleId() === ruleId) resetForm();
  }

  return (
    <div class="settings-section">
      <FormField label="Existing Rules">
        <div class="automod-rule-list">
          <For each={props.community.automodRules}>
            {(rule) => (
              <div class="automod-rule-card">
                <div class="automod-rule-card-header">
                  <div>
                    <div class="automod-rule-name">{rule.name}</div>
                    <div class="automod-rule-meta">
                      {rule.enabled ? "Enabled" : "Disabled"} • {rule.action.replaceAll("_", " ")}
                    </div>
                  </div>
                  <div class="automod-rule-actions">
                    <button class="settings-copy-btn" onClick={() => editRule(rule.ruleId)}>Edit</button>
                    <button class="form-btn-danger automod-rule-delete-btn" onClick={() => void deleteRule(rule.ruleId)}>Delete</button>
                  </div>
                </div>
              </div>
            )}
          </For>
          <div class="settings-hint">Configured rules apply only on this client after decryption.</div>
        </div>
      </FormField>

      <FormField label="Rule Name">
        <input class="form-input" value={name()} onInput={(e) => setName(e.currentTarget.value)} placeholder="Blocked words" />
      </FormField>
      <FormField label="Keywords">
        <textarea
          class="form-textarea"
          rows={6}
          value={keywords()}
          onInput={(e) => setKeywords(e.currentTarget.value)}
          placeholder="One keyword per line"
        />
      </FormField>
      <FormField label="Regex Patterns">
        <textarea
          class="form-textarea"
          rows={4}
          value={regexPatterns()}
          onInput={(e) => setRegexPatterns(e.currentTarget.value)}
          placeholder="One regex per line"
        />
      </FormField>
      <FormField label="Action">
        <select class="form-select" value={action()} onChange={(e) => setAction(e.currentTarget.value as "block_locally" | "blur_content" | "alert_moderators")}>
          <option value="block_locally">Block locally</option>
          <option value="blur_content">Blur content</option>
          <option value="alert_moderators">Alert moderators</option>
        </select>
      </FormField>
      <label class="automod-enabled-toggle">
        <input type="checkbox" checked={enabled()} onChange={(e) => setEnabled(e.currentTarget.checked)} />
        <span>Rule enabled</span>
      </label>
      <div class="settings-actions-sticky">
        <button class="form-btn-primary" onClick={() => void saveRule()} disabled={saving()}>
          {saving() ? "Saving..." : editingRuleId() ? "Save Rule" : "Create Rule"}
        </button>
        <button class="settings-copy-btn" onClick={resetForm}>Clear</button>
      </div>
    </div>
  );
};

export default AutoModTab;
