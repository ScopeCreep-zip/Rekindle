//! The members on screen in the Host (plan E3.4 Q1), on both modes: a
//! frontend reports which members it shows (`MembersOnScreen`, at most 50),
//! and the beat of each one outside the ring is inspected at once and then
//! once per beat period, into the beat view, so the 15-minute rule covers
//! them as it covers the ring. A ring member is skipped: its watch
//! delivers its beat. The inspected records are deliberately unwatched
//! (the ring holds the watch budget), the one app inspect D4 allows, on a
//! set the frontend's report defines (`rekindle_presence::community::
//! on_screen`).

use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_records::port::OpenIntent;

use super::HostGovernance;

impl HostGovernance {
    /// Take a frontend's members on screen in `community_id` (checked: at
    /// most 50 pseudonyms) and inspect the new ones at once.
    pub(crate) fn set_members_on_screen(&self, community_id: &str, members: Vec<String>) {
        let added = {
            let mut map = self.ctx.communities.map().write();
            let Some(community) = map.get_mut(community_id) else {
                return;
            };
            community.on_screen.set(members)
        };
        if added.is_empty() {
            return;
        }
        let host = self.clone();
        let community = community_id.to_owned();
        self.scope()
            .spawn_or_drop("on-screen beat read", async move {
                host.inspect_on_screen_in(&community, Some(&added)).await;
            });
    }

    /// Inspect every community's members on screen outside the ring (the
    /// presence adapter's once-per-beat-period task).
    pub(crate) async fn inspect_on_screen(&self) {
        let communities: Vec<String> = self.ctx.communities.map().read().keys().cloned().collect();
        for community in communities {
            self.inspect_on_screen_in(&community, None).await;
        }
    }

    /// Inspect `community_id`'s members on screen outside the ring (only
    /// those in `only`, when given).
    async fn inspect_on_screen_in(&self, community_id: &str, only: Option<&[String]>) {
        for (member, record) in self.on_screen_records(community_id, only) {
            let arrived = {
                let now = self.ctx.clock.now_ms();
                let mut map = self.ctx.communities.map().write();
                let Some(community) = map.get_mut(community_id) else {
                    return;
                };
                community.on_screen.inspected(&member, now)
            };
            let Ok(lease) = self.acquire_record(&record, None, OpenIntent::Check).await else {
                continue;
            };
            self.read_member_beat(community_id, &record, lease, arrived)
                .await;
            self.release_record(lease).await;
        }
    }

    /// The (member, member record) on screen in `community_id` that the
    /// ring does not watch and that are not us.
    fn on_screen_records(
        &self,
        community_id: &str,
        only: Option<&[String]>,
    ) -> Vec<(String, String)> {
        let map = self.ctx.communities.map().read();
        let Some(community) = map.get(community_id) else {
            return Vec::new();
        };
        let Some(state) = &community.governance_state else {
            return Vec::new();
        };
        let me = community.my_pseudonym_key.as_deref();
        community
            .on_screen
            .members()
            .filter(|m| only.is_none_or(|only| only.contains(m)))
            .filter(|m| Some(m.as_str()) != me)
            .filter_map(|m| {
                let (_, entry) = state.members.iter().find(|(p, _)| p.to_hex() == *m)?;
                (!community.watch_ring.contains_key(&entry.member_record))
                    .then(|| (m.clone(), entry.member_record.clone()))
            })
            .collect()
    }
}
