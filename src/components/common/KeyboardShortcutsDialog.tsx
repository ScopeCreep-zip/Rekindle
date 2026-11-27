import { type Component, For, onCleanup, onMount } from "solid-js";
import Modal from "./Modal";
import { SHORTCUT_SECTIONS, isHelpChord, keyLabel } from "../../utils/keyboard_shortcuts";
import { setShortcutsHelpOpen, shortcutsHelpOpen } from "../../stores/shortcuts_help.store";

/**
 * The keyboard-shortcuts help dialog (wiring guide §13f): every shortcut
 * the app has, grouped by where it works. Mounted once per window at the
 * router root; Mod+/ opens it from anywhere in the window, and the buddy
 * list's action bar has a button for it.
 */
const KeyboardShortcutsDialog: Component = () => {
  function onKeyDown(e: KeyboardEvent): void {
    if (isHelpChord(e)) {
      e.preventDefault();
      setShortcutsHelpOpen(true);
    }
  }
  onMount(() => window.addEventListener("keydown", onKeyDown));
  onCleanup(() => window.removeEventListener("keydown", onKeyDown));

  return (
    <Modal
      isOpen={shortcutsHelpOpen()}
      title="Keyboard shortcuts"
      onClose={() => setShortcutsHelpOpen(false)}
      size="md"
    >
      {/* The list scrolls on a short window: it is a named, focusable
        * region so the keyboard can scroll it (SC 2.1.1). */}
      <div class="shortcuts-help" role="region" aria-label="All shortcuts" tabIndex={0}>
        <For each={SHORTCUT_SECTIONS}>
          {(section) => (
            <section class="shortcuts-help-section" aria-label={section.title}>
              <h3 class="shortcuts-help-heading">{section.title}</h3>
              <dl class="shortcuts-help-list">
                <For each={section.rows}>
                  {(row) => (
                    <div class="shortcuts-help-row">
                      <dt class="shortcuts-help-keys">
                        <For each={row.keys}>
                          {(chord, i) => (
                            <>
                              {i() > 0 ? <span class="shortcuts-help-or"> or </span> : null}
                              <kbd class="shortcuts-help-chord">
                                <For each={chord}>
                                  {(key, k) => (
                                    <>
                                      {k() > 0 ? "+" : null}
                                      <kbd class="shortcuts-help-key">{keyLabel(key)}</kbd>
                                    </>
                                  )}
                                </For>
                              </kbd>
                            </>
                          )}
                        </For>
                      </dt>
                      <dd class="shortcuts-help-action">{row.action}</dd>
                    </div>
                  )}
                </For>
              </dl>
            </section>
          )}
        </For>
      </div>
    </Modal>
  );
};

export default KeyboardShortcutsDialog;
