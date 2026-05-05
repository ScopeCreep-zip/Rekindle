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
  CREATE_INSTANT_INVITE,
  VIEW_AUDIT_LOG,
  VIEW_INSIGHTS,
} from "../../ipc/permissions";
import OverviewTab from "./settings/OverviewTab";
import ChannelsTab from "./settings/ChannelsTab";
import MembersTab from "./settings/MembersTab";
import RolesTab from "./settings/RolesTab";
import BansTab from "./settings/BansTab";
import SecurityTab from "./settings/SecurityTab";
import InvitesTab from "./settings/InvitesTab";
import AuditLogTab from "./settings/AuditLogTab";
import AutoModTab from "./settings/AutoModTab";
import AnalyticsTab from "./settings/AnalyticsTab";

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

type TabId = "overview" | "channels" | "members" | "invites" | "roles" | "bans" | "audit-log" | "security" | "automod" | "analytics";

const CommunitySettingsModal: Component<CommunitySettingsModalProps> = (props) => {
  const [activeTab, setActiveTab] = createSignal<TabId>("overview");
  const [confirmAction, setConfirmAction] = createSignal<ConfirmOptions | null>(null);

  const myPerms = createMemo(() =>
    calculateBasePermissions(props.myRoleIds, props.community.roles),
  );

  const canManageCommunity = createMemo(() => hasPermission(myPerms(), MANAGE_COMMUNITY));
  const canManageChannels = createMemo(() => hasPermission(myPerms(), MANAGE_CHANNELS));
  const canManageRoles = createMemo(() => hasPermission(myPerms(), MANAGE_ROLES));
  const canKick = createMemo(() => hasPermission(myPerms(), KICK_MEMBERS));
  const canBan = createMemo(() => hasPermission(myPerms(), BAN_MEMBERS));
  const canModerate = createMemo(() => hasPermission(myPerms(), MODERATE_MEMBERS));
  const canCreateInvite = createMemo(() => hasPermission(myPerms(), CREATE_INSTANT_INVITE));
  const canViewAuditLog = createMemo(() => hasPermission(myPerms(), VIEW_AUDIT_LOG));
  const canViewInsights = createMemo(() => hasPermission(myPerms(), VIEW_INSIGHTS));

  const isAdmin = createMemo(() =>
    canManageCommunity() || canManageChannels() || canManageRoles() || canKick() || canBan()
  );

  const tabs = createMemo((): { id: TabId; label: string }[] => {
    if (!isAdmin()) {
      // Non-admins see only read-only overview and members
      return [
        { id: "overview", label: "Overview" },
        { id: "members", label: "Members" },
      ];
    }

    const base: { id: TabId; label: string }[] = [
      { id: "overview", label: "Overview" },
      { id: "channels", label: "Channels" },
      { id: "members", label: "Members" },
    ];
    if (canCreateInvite() || canManageCommunity()) base.push({ id: "invites", label: "Invites" });
    if (canManageRoles()) base.push({ id: "roles", label: "Roles" });
    if (canBan()) base.push({ id: "bans", label: "Bans" });
    if (canManageCommunity()) base.push({ id: "automod", label: "AutoMod" });
    if (canViewInsights()) base.push({ id: "analytics", label: "Analytics" });
    // Merge audit log into security for admins; show standalone if only audit log perm
    if (canViewAuditLog() && !canManageCommunity()) base.push({ id: "audit-log", label: "Audit Log" });
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

  const modalTitle = createMemo(() => isAdmin() ? "Community Settings" : "Community Info");

  return (
    <Modal isOpen={props.isOpen} title={modalTitle()} onClose={props.onClose} size="lg">
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
            canManageCommunity={canManageCommunity()}
            requestConfirm={requestConfirm}
          />
        </Show>
        <Show when={activeTab() === "invites"}>
          <InvitesTab
            communityId={props.community.id}
            canCreateInvite={canCreateInvite()}
            canManage={canManageCommunity()}
          />
        </Show>
        <Show when={activeTab() === "roles"}>
          <RolesTab community={props.community} requestConfirm={requestConfirm} />
        </Show>
        <Show when={activeTab() === "bans"}>
          <BansTab communityId={props.community.id} canBan={canBan()} />
        </Show>
        <Show when={activeTab() === "automod"}>
          <AutoModTab community={props.community} />
        </Show>
        <Show when={activeTab() === "analytics"}>
          <AnalyticsTab community={props.community} />
        </Show>
        <Show when={activeTab() === "audit-log"}>
          <AuditLogTab communityId={props.community.id} />
        </Show>
        <Show when={activeTab() === "security"}>
          <SecurityTab community={props.community} requestConfirm={requestConfirm} />
          {/* Audit log merged into security tab for admins */}
          <Show when={canViewAuditLog()}>
            <details class="settings-audit-collapsible">
              <summary class="form-label">Audit Log</summary>
              <AuditLogTab communityId={props.community.id} />
            </details>
          </Show>
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
