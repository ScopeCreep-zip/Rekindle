import { type Component, createEffect, createMemo, createSignal, For, Show } from "solid-js";
import type { CommunityCapability } from "../../ipc/commands/dto_capabilities";
import {
  BAN_MEMBERS,
  CREATE_INVITES,
  hasPermission,
  KICK_MEMBERS,
  MANAGE_CHANNELS,
  MANAGE_COMMUNITY,
  MANAGE_EXPRESSIONS,
  MANAGE_ROLES,
  servedPermissions,
  VIEW_AUDIT_LOG,
} from "../../ipc/permissions";
import type { Community } from "../../stores/community.store";
import { deniedFor } from "../../utils/denied";
import ConfirmDialog from "../common/ConfirmDialog";
import Modal from "../common/Modal";
import AnalyticsTab from "./settings/AnalyticsTab";
import AuditLogTab from "./settings/AuditLogTab";
import AutoModTab from "./settings/AutoModTab";
import BansTab from "./settings/BansTab";
import ChannelsTab from "./settings/ChannelsTab";
import ExpressionsTab from "./settings/ExpressionsTab";
import InvitesTab from "./settings/InvitesTab";
import MembersTab from "./settings/MembersTab";
import OverviewTab from "./settings/OverviewTab";
import PrivacyTab from "./settings/PrivacyTab";
import RolesTab from "./settings/RolesTab";
import SecurityTab from "./settings/SecurityTab";
import type { ConfirmOptions } from "./settings/types";
import WelcomeTab from "./settings/WelcomeTab";

interface CommunitySettingsModalProps {
  isOpen: boolean;
  community: Community;
  onClose: () => void;
}

type TabId =
  | "overview"
  | "channels"
  | "members"
  | "invites"
  | "roles"
  | "bans"
  | "audit-log"
  | "security"
  | "automod"
  | "analytics"
  | "privacy"
  | "welcome"
  | "expressions";

const CommunitySettingsModal: Component<CommunitySettingsModalProps> = (props) => {
  const [activeTab, setActiveTab] = createSignal<TabId>("overview");
  const [confirmAction, setConfirmAction] = createSignal<ConfirmOptions | null>(null);

  // The backend's base flags (plan E3.2 S9).
  const myPerms = createMemo(() => servedPermissions(props.community));

  const canManageCommunity = createMemo(() => hasPermission(myPerms(), MANAGE_COMMUNITY));
  const canManageChannels = createMemo(() => hasPermission(myPerms(), MANAGE_CHANNELS));
  const canManageRoles = createMemo(() => hasPermission(myPerms(), MANAGE_ROLES));
  const canKick = createMemo(() => hasPermission(myPerms(), KICK_MEMBERS));
  const canBan = createMemo(() => hasPermission(myPerms(), BAN_MEMBERS));
  const canCreateInvite = createMemo(() => hasPermission(myPerms(), CREATE_INVITES));
  const canViewAuditLog = createMemo(() => hasPermission(myPerms(), VIEW_AUDIT_LOG));
  const canManageExpressions = createMemo(() => hasPermission(myPerms(), MANAGE_EXPRESSIONS));

  const isAdmin = createMemo(
    () =>
      canManageCommunity() ||
      canManageChannels() ||
      canManageRoles() ||
      canKick() ||
      canBan() ||
      canManageExpressions()
  );

  // Never-hide: every tab is listed; one we may not open is disabled with
  // the Host's reason (`denied`), never left out. The bits decide nothing.
  const tabs = createMemo((): { id: TabId; label: string; denied: string | null }[] => {
    const why = (cap: CommunityCapability): string | null => deniedFor(props.community, cap);
    return [
      { id: "overview", label: "Overview", denied: null },
      { id: "channels", label: "Channels", denied: why("manageChannels") },
      { id: "members", label: "Members", denied: null },
      { id: "privacy", label: "Privacy", denied: null },
      { id: "invites", label: "Invites", denied: why("createInvite") },
      { id: "roles", label: "Roles", denied: why("manageRoles") },
      { id: "bans", label: "Bans", denied: why("banMembers") },
      { id: "welcome", label: "Welcome", denied: why("manageWelcome") },
      { id: "expressions", label: "Emoji", denied: why("manageExpressions") },
      { id: "automod", label: "AutoMod", denied: why("manageAutomod") },
      { id: "analytics", label: "Analytics", denied: why("viewInsights") },
      { id: "audit-log", label: "Audit Log", denied: why("viewAuditLog") },
      { id: "security", label: "Security", denied: why("manageSecurity") },
    ];
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

  const modalTitle = createMemo(() => (isAdmin() ? "Community Settings" : "Community Info"));

  return (
    <Modal isOpen={props.isOpen} title={modalTitle()} onClose={props.onClose} size="lg">
      <div class="form-tabs">
        <For each={tabs()}>
          {(tab) => (
            <button
              type="button"
              class={`form-tab ${activeTab() === tab.id ? "form-tab-active" : ""}`}
              disabled={tab.denied !== null}
              title={tab.denied ?? undefined}
              aria-description={tab.denied ?? undefined}
              onClick={() => setActiveTab(tab.id)}
            >
              {tab.label}
            </button>
          )}
        </For>
      </div>

      <div class="settings-content">
        <Show when={activeTab() === "overview"}>
          <OverviewTab
            community={props.community}
            manageDenied={deniedFor(props.community, "manageCommunity")}
          />
        </Show>
        <Show when={activeTab() === "channels"}>
          <ChannelsTab community={props.community} canManageChannels={canManageChannels()} />
        </Show>
        <Show when={activeTab() === "members"}>
          <MembersTab
            community={props.community}
            manageDenied={deniedFor(props.community, "manageCommunity")}
          />
        </Show>
        <Show when={activeTab() === "invites"}>
          <InvitesTab
            communityId={props.community.id}
            canCreateInvite={canCreateInvite()}
            revokeDenied={deniedFor(props.community, "manageCommunity")}
          />
        </Show>
        <Show when={activeTab() === "roles"}>
          <RolesTab community={props.community} />
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
        <Show when={activeTab() === "welcome"}>
          <WelcomeTab community={props.community} />
        </Show>
        <Show when={activeTab() === "expressions"}>
          <ExpressionsTab community={props.community} />
        </Show>
        <Show when={activeTab() === "privacy"}>
          <PrivacyTab community={props.community} />
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
        onConfirm={() => {
          confirmAction()?.action();
          setConfirmAction(null);
        }}
        onCancel={() => setConfirmAction(null)}
      />
    </Modal>
  );
};

export default CommunitySettingsModal;
