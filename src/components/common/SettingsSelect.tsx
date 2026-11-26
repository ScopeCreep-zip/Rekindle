import { Component } from "solid-js";
import { Select } from "@kobalte/core/select";

/** One choice: the value the caller persists and the text shown for it. */
export interface SettingsSelectOption {
  value: string;
  label: string;
}

interface SettingsSelectProps {
  options: SettingsSelectOption[];
  /** The stored value, rendered as is: the component keeps no copy. */
  value: string;
  /** Called with the chosen option's value when the choice changes. */
  onChange: (value: string) => void;
  disabled?: boolean;
  /** Id of the visible label naming the control (`FormField`'s `labelId`). */
  labelledBy: string;
}

/**
 * Settings' drop-down list: Kobalte's Select (the APG select-only combobox
 * with a listbox), whose open list is DOM the skin colours, where a native
 * `<select>`'s list is drawn by the OS. The value comes from the caller's
 * store on every render; a choice only reports to `onChange`. A value no
 * option carries shows an empty trigger, as the native select did.
 */
const SettingsSelect: Component<SettingsSelectProps> = (props) => (
  <Select<SettingsSelectOption>
    options={props.options}
    optionValue="value"
    optionTextValue="label"
    value={props.options.find((o) => o.value === props.value) ?? null}
    onChange={(option) => {
      if (option && option.value !== props.value) props.onChange(option.value);
    }}
    disallowEmptySelection
    disabled={props.disabled}
    sameWidth
    gutter={2}
    itemComponent={(itemProps) => (
      <Select.Item item={itemProps.item} class="settings-select-item">
        <Select.ItemLabel>{itemProps.item.rawValue.label}</Select.ItemLabel>
      </Select.Item>
    )}
  >
    <Select.Trigger class="settings-select-trigger" aria-labelledby={props.labelledBy}>
      <Select.Value<SettingsSelectOption> class="settings-select-value">
        {(state) => state.selectedOption().label}
      </Select.Value>
      <Select.Icon class="settings-select-chevron" />
    </Select.Trigger>
    <Select.Portal>
      <Select.Content class="settings-select-content">
        <Select.Listbox class="settings-select-listbox" />
      </Select.Content>
    </Select.Portal>
  </Select>
);

export default SettingsSelect;
