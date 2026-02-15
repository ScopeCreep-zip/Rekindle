import { Component } from "solid-js";

interface RoleTagProps {
  name: string;
  color?: number;
}

function colorToHex(color: number): string | undefined {
  if (!color) return undefined;
  return `#${(color & 0xFFFFFF).toString(16).padStart(6, "0")}`;
}

const RoleTag: Component<RoleTagProps> = (props) => {
  const hex = () => colorToHex(props.color ?? 0);

  return (
    <span
      class={hex() ? "role-tag" : "role-tag role-tag-default"}
      style={hex() ? {
        background: `color-mix(in srgb, ${hex()} 25%, transparent)`,
        color: hex(),
      } : {}}
    >
      {props.name}
    </span>
  );
};

export default RoleTag;
