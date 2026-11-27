use super::*;
use crate::colour;

#[test]
fn oklab_round_trips_and_walks_minimally() {
    for c in [[12, 51, 87], [255, 255, 255], [0, 0, 0], [26, 103, 175]] {
        assert_eq!(from_oklab(oklab(c)), c);
    }
    let ground = [22, 22, 22];
    let new = walk([12, 51, 87], true, |c| {
        Ok::<_, std::convert::Infallible>(colour::contrast(c, ground) >= 4.5)
    })
    .unwrap()
    .unwrap();
    assert!(colour::contrast(new, ground) >= 4.5);
    let [l, ..] = oklab(new);
    let less = from_oklab([l - 0.01, oklab(new)[1], oklab(new)[2]]);
    assert!(
        colour::contrast(less, ground) < 4.5,
        "{new:?} is not minimal"
    );
}
