use std::time::{Duration, Instant};

use crate::{DataSize, TwccSeq};

use super::{AckedPacket, ArrivalGroup, ArrivalGroupAccumulator, Belongs, TimeDelta};

#[test]
fn test_arrival_group_all_packets_belong_to_empty_group() {
    let now = Instant::now();
    let group = ArrivalGroup::default();

    assert_eq!(
        group.belongs_to_group(&AckedPacket {
            seq_no: 1.into(),
            size: DataSize::ZERO,
            local_send_time: now,
            remote_recv_time: now + duration_us(10),
            local_recv_time: now + duration_us(12),
        }),
        Belongs::Yes,
        "Any packet should belong to an empty arrival group"
    );
}

#[test]
fn test_arrival_group_all_packets_sent_within_burst_interval_belong() {
    let now = Instant::now();
    let packets = vec![
        AckedPacket {
            seq_no: 0.into(),
            size: DataSize::ZERO,
            local_send_time: now,
            remote_recv_time: now + duration_us(150),
            local_recv_time: now + duration_us(200),
        },
        AckedPacket {
            seq_no: 1.into(),
            size: DataSize::ZERO,
            local_send_time: now + duration_us(50),
            remote_recv_time: now + duration_us(225),
            local_recv_time: now + duration_us(275),
        },
        AckedPacket {
            seq_no: 2.into(),
            size: DataSize::ZERO,
            local_send_time: now + duration_us(1005),
            remote_recv_time: now + duration_us(1140),
            local_recv_time: now + duration_us(1190),
        },
        AckedPacket {
            seq_no: 3.into(),
            size: DataSize::ZERO,
            local_send_time: now + duration_us(4995),
            remote_recv_time: now + duration_us(5001),
            local_recv_time: now + duration_us(5051),
        },
        // Should not belong
        AckedPacket {
            seq_no: 4.into(),
            size: DataSize::ZERO,
            local_send_time: now + duration_us(5700),
            remote_recv_time: now + duration_us(6000),
            local_recv_time: now + duration_us(5750),
        },
    ];

    let mut group = ArrivalGroup::default();

    for p in packets {
        let need_new_group = group.belongs_to_group(&p).new_group();
        if !need_new_group {
            group.add_packet(&p);
        }
    }

    assert_eq!(group.size, 4, "Expected group to contain 4 packets");
}

#[test]
fn test_arrival_group_out_order_arrival_ignored() {
    let now = Instant::now();
    let packets = vec![
        AckedPacket {
            seq_no: 0.into(),
            size: DataSize::ZERO,
            local_send_time: now,
            remote_recv_time: now + duration_us(150),
            local_recv_time: now + duration_us(200),
        },
        AckedPacket {
            seq_no: 1.into(),
            size: DataSize::ZERO,
            local_send_time: now + duration_us(50),
            remote_recv_time: now + duration_us(225),
            local_recv_time: now + duration_us(275),
        },
        AckedPacket {
            seq_no: 2.into(),
            size: DataSize::ZERO,
            local_send_time: now + duration_us(1005),
            remote_recv_time: now + duration_us(1140),
            local_recv_time: now + duration_us(1190),
        },
        AckedPacket {
            seq_no: 3.into(),
            size: DataSize::ZERO,
            local_send_time: now + duration_us(4995),
            remote_recv_time: now + duration_us(5001),
            local_recv_time: now + duration_us(5051),
        },
        // Should be skipped
        AckedPacket {
            seq_no: 4.into(),
            size: DataSize::ZERO,
            local_send_time: now.checked_sub(duration_us(100)).unwrap(),
            remote_recv_time: now + duration_us(5000),
            local_recv_time: now + duration_us(5050),
        },
        // Should not belong
        AckedPacket {
            seq_no: 5.into(),
            size: DataSize::ZERO,
            local_send_time: now + duration_us(5700),
            remote_recv_time: now + duration_us(6000),
            local_recv_time: now + duration_us(6050),
        },
    ];

    let mut group = ArrivalGroup::default();

    for p in packets {
        let need_new_group = group.belongs_to_group(&p).new_group();
        if !need_new_group {
            group.add_packet(&p);
        }
    }

    assert_eq!(group.size, 4, "Expected group to contain 4 packets");
}

#[test]
fn test_arrival_group_arrival_membership() {
    let now = Instant::now();
    let packets = vec![
        AckedPacket {
            seq_no: 0.into(),
            size: DataSize::ZERO,
            local_send_time: now,
            remote_recv_time: now + duration_us(150),
            local_recv_time: now + duration_us(200),
        },
        AckedPacket {
            seq_no: 1.into(),
            size: DataSize::ZERO,
            local_send_time: now + duration_us(50),
            remote_recv_time: now + duration_us(225),
            local_recv_time: now + duration_us(275),
        },
        AckedPacket {
            seq_no: 2.into(),
            size: DataSize::ZERO,
            local_send_time: now + duration_us(5152),
            // Just less than 5ms inter arrival delta
            remote_recv_time: now + duration_us(5224),
            local_recv_time: now + duration_us(5274),
        },
        // Should not belong
        AckedPacket {
            seq_no: 3.into(),
            size: DataSize::ZERO,
            local_send_time: now + duration_us(5700),
            remote_recv_time: now + duration_us(6000),
            local_recv_time: now + duration_us(6050),
        },
    ];

    let mut group = ArrivalGroup::default();

    for p in packets {
        let need_new_group = group.belongs_to_group(&p).new_group();
        if !need_new_group {
            group.add_packet(&p);
        }
    }

    assert_eq!(group.size, 3, "Expected group to contain 4 packets");
}

#[test]
fn group_reorder() {
    let data = vec![
        ((Duration::from_millis(0), Duration::from_millis(0)), None),
        ((Duration::from_millis(60), Duration::from_millis(5)), None),
        ((Duration::from_millis(40), Duration::from_millis(10)), None),
        (
            (Duration::from_millis(70), Duration::from_millis(20)),
            Some((TimeDelta::from_millis(-20), TimeDelta::from_millis(5))),
        ),
    ];

    let now = Instant::now();
    let mut aga = ArrivalGroupAccumulator::default();

    for ((local_send_time, remote_recv_time), deltas) in data {
        let group_delta = aga.accumulate_packet(&AckedPacket {
            seq_no: TwccSeq::default(),
            size: DataSize::default(),
            local_send_time: now + local_send_time,
            remote_recv_time: now + remote_recv_time,
            local_recv_time: Instant::now(), // does not matter
        });

        assert_eq!(group_delta.map(|d| (d.send_delta, d.arrival_delta)), deltas);
    }
}

fn duration_us(us: u64) -> Duration {
    Duration::from_micros(us)
}
