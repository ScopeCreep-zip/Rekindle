//! A/B finding R1-7 (plan E3.2): `community info` counts the members of
//! the merged governance (it showed 1 on both sides of a two-member
//! community) and names each channel's held text-key generation (it showed
//! 0 while generation 1 was held).

use std::time::Duration;

use crate::harness::{eventually, Community};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn info_counts_the_merged_members_and_the_held_key() {
    let community = Community::of(2, 152).await;
    for i in [0, 1] {
        assert!(
            eventually(&community.sim, Duration::from_secs(60), || async {
                community.info(i).await["member_count"] == 2
            })
            .await,
            "node {i} counts both members: {}",
            community.info(i).await
        );
        let info = community.info(i).await;
        let general = (info["channels"].as_array().into_iter().flatten())
            .find(|c| c["name"] == "general")
            .cloned()
            .expect("a general channel");
        let held = community.mek_list(i).await;
        let newest = (held.as_array().into_iter().flatten())
            .filter_map(|k| k["generation"].as_u64())
            .max()
            .expect("a held key");
        assert!(newest >= 1);
        assert_eq!(
            general["mekGeneration"].as_u64(),
            Some(newest),
            "node {i}'s general channel names the held generation: {info}"
        );
    }
}
