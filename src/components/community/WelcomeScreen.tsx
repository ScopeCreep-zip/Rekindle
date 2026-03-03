import { Component, For, Show } from "solid-js";
import type { WelcomeScreen as WelcomeScreenData } from "../../stores/types";

interface WelcomeScreenProps {
  screen: WelcomeScreenData;
  communityName: string;
  onChannelClick: (channelId: string) => void;
}

const WelcomeScreen: Component<WelcomeScreenProps> = (props) => {
  return (
    <div class="welcome-screen">
      <div class="welcome-screen-header">
        <h2>{props.communityName}</h2>
      </div>

      <Show when={props.screen.description}>
        <p class="welcome-screen-description">{props.screen.description}</p>
      </Show>

      <Show when={props.screen.channels.length > 0}>
        <div class="welcome-screen-channels">
          <h3>Featured Channels</h3>
          <div class="welcome-channel-list">
            <For each={props.screen.channels}>
              {(ch) => (
                <button
                  class="welcome-channel-entry"
                  onClick={() => props.onChannelClick(ch.channelId)}
                >
                  <div class="welcome-channel-info">
                    <Show when={ch.emoji}>
                      <span class="welcome-channel-emoji">{ch.emoji}</span>
                    </Show>
                    <span class="welcome-channel-name">#{ch.channelId}</span>
                  </div>
                  <p class="welcome-channel-desc">{ch.description}</p>
                </button>
              )}
            </For>
          </div>
        </div>
      </Show>
    </div>
  );
};

export default WelcomeScreen;
