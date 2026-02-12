import { Component } from "solid-js";
import type { UserStatus } from "../../stores/auth.store";

interface StatusDotProps {
  status: UserStatus | string;
}

const statusClassMap: Record<UserStatus, string> = {
  online: "status-dot status-dot-online",
  away: "status-dot status-dot-away",
  busy: "status-dot status-dot-busy",
  offline: "status-dot status-dot-offline",
};

const StatusDot: Component<StatusDotProps> = (props) => {
  return <div class={statusClassMap[props.status as UserStatus] ?? "status-dot status-dot-offline"} />;
};

export default StatusDot;
