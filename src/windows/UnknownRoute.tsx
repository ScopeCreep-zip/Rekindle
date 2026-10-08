import { Component } from "solid-js";
import Titlebar from "../components/titlebar/Titlebar";

/// Shown when a webview loads a path the app does not route. Every window
/// is opened by the backend with a known path, so this only appears if
/// something navigated a webview where it should not go.
const UnknownRoute: Component = () => (
  <div class="app-frame">
    <Titlebar title="Rekindle" />
    <div class="empty-placeholder">
      <div class="empty-placeholder-title">Nothing to show here</div>
      <div class="empty-placeholder-subtitle">You can close this window.</div>
    </div>
  </div>
);

export default UnknownRoute;
