use super::ForkedRandom;
use rekindle_utils::random::{OsRandom, RandomSource};

#[test]
fn a_fork_follows_its_parent_only() {
    let fork = |parent: &dyn RandomSource| {
        let child = ForkedRandom::fork(parent);
        (child.next_u32(), child.next_u32())
    };
    let (a, b) = (OsRandom, OsRandom);
    assert_ne!(fork(&a), fork(&b));
}

/// A parent that hands out the same bytes every time.
struct Fixed(u8);

impl rand::CryptoRng for Fixed {}

impl RandomSource for Fixed {
    fn fill(&self, dest: &mut [u8]) {
        dest.fill(self.0);
    }
}

fn draw(rng: &dyn RandomSource) -> [u8; 64] {
    let mut out = [0u8; 64];
    rng.fill(&mut out);
    out
}

#[test]
fn one_parent_seed_gives_one_sequence() {
    let (a, b) = (ForkedRandom::fork(&Fixed(7)), ForkedRandom::fork(&Fixed(7)));
    for _ in 0..4 {
        assert_eq!(draw(&a), draw(&b));
    }
    assert_ne!(draw(&ForkedRandom::fork(&Fixed(8))), draw(&b));
}

#[test]
fn captured_state_cannot_regenerate_earlier_output() {
    let rng = ForkedRandom::fork(&Fixed(7));
    let earlier = draw(&rng);
    // Everything the generator keeps after the fill, copied out as an
    // attacker who reads its memory would.
    let captured = rng.0.lock().clone();
    assert_ne!(*captured, [7u8; 32], "the seed was replaced");
    let copy = ForkedRandom::fork(&Fixed(0));
    *copy.0.lock() = captured;
    let mut replay = vec![0u8; 64 * 64];
    copy.fill(&mut replay);
    assert!(replay.windows(64).all(|w| w != earlier));
    // The copy can only follow the original forward.
    let copy = ForkedRandom::fork(&Fixed(0));
    *copy.0.lock() = rng.0.lock().clone();
    assert_eq!(draw(&copy), draw(&rng));
}
