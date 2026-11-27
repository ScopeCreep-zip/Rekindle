import { hostInvoke, invoke } from "../invoke";
import type { GovernanceWriteAnswer } from "./community";
import type { CommunityBanView } from "./dto";
import type { QuietHoursSettings } from "./types";

/** A community's administration and our settings in it: membership and
 *  device enrolment, leaving, channel and info edits, bans and blocks, key
 *  rotation, notifications, quiet hours and link previews. */
export const communityAdminCommands = {
  removeCommunityMember: (communityId: string, pseudonymKey: string) =>
    hostInvoke<unknown>("remove_community_member", { communityId, pseudonymKey }).then(
      () => undefined
    ),
  /** Enrol this device's fresh key in a community it may not write to
   *  (`CommunityDeviceEnrol`; the Host's TrustChanging review, `EnrolDevice`). */
  communityDeviceEnrol: (communityId: string) =>
    hostInvoke<null>("community_device_enrol", { communityId }),
  leaveCommunity: (communityId: string) =>
    hostInvoke<unknown>("leave_community", { communityId }).then(() => undefined),
  deleteChannel: (communityId: string, channelId: string) =>
    hostInvoke<GovernanceWriteAnswer & { deleted: string }>("delete_channel", {
      communityId,
      channelId,
    }).then(() => undefined),
  renameChannel: (communityId: string, channelId: string, newName: string) =>
    hostInvoke<void>("rename_channel", { communityId, channelId, newName }),
  /**
   * Architecture §32 Phase 5 Week 15 — community info edit. Each
   * argument is null to leave the field unchanged. Backend reads
   * current `governance.metadata`, overrides only the supplied fields,
   * and re-emits a `CommunityMeta` LWW entry so unchanged values
   * (e.g. `iconHash`) are not nuked when only `name` is edited.
   * `iconHash`/`bannerHash` are BLAKE3 hex hashes returned by
   * `setCommunityAvatar`/`setCommunityBanner`.
   */
  updateCommunityInfo: (
    communityId: string,
    name: string | null,
    description: string | null,
    iconHash: string | null = null,
    bannerHash: string | null = null
  ) =>
    hostInvoke<unknown>("update_community_info", {
      communityId,
      name,
      description,
      iconHash,
      bannerHash,
    }),
  banMember: (communityId: string, pseudonymKey: string) =>
    hostInvoke<unknown>("ban_member", { communityId, pseudonymKey }).then(() => undefined),
  unbanMember: (communityId: string, pseudonymKey: string) =>
    hostInvoke<unknown>("unban_member", { communityId, pseudonymKey }).then(() => undefined),
  blockCommunityMember: (communityId: string, pseudonymKey: string) =>
    hostInvoke<unknown>("block_community_member", { communityId, pseudonymKey }).then(
      () => undefined
    ),
  unblockCommunityMember: (communityId: string, pseudonymKey: string) =>
    hostInvoke<unknown>("unblock_community_member", { communityId, pseudonymKey }).then(
      () => undefined
    ),
  listCommunityBlocks: (communityId: string) =>
    hostInvoke<string[]>("list_community_blocks", { communityId }),
  getBanList: (communityId: string) =>
    hostInvoke<CommunityBanView[]>("get_ban_list", { communityId }),
  rotateMek: (communityId: string, idempotencyKey?: string) =>
    invoke<void>("rotate_mek", {
      communityId,
      // Phase 8 — idempotency key dedupes click-spam. Without it, a
      // double-rotate desyncs MEK generations across mesh peers.
      idempotencyKey: idempotencyKey ?? crypto.randomUUID(),
    }),
  setChannelNotificationLevel: (
    communityId: string,
    channelId: string,
    level: "all" | "mentions" | "nothing"
  ) => invoke<void>("set_channel_notification_level", { communityId, channelId, level }),
  /**
   * Architecture §17.1 tier 2 — community-wide default notification
   * level (broadcast as a `CommunityNotificationDefault` governance
   * entry). Per-channel overrides still win locally per §17.1.
   */
  setCommunityDefaultNotificationLevel: (
    communityId: string,
    level: "all" | "mentions" | "nothing"
  ) =>
    hostInvoke<unknown>("set_community_default_notification_level", { communityId, level }).then(
      () => undefined
    ),
  /** Read the current community default notification level (or `null`). */
  getCommunityDefaultNotificationLevel: (communityId: string) =>
    hostInvoke<"all" | "mentions" | "nothing" | null>("get_community_default_notification_level", {
      communityId,
    }),
  /**
   * Architecture §32 Phase 7 Week 25 — set the notification sound for
   * a channel (`channelId` non-empty) or for the community default
   * (`channelId = ""`). `soundRef = null` removes the override and
   * re-inherits from the next level up.
   */
  setNotificationSound: (communityId: string, channelId: string, soundRef: string | null) =>
    invoke<void>("set_notification_sound", {
      communityId,
      channelId,
      soundRef,
    }),
  /**
   * Resolve the effective notification sound for `(community, channel)`
   * using channel override → community default → null fallthrough.
   */
  getNotificationSound: (communityId: string, channelId: string) =>
    invoke<string | null>("get_notification_sound", {
      communityId,
      channelId,
    }),
  /**
   * Architecture §32 Phase 7 Week 25 — Do Not Disturb global toggle.
   * When `true`, suppresses every notification regardless of channel
   * level, mention status, or quiet-hours window.
   */
  setDoNotDisturb: (enabled: boolean) => invoke<void>("set_do_not_disturb", { enabled }),
  getDoNotDisturb: () => invoke<boolean>("get_do_not_disturb"),
  /**
   * Architecture §28.8 line 3220 — IP-privacy toggle for outgoing link
   * previews. When false, this device skips the OpenGraph fetch so no
   * third-party server learns this device's IP. Receivers' previews
   * from other senders continue to render.
   */
  setLinkPreviewsEnabled: (enabled: boolean) =>
    invoke<void>("set_link_previews_enabled", { enabled }),
  getLinkPreviewsEnabled: () => invoke<boolean>("get_link_previews_enabled"),
  setQuietHours: (enabled: boolean, startHour: number, endHour: number, timezone: string) =>
    invoke<void>("set_quiet_hours", {
      enabled,
      startHour,
      endHour,
      timezone,
    }),
  getQuietHours: () => invoke<QuietHoursSettings>("get_quiet_hours"),
};
