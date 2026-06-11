// Label must NOT be constructible from outside the crate.
// No From<String>, no FromStr, no tuple constructor.
use rekindle_identity::Label;

fn main() {
    let _label: Label = "origin.seed".parse().unwrap(); // must not compile — no FromStr
}
