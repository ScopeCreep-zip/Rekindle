import { Component } from "solid-js";

interface RoleTagProps {
  name: string;
  color?: string;
}

const RoleTag: Component<RoleTagProps> = (props) => {
  return (
    <span
      class={props.color ? "role-tag" : "role-tag role-tag-default"}
      style={props.color ? {
        background: `color-mix(in srgb, ${props.color} 25%, transparent)`,
        color: props.color,
      } : {}}
    >
      {props.name}
    </span>
  );
};

export default RoleTag;
