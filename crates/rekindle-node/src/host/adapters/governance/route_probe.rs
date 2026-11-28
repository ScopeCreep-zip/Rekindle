//! A member route that still appears on M/6 can answer a liveness probe.
//! Check the record/version before sending it and again before making the
//! answer visible. Watches or concurrent reads can advance M/6 meanwhile.

use rekindle_presence::community::RouteVersion;
use rekindle_transport::liveness::dead::Probed;
use rekindle_transport::route_trace::blob_tag;
use rekindle_utils::log_scrub::tag;

use super::HostGovernance;

impl HostGovernance {
    pub(crate) fn take_probed_route(
        &self,
        community: &str,
        record: &str,
        version: &RouteVersion,
    ) -> bool {
        let current = self.binding_of_record(community, record).is_some()
            && self
                .ctx
                .communities
                .map()
                .write()
                .get_mut(community)
                .is_some_and(|c| c.member_routes.set_usable(record, version, true));
        if current {
            self.drain_pending_mesh(community);
        }
        current
    }

    pub(super) async fn probe_member_route(
        &self,
        community: &str,
        record: &str,
        version: &RouteVersion,
    ) -> bool {
        if self.binding_of_record(community, record).is_none()
            || !self
                .ctx
                .communities
                .map()
                .read()
                .get(community)
                .is_some_and(|c| c.member_routes.is_current(record, version))
        {
            return false;
        }
        let Ok(node) = self.transport() else {
            return false;
        };
        let disposition = match node.probe_route(&version.blob).await {
            Probed::Answered => {
                if self.take_probed_route(community, record, version) {
                    "probe_answered"
                } else {
                    "probe_obsolete"
                }
            }
            Probed::ImportFailed(reason) | Probed::Silent(reason) => {
                tracing::debug!(%reason, "member route probe failed");
                "probe_failed"
            }
        };
        tracing::debug!(
            community,
            control_type = "probe",
            route_tag = %blob_tag(&version.blob),
            record_tag = %tag(record),
            seq = version.seq,
            read_disposition = disposition,
            "member route resolution"
        );
        disposition == "probe_answered"
    }

    /// Run a transport probe and let the simulator delay delivery of its answer.
    #[cfg(feature = "netsim-entry")]
    pub async fn netsim_delay_probe_answer(
        &self,
        community: &str,
        record: &str,
        version: &RouteVersion,
        answered: tokio::sync::oneshot::Sender<()>,
        resume: tokio::sync::oneshot::Receiver<()>,
    ) -> bool {
        let Ok(node) = self.transport() else {
            return false;
        };
        if node.probe_route(&version.blob).await != Probed::Answered {
            return false;
        }
        if answered.send(()).is_err() || resume.await.is_err() {
            return false;
        }
        self.take_probed_route(community, record, version)
    }

    pub(super) fn spawn_member_route_probe(
        &self,
        community: &str,
        record: &str,
        version: RouteVersion,
    ) {
        let governance = self.clone();
        let owned = community.to_owned();
        let record = record.to_owned();
        self.ctx
            .community_scope(community)
            .spawn_or_drop("dead member route probe", async move {
                governance
                    .probe_member_route(&owned, &record, &version)
                    .await;
            });
    }
}
