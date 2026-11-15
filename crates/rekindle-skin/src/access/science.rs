//! The colour science of validator step 6, every formula from the sources
//! `skins/research/d3-distinctness.md` §6.2 verified: sRGB decode (WCAG
//! 2.2), the CSS Color 4 linear-sRGB → XYZ (D65) matrix, CIELAB, CIEDE2000
//! (Sharma, Wu and Dalal 2005, kL = kC = kH = 1), OKLab/OKLCH (CSS Color 4)
//! and colour-vision simulation in linear sRGB (Viénot 1999 protan and
//! deutan, Brettel 1997 tritan, severity 1, as libDaltonLens re-derives them
//! for sRGB; §2.3). There is no 8-bit re-quantisation between steps.

/// A colour in linear sRGB, each channel in `[0, 1]`.
pub type Linear = [f64; 3];

type Matrix = [[f64; 3]; 3];

fn mul(m: &Matrix, v: Linear) -> Linear {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Gamma-encoded sRGB to linear (`wcag22.txt:1790-1792`).
#[must_use]
pub fn linear(rgb: [u8; 3]) -> Linear {
    rgb.map(|c| {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    })
}

/// CSS Color 4 `lin_sRGB_to_XYZ`, the rational form (`css-color-4.txt:11546-11556`).
const TO_XYZ: Matrix = [
    [
        506_752.0 / 1_228_815.0,
        87_881.0 / 245_763.0,
        12_673.0 / 70_218.0,
    ],
    [
        87_098.0 / 409_605.0,
        175_762.0 / 245_763.0,
        12_673.0 / 175_545.0,
    ],
    [
        7_918.0 / 409_605.0,
        87_881.0 / 737_289.0,
        1_001_167.0 / 1_053_270.0,
    ],
];

/// CIELAB (D65 white from the chromaticity `0.3127, 0.3290`).
#[must_use]
pub fn lab(c: Linear) -> [f64; 3] {
    const WHITE: [f64; 3] = [0.3127 / 0.3290, 1.0, (1.0 - 0.3127 - 0.3290) / 0.3290];
    const EPSILON: f64 = 216.0 / 24_389.0;
    const KAPPA: f64 = 24_389.0 / 27.0;
    let xyz = mul(&TO_XYZ, c);
    let f = [0, 1, 2].map(|i| {
        let r = xyz[i] / WHITE[i];
        if r > EPSILON {
            r.cbrt()
        } else {
            (KAPPA * r + 16.0) / 116.0
        }
    });
    [
        116.0 * f[1] - 16.0,
        500.0 * (f[0] - f[1]),
        200.0 * (f[1] - f[2]),
    ]
}

/// CSS Color 4 `XYZ_to_OKLab` (`css-color-4.txt:11893-11910`).
#[must_use]
pub fn oklab(c: Linear) -> [f64; 3] {
    const TO_LMS: Matrix = [
        [
            0.819_022_437_996_703,
            0.361_906_260_052_890_4,
            -0.128_873_781_520_987_9,
        ],
        [
            0.032_983_653_932_388_5,
            0.929_286_861_586_343_4,
            0.036_144_666_350_642_4,
        ],
        [
            0.048_177_189_359_624_2,
            0.264_239_531_752_730_9,
            0.633_547_828_469_430_9,
        ],
    ];
    const TO_OK: Matrix = [
        [
            0.210_454_268_309_314,
            0.793_617_774_702_305_4,
            -0.004_072_043_011_619_3,
        ],
        [
            1.977_998_532_431_168_4,
            -2.428_592_242_048_58,
            0.450_593_709_617_411,
        ],
        [
            0.025_904_042_465_547_8,
            0.782_771_712_457_529_6,
            -0.808_675_754_923_077_4,
        ],
    ];
    mul(&TO_OK, mul(&TO_LMS, mul(&TO_XYZ, c)).map(f64::cbrt))
}

/// OKLCH chroma and hue in degrees `[0, 360)`.
#[must_use]
pub fn oklch(c: Linear) -> (f64, f64) {
    let [_, a, b] = oklab(c);
    (a.hypot(b), b.atan2(a).to_degrees().rem_euclid(360.0))
}

/// WCAG relative luminance of a linear colour (`wcag22.txt:1788`).
#[must_use]
pub fn luminance(c: Linear) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn hue(b: f64, a: f64) -> f64 {
    if a == 0.0 && b == 0.0 {
        0.0
    } else {
        b.atan2(a).to_degrees().rem_euclid(360.0)
    }
}

/// CIEDE2000, Sharma et al. 2005 Eqs. 1–22 with the mean-hue and
/// hue-difference conventions of their `deltaE2000.m`.
#[must_use]
pub fn delta_e00(lab1: [f64; 3], lab2: [f64; 3]) -> f64 {
    let pow7 = |c: f64| c.powi(7);
    let [l1, a1, b1] = lab1;
    let [l2, a2, b2] = lab2;
    let c_mean = f64::midpoint(a1.hypot(b1), a2.hypot(b2));
    let g = 0.5 * (1.0 - (pow7(c_mean) / (pow7(c_mean) + pow7(25.0))).sqrt());
    let (a1p, a2p) = ((1.0 + g) * a1, (1.0 + g) * a2);
    let (c1p, c2p) = (a1p.hypot(b1), a2p.hypot(b2));
    let (h1p, h2p) = (hue(b1, a1p), hue(b2, a2p));
    let product = c1p * c2p;
    let dhp = if product == 0.0 {
        0.0
    } else {
        let d = h2p - h1p;
        if d > 180.0 {
            d - 360.0
        } else if d < -180.0 {
            d + 360.0
        } else {
            d
        }
    };
    let d_l = l2 - l1;
    let d_c = c2p - c1p;
    let d_h = 2.0 * product.sqrt() * (dhp / 2.0).to_radians().sin();
    let l_mean = f64::midpoint(l1, l2);
    let cp_mean = f64::midpoint(c1p, c2p);
    let h_mean = if product == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        f64::midpoint(h1p, h2p)
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) / 2.0
    } else {
        (h1p + h2p - 360.0) / 2.0
    };
    let cos = |deg: f64| deg.to_radians().cos();
    let t =
        1.0 - 0.17 * cos(h_mean - 30.0) + 0.24 * cos(2.0 * h_mean) + 0.32 * cos(3.0 * h_mean + 6.0)
            - 0.20 * cos(4.0 * h_mean - 63.0);
    let d_theta = 30.0 * (-((h_mean - 275.0) / 25.0).powi(2)).exp();
    let r_c = 2.0 * (pow7(cp_mean) / (pow7(cp_mean) + pow7(25.0))).sqrt();
    let s_l = 1.0 + 0.015 * (l_mean - 50.0).powi(2) / (20.0 + (l_mean - 50.0).powi(2)).sqrt();
    let s_c = 1.0 + 0.045 * cp_mean;
    let s_h = 1.0 + 0.015 * cp_mean * t;
    let r_t = -(2.0 * d_theta).to_radians().sin() * r_c;
    let (tl, tc, th) = (d_l / s_l, d_c / s_c, d_h / s_h);
    (tl * tl + tc * tc + th * th + r_t * tc * th).sqrt()
}

/// A vision the D2 distance is measured under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vision {
    Normal,
    /// Viénot 1999, severity 1.
    Protan,
    /// Viénot 1999, severity 1.
    Deutan,
    /// Brettel 1997, two half-planes, severity 1.
    Tritan,
}

impl Vision {
    pub const ALL: [Self; 4] = [Self::Normal, Self::Protan, Self::Deutan, Self::Tritan];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Protan => "protan",
            Self::Deutan => "deutan",
            Self::Tritan => "tritan",
        }
    }

    /// `c` as this vision sees it, each channel clamped to `[0, 1]`
    /// (`libDaltonLens.c:139-151, 182-191, 228-238`).
    #[must_use]
    pub fn simulate(self, c: Linear) -> Linear {
        const PROTAN: Matrix = [
            [0.11238, 0.88762, 0.0],
            [0.11238, 0.88762, 0.0],
            [0.00401, -0.00401, 1.0],
        ];
        const DEUTAN: Matrix = [
            [0.29275, 0.70725, 0.0],
            [0.29275, 0.70725, 0.0],
            [-0.02234, 0.02234, 1.0],
        ];
        const TRITAN_NORMAL: Linear = [0.03901, -0.02788, -0.01113];
        const TRITAN_1: Matrix = [
            [1.01277, 0.13548, -0.14826],
            [-0.01243, 0.86812, 0.14431],
            [0.07589, 0.80500, 0.11911],
        ];
        const TRITAN_2: Matrix = [
            [0.93678, 0.18979, -0.12657],
            [0.06154, 0.81526, 0.12320],
            [-0.37562, 1.12767, 0.24796],
        ];
        let out = match self {
            Self::Normal => return c,
            Self::Protan => mul(&PROTAN, c),
            Self::Deutan => mul(&DEUTAN, c),
            Self::Tritan => {
                let side: f64 = TRITAN_NORMAL.iter().zip(c).map(|(n, x)| n * x).sum();
                mul(if side >= 0.0 { &TRITAN_1 } else { &TRITAN_2 }, c)
            }
        };
        out.map(|x| x.clamp(0.0, 1.0))
    }
}

/// ΔE00 between `a` and `b` as `vision` sees them.
#[must_use]
pub fn distance(a: [u8; 3], b: [u8; 3], vision: Vision) -> f64 {
    let see = |c| lab(vision.simulate(linear(c)));
    delta_e00(see(a), see(b))
}
