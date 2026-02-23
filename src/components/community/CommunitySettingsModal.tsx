import { Component, For, Show, createSignal, createEffect, createMemo } from "solid-js";
import Modal from "../common/Modal";
import ConfirmDialog from "../common/ConfirmDialog";
import type { Community } from "../../stores/community.store";
import {
  calculateBasePermissions,
  hasPermission,
  MANAGE_CHANNELS,
  MANAGE_COMMUNITY,
  MANAGE_ROLES,
  KICK_MEMBERS,
  BAN_MEMBERS,
  MODERATE_MEMBERS,
} from "../../ipc/permissions";
import OverviewTab from "./settings/OverviewTab";
import ChannelsTab from "./settings/ChannelsTab";
import MembersTab from "./settings/MembersTab";
import RolesTab from "./settings/RolesTab";
import BansTab from "./settings/BansTab";
import SecurityTab from "./settings/SecurityTab";

export interface ConfirmOptions {
  title: string;
  message: string;
  confirmLabel?: string;
  action: () => void;
}

interface CommunitySettingsModalProps {
  isOpen: boolean;
  community: Community;
  myRoleIds: number[];
  onClose: () => void;
}

type TabId = "overview" | "channels" | "members" | "roles" | "bans" | "security";

const CommunitySettingsModal: Component<CommunitySettingsModalProps> = (props) => {
  const [activeTab, setActiveTab] = createSignal<TabId>("overview");
  const [confirmAction, setConfirmAction] = createSignal<ConfirmOptions | null>(null);

  const myPerms = createMemo(() =>
    calculateBasePermissions(props.myRoleIds, props.community.roles, props.community.isHosted),
  );

  const canManageCommunity = createMemo(() => hasPermission(myPerms(), MANAGE_COMMUNITY));
  const canManageChannels = createMemo(() => hasPermission(myPerms(), MANAGE_CHANNELS));
  const canManageRoles = createMemo(() => hasPermission(myPerms(), MANAGE_ROLES));
  const canKick = createMemo(() => hasPermission(myPerms(), KICK_MEMBERS));
  const canBan = createMemo(() => hasPermission(myPerms(), BAN_MEMBERS));
  const canModerate = createMemo(() => hasPermission(myPerms(), MODERATE_MEMBERS));

  const tabs = createMemo((): { id: TabId; label: string }[] => {
    const base: { id: TabId; label: string }[] = [
      { id: "overview", label: "Overview" },
      { id: "channels", label: "Channels" },
      { id: "members", label: "Members" },
    ];
    if (canManageRoles()) base.push({ id: "roles", label: "Roles" });
    if (canBan()) base.push({ id: "bans", label: "Bans" });
    if (canManageCommunity()) base.push({ id: "security", label: "Security" });
    return base;
  });

  createEffect(() => {
    if (props.isOpen) {
      setActiveTab("overview");
      setConfirmAction(null);
    }
  });

  function requestConfirm(opts: ConfirmOptions): void {
    setConfirmAction(opts);
  }

  return (
    <Modal isOpen={props.isOpen} title="Community Settings" onClose={props.onClose} size="lg">
      <div class="form-tabs">
        <For each={tabs()}>
          {(tab) => (
            <button
              class={`form-tab ${activeTab() === tab.id ? "form-tab-active" : ""}`}
              onClick={() => setActiveTab(tab.id)}
            >
              {tab.label}
            </button>
          )}
        </For>
      </div>

      <div class="settings-content">
        <Show when={activeTab() === "overview"}>
          <OverviewTab community={props.community} canManage={canManageCommunity()} />
        </Show>
        <Show when={activeTab() === "channels"}>
          <ChannelsTab community={props.community} canManageChannels={canManageChannels()} requestConfirm={requestConfirm} />
        </Show>
        <Show when={activeTab() === "members"}>
          <MembersTab
            community={props.community}
            myRoleIds={props.myRoleIds}
            canManageRoles={canManageRoles()}
            canKick={canKick()}
            canBan={canBan()}
            canModerate={canModerate()}
            requestConfirm={requestConfirm}
          />
        </Show>
        <Show when={activeTab() === "roles"}>
          <RolesTab community={props.community} requestConfirm={requestConfirm} />
        </Show>
        <Show when={activeTab() === "bans"}>
          <BansTab communityId={props.community.id} canBan={canBan()} />
        </Show>
        <Show when={activeTab() === "security"}>
          <SecurityTab community={props.community} requestConfirm={requestConfirm} />
        </Show>
      </div>

      <ConfirmDialog
        isOpen={confirmAction() !== null}
        title={confirmAction()?.title ?? ""}
        message={confirmAction()?.message ?? ""}
        danger
        confirmLabel={confirmAction()?.confirmLabel ?? "Confirm"}
        onConfirm={() => { confirmAction()?.action(); setConfirmAction(null); }}
        onCancel={() => setConfirmAction(null)}
      />
    </Modal>
  );
};

export default CommunitySettingsModal;
