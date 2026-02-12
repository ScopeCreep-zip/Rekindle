import { Component, Show } from "solid-js";
import type { UserStatus } from "../../stores/auth.store";

interface AvatarProps {
  displayName: string;
  status?: UserStatus;
  size?: number;
  avatarUrl?: string;
}

function getInitials(name: string): string {
  return name
    .split(" ")
    .map((w) => w[0])
    .join("")
    .toUpperCase()
    .slice(0, 2);
}

const Avatar: Component<AvatarProps> = (props) => {
  const size = () => props.size ?? 32;

  return (
    <Show
      when={props.avatarUrl}
      fallback={
        <div
          class="avatar"
          style={{
            width: `${size()}px`,
            height: `${size()}px`,
            "font-size": `${Math.floor(size() * 0.4)}px`,
          }}
        >
          {getInitials(props.displayName)}
        </div>
      }
    >
      <img
        class="avatar"
        src={props.avatarUrl}
        alt={props.displayName}
        style={{
          width: `${size()}px`,
          height: `${size()}px`,
          "object-fit": "cover",
          "border-radius": "50%",
        }}
      />
    </Show>
  );
};

export default Avatar;
