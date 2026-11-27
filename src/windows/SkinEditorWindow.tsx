import { type Component, onMount } from "solid-js";
import Titlebar from "../components/titlebar/Titlebar";
import { startEventStream } from "../ipc/channels";
import SkinEditor from "./skin_editor/SkinEditor";
import { setWindowKind } from "../utils/window_kind";

/**
 * The skin editor's window (plan E7.7.5: a Settings sub-window, opened
 * from Settings → Appearance). It is the editing window: the draft's live
 * style applies to this document only. Its event stream carries the
 * device skin (`SkinChanged`), which the shared `useSkin` applies under
 * the draft.
 */
const SkinEditorWindow: Component = () => {
  onMount(() => {
    void startEventStream();
  });
  setWindowKind("skin-editor");
  return (
    <div class="app-frame">
      <a href="#main-content" class="skip-link">Skip to the skin editor</a>
      <Titlebar title="Skin Editor" />
      <div class="settings-content" id="main-content" tabindex="-1">
        <SkinEditor />
      </div>
    </div>
  );
};

export default SkinEditorWindow;
