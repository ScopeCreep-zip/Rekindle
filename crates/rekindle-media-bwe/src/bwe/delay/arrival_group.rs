use std::mem;
use std::time::{Duration, Instant};

use crate::TwccSeq;

use super::super::time::{BweTimestamp, TimeDelta};
use super::super::AckedPacket;

const BURST_TIME_INTERVAL: Duration = Duration::from_millis(5);
const SEND_TIME_GROUP_LENGTH: Duration = Duration::from_millis(5);
const MAX_BURST_DURATION: Duration = Duration::from_millis(100);

#[derive(Debug, Default)]
pub struct ArrivalGroup {
    first: Option<(TwccSeq, Instant, Instant)>,
    last_seq_no: Option<TwccSeq>,
    last_local_send_time: Option<Instant>,
    last_remote_recv_time: Option<Instant>,
    size: usize,
}

impl ArrivalGroup {
    /// Maybe add a packet to the group.
    ///
    /// Returns [`true`] if a new group needs to be created and [`false`] otherwise.
    fn add_packet(&mut self, packet: &AckedPacket) -> bool {
        match self.belongs_to_group(packet) {
            Belongs::NewGroup => return true,
            Belongs::Skipped => return false,
            Belongs::Yes => {}
        }

        if self.first.is_none() {
            self.first = Some((
                packet.seq_no,
                packet.local_send_time,
                packet.remote_recv_time,
            ));
        }

        self.last_remote_recv_time = self
            .last_remote_recv_time
            .max(Some(packet.remote_recv_time));
        self.last_local_send_time = self.last_local_send_time.max(Some(packet.local_send_time));
        self.size += 1;
        self.last_seq_no = self.last_seq_no.max(Some(packet.seq_no));

        false
    }

    fn belongs_to_group(&self, packet: &AckedPacket) -> Belongs {
        let Some((_, first_local_send_time, first_remote_recv_time)) = self.first else {
            // Start of the group
            return Belongs::Yes;
        };

        let Some(first_send_delta) = packet
            .local_send_time
            .checked_duration_since(first_local_send_time)
        else {
            // Out of order
            return Belongs::Skipped;
        };

        let send_time_delta = BweTimestamp::from(packet.local_send_time) - self.local_send_time();
        if send_time_delta == TimeDelta::ZERO {
            return Belongs::Yes;
        }
        let arrival_time_delta =
            BweTimestamp::from(packet.remote_recv_time) - self.remote_recv_time();

        let propagation_delta = arrival_time_delta - send_time_delta;
        if propagation_delta < TimeDelta::ZERO
            && arrival_time_delta <= BURST_TIME_INTERVAL
            && packet.remote_recv_time - first_remote_recv_time < MAX_BURST_DURATION
        {
            Belongs::Yes
        } else if first_send_delta > SEND_TIME_GROUP_LENGTH {
            Belongs::NewGroup
        } else {
            Belongs::Yes
        }
    }

    /// Calculate the send time delta between self and a subsequent group.
    fn departure_delta(&self, other: &Self) -> TimeDelta {
        BweTimestamp::from(other.local_send_time()) - self.local_send_time()
    }

    /// Calculate the remote receive time delta between self and a subsequent group.
    fn arrival_delta(&self, other: &Self) -> TimeDelta {
        BweTimestamp::from(other.remote_recv_time()) - self.remote_recv_time()
    }

    /// The local send time i.e. departure time, for the group.
    ///
    /// Panics if the group doesn't have at least one packet.
    fn local_send_time(&self) -> Instant {
        self.last_local_send_time
            .expect("local_send_time to only be called on non-empty groups")
    }

    /// The remote receive time i.e. arrival time, for the group.
    ///
    /// Panics if the group doesn't have at least one packet.
    fn remote_recv_time(&self) -> Instant {
        self.last_remote_recv_time
            .expect("remote_recv_time to only be called on non-empty groups")
    }
}

/// Whether a given packet is belongs to a group or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Belongs {
    /// The packet is belongs to the group.
    Yes,
    /// The packet is does not belong to the group, a new group should be created.
    NewGroup,
    /// The packet was skipped and a decision wasn't made.
    Skipped,
}

impl Belongs {
    #[cfg(test)]
    fn new_group(&self) -> bool {
        matches!(self, Self::NewGroup)
    }
}

#[derive(Debug, Default)]
pub struct ArrivalGroupAccumulator {
    previous_group: Option<ArrivalGroup>,
    current_group: ArrivalGroup,
}

impl ArrivalGroupAccumulator {
    ///
    /// Accumulate a packet.
    ///
    /// If adding this packet produced a new delay delta it is returned.
    pub fn accumulate_packet(&mut self, packet: &AckedPacket) -> Option<InterGroupDelayDelta> {
        let need_new_group = self.current_group.add_packet(packet);

        if !need_new_group {
            return None;
        }

        // Variation between previous group and current.
        let arrival_delta = self.arrival_delta();
        let send_delta = self.send_delta();
        let last_remote_recv_time = self.current_group.remote_recv_time();

        let current_group = mem::take(&mut self.current_group);
        self.previous_group = Some(current_group);

        self.current_group.add_packet(packet);

        Some(InterGroupDelayDelta {
            send_delta: send_delta?,
            arrival_delta: arrival_delta?,
            last_remote_recv_time,
        })
    }

    fn arrival_delta(&self) -> Option<TimeDelta> {
        self.previous_group
            .as_ref()
            .map(|prev| prev.arrival_delta(&self.current_group))
    }

    fn send_delta(&self) -> Option<TimeDelta> {
        self.previous_group
            .as_ref()
            .map(|prev| prev.departure_delta(&self.current_group))
    }
}

/// The calculate delay delta between two groups of packets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterGroupDelayDelta {
    /// The delta between the send times of the two groups i.e. delta between the last packet sent
    /// in each group.
    pub send_delta: TimeDelta,
    /// The delta between the remote arrival times of the two groups.
    pub arrival_delta: TimeDelta,
    /// The reported receive time for the last packet in the first arrival group.
    pub last_remote_recv_time: Instant,
}

#[cfg(test)]
#[path = "arrival_group_test.rs"]
mod test;
