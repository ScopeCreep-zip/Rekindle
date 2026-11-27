//! Conversation navigation and group membership actions.

use crate::action::Action;
use crate::views::ViewKind;
use rekindle_ipc::protocol::{GroupDmRequest, IpcRequest};

use super::App;

impl App {
    pub(super) fn reduce_conversations(&mut self, action: Action) {
        match action {
            Action::ShowDmInbox => {
                self.nav
                    .navigate(ViewKind::DmInbox, self.theme.use_unicode());
                self.load_dm_inbox();
            }
            Action::ShowDmThread { peer_key } => {
                self.nav.navigate(
                    ViewKind::DmThread {
                        peer_key: peer_key.clone(),
                    },
                    self.theme.use_unicode(),
                );
                self.load_dm_history(peer_key);
            }
            Action::ShowFriendList => {
                self.nav
                    .navigate(ViewKind::FriendList, self.theme.use_unicode());
                self.load_friend_list();
            }
            Action::ShowGroupDms => {
                self.nav
                    .navigate(ViewKind::GroupDms, self.theme.use_unicode());
                self.load_group_dms();
            }
            Action::ShowGroupMessages { group_id } => self.load_group_messages(group_id),
            Action::LoadDmHistory { peer_key } => self.load_dm_history(peer_key),
            Action::AcceptGroupInvite { group_id } => {
                self.spawn_group_request(
                    IpcRequest::GroupDm(GroupDmRequest::Accept { group_id }),
                    "accept group",
                );
            }
            Action::DeclineGroupInvite { group_id } => {
                self.spawn_group_request(
                    IpcRequest::GroupDm(GroupDmRequest::Decline { group_id }),
                    "decline group",
                );
            }
            Action::LeaveGroup { group_id } => {
                self.spawn_group_request(
                    IpcRequest::GroupDm(GroupDmRequest::Leave { group_id }),
                    "leave group",
                );
            }
            _ => unreachable!("conversation action"),
        }
    }
}
