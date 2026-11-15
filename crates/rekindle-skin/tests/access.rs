//! Validator step 6: the colour science against its conformance data and
//! the test vectors of `skins/research/d3-distinctness.md` §7, numbers from
//! its `raw/d3/calc/` scripts (the oracle).

use std::collections::BTreeMap;

use rekindle_skin::access::science::{delta_e00, distance, lab, linear, luminance, oklab, Vision};
use rekindle_skin::access::{danger_failures, failures};
use rekindle_skin::registry::{resolve, Base, ENTRIES};
use rekindle_types::dto::skin::{SkinRule, TokenValue};

/// Sharma, Wu and Dalal 2005, all 34 rows: L1 a1 b1 L2 a2 b2 ΔE00.
const SHARMA: [[f64; 7]; 34] = [
    [50.0000, 2.6772, -79.7751, 50.0000, 0.0000, -82.7485, 2.0425],
    [50.0000, 3.1571, -77.2803, 50.0000, 0.0000, -82.7485, 2.8615],
    [50.0000, 2.8361, -74.0200, 50.0000, 0.0000, -82.7485, 3.4412],
    [
        50.0000, -1.3802, -84.2814, 50.0000, 0.0000, -82.7485, 1.0000,
    ],
    [
        50.0000, -1.1848, -84.8006, 50.0000, 0.0000, -82.7485, 1.0000,
    ],
    [
        50.0000, -0.9009, -85.5211, 50.0000, 0.0000, -82.7485, 1.0000,
    ],
    [50.0000, 0.0000, 0.0000, 50.0000, -1.0000, 2.0000, 2.3669],
    [50.0000, -1.0000, 2.0000, 50.0000, 0.0000, 0.0000, 2.3669],
    [50.0000, 2.4900, -0.0010, 50.0000, -2.4900, 0.0009, 7.1792],
    [50.0000, 2.4900, -0.0010, 50.0000, -2.4900, 0.0010, 7.1792],
    [50.0000, 2.4900, -0.0010, 50.0000, -2.4900, 0.0011, 7.2195],
    [50.0000, 2.4900, -0.0010, 50.0000, -2.4900, 0.0012, 7.2195],
    [50.0000, -0.0010, 2.4900, 50.0000, 0.0009, -2.4900, 4.8045],
    [50.0000, -0.0010, 2.4900, 50.0000, 0.0010, -2.4900, 4.8045],
    [50.0000, -0.0010, 2.4900, 50.0000, 0.0011, -2.4900, 4.7461],
    [50.0000, 2.5000, 0.0000, 50.0000, 0.0000, -2.5000, 4.3065],
    [50.0000, 2.5000, 0.0000, 73.0000, 25.0000, -18.0000, 27.1492],
    [50.0000, 2.5000, 0.0000, 61.0000, -5.0000, 29.0000, 22.8977],
    [50.0000, 2.5000, 0.0000, 56.0000, -27.0000, -3.0000, 31.9030],
    [50.0000, 2.5000, 0.0000, 58.0000, 24.0000, 15.0000, 19.4535],
    [50.0000, 2.5000, 0.0000, 50.0000, 3.1736, 0.5854, 1.0000],
    [50.0000, 2.5000, 0.0000, 50.0000, 3.2972, 0.0000, 1.0000],
    [50.0000, 2.5000, 0.0000, 50.0000, 1.8634, 0.5757, 1.0000],
    [50.0000, 2.5000, 0.0000, 50.0000, 3.2592, 0.3350, 1.0000],
    [
        60.2574, -34.0099, 36.2677, 60.4626, -34.1751, 39.4387, 1.2644,
    ],
    [
        63.0109, -31.0961, -5.8663, 62.8187, -29.7946, -4.0864, 1.2630,
    ],
    [61.2901, 3.7196, -5.3901, 61.4292, 2.2480, -4.9620, 1.8731],
    [35.0831, -44.1164, 3.7933, 35.0232, -40.0716, 1.5901, 1.8645],
    [
        22.7233, 20.0904, -46.6940, 23.0331, 14.9730, -42.5619, 2.0373,
    ],
    [36.4612, 47.8580, 18.3852, 36.2715, 50.5065, 21.2231, 1.4146],
    [90.8027, -2.0831, 1.4410, 91.1528, -1.6435, 0.0447, 1.4441],
    [90.9257, -0.5406, -0.9208, 88.6381, -0.8985, -0.7239, 1.5381],
    [6.7747, -0.2908, -2.4247, 5.8714, -0.0985, -2.2286, 0.6377],
    [2.0776, 0.0795, -1.1350, 0.9033, -0.0636, -0.5514, 0.9082],
];

#[test]
fn ciede2000_conforms_to_sharma() {
    for row in SHARMA {
        let d = delta_e00([row[0], row[1], row[2]], [row[3], row[4], row[5]]);
        assert!((d - row[6]).abs() < 1e-4, "{row:?}: {d}");
        let back = delta_e00([row[3], row[4], row[5]], [row[0], row[1], row[2]]);
        assert!((back - row[6]).abs() < 1e-4, "{row:?} reversed: {back}");
    }
}

fn hex(text: &str) -> [u8; 3] {
    let value = u32::from_str_radix(text.trim_start_matches('#'), 16).expect("hex");
    let [_, red, green, blue] = value.to_be_bytes();
    [red, green, blue]
}

fn close(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < tol)
}

#[test]
fn spot_values_match_the_oracle() {
    let red = linear(hex("#ef4444"));
    assert!(
        close(lab(red), [54.975, 64.686, 39.128], 1e-3),
        "{:?}",
        lab(red)
    );
    assert!(
        close(oklab(red), [0.6368, 0.1879, 0.0889], 1e-4),
        "{:?}",
        oklab(red)
    );
    assert!((luminance(red) - 0.2290).abs() < 1e-4);
    let blue = linear(hex("#177cc1"));
    assert!(
        close(lab(blue), [50.033, -1.893, -43.572], 1e-3),
        "{:?}",
        lab(blue)
    );
    assert!(
        close(oklab(blue), [0.5673, -0.0564, -0.1243], 1e-4),
        "{:?}",
        oklab(blue)
    );
    assert!((luminance(blue) - 0.1845).abs() < 1e-4);
}

/// The failures as `rule/vision` tags, for comparison with d3 §7.1.
fn tags(rules: &[SkinRule]) -> Vec<String> {
    rules
        .iter()
        .map(|r| match r {
            SkinRule::DangerNotRed { .. } => "D1".to_owned(),
            SkinRule::AccentIsRed { .. } => "D3".to_owned(),
            SkinRule::TooClose { rule, vision, .. } => format!("{rule}/{vision}"),
            other => format!("{other}"),
        })
        .collect()
}

/// d3 §7.1 V1–V14: danger, accent, ΔE00 normal/protan/deutan/tritan
/// (`validator.py`, three decimals) and the expected failures.
const VECTORS: &[(&str, &str, [f64; 4], &[&str])] = &[
    ("#ef4444", "#177cc1", [48.691, 44.217, 58.995, 61.894], &[]),
    (
        "#177cc1",
        "#177cc1",
        [0.0; 4],
        &["D1", "D2/normal", "D2/protan", "D2/deutan", "D2/tritan"],
    ),
    (
        "#ef4444",
        "#ef4444",
        [0.0; 4],
        &["D3", "D2/normal", "D2/protan", "D2/deutan", "D2/tritan"],
    ),
    (
        "#e01b24",
        "#e62d42",
        [6.378, 6.897, 4.930, 3.005],
        &["D3", "D2/normal", "D2/protan", "D2/deutan", "D2/tritan"],
    ),
    ("#b3261e", "#6750a4", [37.459, 47.746, 56.066, 30.789], &[]),
    ("#f2b8b5", "#d0bcff", [23.438, 27.713, 33.415, 19.752], &[]),
    ("#ff3b30", "#007aff", [48.603, 59.615, 72.275, 63.325], &[]),
    (
        "#ec4899",
        "#177cc1",
        [46.835, 7.933, 25.669, 59.339],
        &["D1", "D2/protan"],
    ),
    (
        "#ea580c",
        "#177cc1",
        [50.222, 55.713, 63.647, 60.484],
        &["D1"],
    ),
    (
        "#d32f2f",
        "#388e3c",
        [65.974, 19.135, 5.411, 50.027],
        &["D2/deutan"],
    ),
    (
        "#ef4444",
        "#f97316",
        [19.721, 17.032, 9.085, 8.254],
        &["D2/normal", "D2/deutan", "D2/tritan"],
    ),
    ("#ef4444", "#14b8a6", [64.777, 26.482, 31.194, 65.349], &[]),
    (
        "#ef4444",
        "#d946ef",
        [34.792, 52.628, 59.783, 10.925],
        &["D2/tritan"],
    ),
    (
        "#ef4444",
        "#991b1b",
        [20.747, 16.280, 22.596, 20.713],
        &["D3"],
    ),
];

#[test]
fn danger_vectors_v1_to_v14() {
    for (i, (danger, accent, de, expected)) in VECTORS.iter().enumerate() {
        for (vision, want) in Vision::ALL.iter().zip(de) {
            let got = distance(hex(danger), hex(accent), *vision);
            assert!(
                (got - want).abs() < 5e-4,
                "V{} {vision:?}: {got} vs {want}",
                i + 1
            );
        }
        let got = tags(&danger_failures(hex(danger), hex(accent)));
        assert_eq!(got, *expected, "V{}", i + 1);
    }
}

#[test]
fn danger_errors_carry_the_measured_values() {
    // V8: pink danger, hue 354.3°, chroma 0.212; protan 7.9 < 14.
    let got = danger_failures(hex("#ec4899"), hex("#177cc1"));
    assert_eq!(
        got[0],
        SkinRule::DangerNotRed {
            hue_x10: 3543,
            chroma_x1000: 211
        }
    );
    assert!(got.contains(&SkinRule::TooClose {
        rule: "D2".into(),
        a: "danger.fill".into(),
        b: "accent.fill".into(),
        vision: "protan".into(),
        measured_x10: 79,
        required_x10: 140,
    }));
}

fn colour(text: &str, alpha: u8) -> TokenValue {
    let [red, green, blue] = hex(text);
    TokenValue::Color {
        r: red,
        g: green,
        b: blue,
        a: alpha,
    }
}

fn check(base: Base, skin: &[(&str, TokenValue)]) -> Vec<SkinRule> {
    let skin: BTreeMap<String, TokenValue> = skin
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect();
    failures(base, &resolve(ENTRIES, base, &skin))
}

#[test]
fn dark_defaults_pass() {
    assert_eq!(check(Base::Dark, &[]), vec![]);
    assert_eq!(check(Base::HcDark, &[]), vec![]);
}

#[test]
fn a_tint_is_measured_composited_over_its_ground() {
    // d3 §7.2: white on #ef4444 at 60 % over #1e1e1e is #9b3535, 7.11:1,
    // which passes; the solid hover fill is 3.76:1, which fails.
    let got = check(Base::Dark, &[("danger.fill", colour("#ef4444", 255))]);
    assert_eq!(
        got,
        vec![
            SkinRule::Contrast {
                rule: "1.4.3".into(),
                token: "danger.fg".into(),
                ground: "danger.fill".into(),
                measured_x100: 376,
                required_x100: 450,
            },
            SkinRule::Contrast {
                rule: "1.4.3".into(),
                token: "text.on-fill".into(),
                ground: "danger.fill".into(),
                measured_x100: 376,
                required_x100: 450,
            }
        ]
    );
    // A 20 % tint over a light status bar puts white text on a light
    // composite: D4 names the composited ground.
    let got = check(
        Base::Dark,
        &[
            ("tint.fill", TokenValue::Number(200)),
            ("bg.status", colour("#d0d0d0", 255)),
        ],
    );
    assert!(
        got.iter()
            .any(|r| matches!(r, SkinRule::Contrast { rule, ground, .. }
            if rule == "D4" && ground == "danger.fill 20.0% over bg.status")),
        "{got:?}"
    );
}

#[test]
fn a_translucent_ground_is_refused() {
    let got = check(Base::Dark, &[("bg.panel", colour("#1e1e1e", 0x80))]);
    assert!(got.contains(&SkinRule::TranslucentGround {
        token: "bg.panel".into()
    }));
}

#[test]
fn the_window_ground_must_match_the_declared_base() {
    let got = check(Base::Dark, &[("bg.dark", colour("#ffffff", 255))]);
    assert!(got.contains(&SkinRule::BaseMismatch {
        declared: "dark".into(),
        token: "bg.dark".into(),
        luminance_x1000: 1000,
    }));
}

#[test]
fn status_colours_stay_apart_and_visible() {
    // d3 §7.3: in-game = online fails S1 at 0.0.
    let got = check(Base::Dark, &[("status.ingame", colour("#53d769", 255))]);
    assert!(got.contains(&SkinRule::TooClose {
        rule: "S1".into(),
        a: "status.online".into(),
        b: "status.ingame".into(),
        vision: "normal".into(),
        measured_x10: 0,
        required_x10: 200,
    }));
    // d3 §7.3: the dark defaults on a light base, online 1.86:1 on white.
    let got = check(Base::Light, &[("status.online", colour("#53d769", 255))]);
    assert!(
        got.contains(&SkinRule::Contrast {
            rule: "S2".into(),
            token: "status.online".into(),
            ground: "the StatusDot cutout".into(),
            measured_x100: 185,
            required_x100: 300,
        }),
        "{got:?}"
    );
}
