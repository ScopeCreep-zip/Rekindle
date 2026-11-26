//! Shared fixtures for the skin-tree import tests: an empty frame and solid
//! art pieces sized by their names.

use rekindle_image::{Rgba, RgbaImage};
use rekindle_types::dto::skin::{Insets, WindowLayout};
use rekindle_types::dto::skin_tree::SkinNode;

/// No insets on any side.
pub(super) const Z: Insets = Insets {
    top: 0,
    right: 0,
    bottom: 0,
    left: 0,
};

/// An empty frame with a small declared minimum, the importer's start.
pub(super) fn frame() -> WindowLayout {
    WindowLayout {
        outsets: Z,
        content: Z,
        min_width: 30,
        min_height: 20,
        tiles: Vec::new(),
        title: None,
        tree: None,
        popups: std::collections::BTreeMap::new(),
    }
}

/// A solid image sized by its name's `-WxH` suffix.
pub(super) fn piece(path: &str) -> RgbaImage {
    let (w, h) = path
        .rsplit_once('-')
        .and_then(|(_, d)| d.split_once('x'))
        .unwrap();
    RgbaImage::from_pixel(
        w.parse().unwrap(),
        h.parse().unwrap(),
        Rgba([10, 20, 30, 255]),
    )
}

/// The node `id` under `nodes`, depth first.
pub(super) fn find<'a>(nodes: &'a [SkinNode], id: &str) -> Option<&'a SkinNode> {
    nodes
        .iter()
        .find_map(|n| (n.id == id).then_some(n).or_else(|| find(&n.children, id)))
}

/// An exact length, `num`/`den` px (`den` > 0): the stylesheet's
/// `calc(...)` evaluated without rounding but where it rounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Ratio {
    num: i64,
    den: i64,
}

impl Ratio {
    pub(super) const fn whole(n: i64) -> Self {
        Self { num: n, den: 1 }
    }

    fn norm(num: i64, den: i64) -> Option<Self> {
        if den == 0 {
            return None;
        }
        let (mut a, mut b) = (num.abs(), den.abs());
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let g = a.max(1) * den.signum();
        Some(Self {
            num: num / g,
            den: den / g,
        })
    }

    pub(super) fn add(self, o: Self) -> Self {
        Self::norm(self.num * o.den + o.num * self.den, self.den * o.den).unwrap_or(self)
    }

    pub(super) const fn neg(self) -> Self {
        Self {
            num: -self.num,
            den: self.den,
        }
    }

    fn mul(self, o: Self) -> Option<Self> {
        Self::norm(self.num * o.num, self.den * o.den)
    }

    fn div(self, o: Self) -> Option<Self> {
        Self::norm(self.num * o.den, self.den * o.num)
    }

    /// `self` floored to a multiple of `step` (CSS `round(down, …)`).
    fn floor_to(self, step: Self) -> Option<Self> {
        let q = self.div(step)?;
        Self::whole(q.num.div_euclid(q.den)).mul(step)
    }

    /// The whole pixel `self` is, `None` for a fraction.
    pub(super) fn px(self) -> Option<i32> {
        (self.den == 1)
            .then(|| i32::try_from(self.num).ok())
            .flatten()
    }
}

/// A CSS length of the subset the stylesheet emits (`px`, `%` of `len`,
/// `calc(...)`, `round(down, …, 1px)`, `+ - * /`) evaluated at a parent
/// `len` px long, as the browser evaluates it; `None` for anything else.
pub(super) fn css_len(v: &str, len: i32) -> Option<Ratio> {
    let mut p = CssExpr {
        s: v.trim().as_bytes(),
        i: 0,
        len: i64::from(len),
    };
    let out = p.sum()?;
    p.ws();
    (p.i == p.s.len()).then_some(out)
}

struct CssExpr<'a> {
    s: &'a [u8],
    i: usize,
    len: i64,
}

impl CssExpr<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i) == Some(&b' ') {
            self.i += 1;
        }
    }

    fn eat(&mut self, t: &str) -> bool {
        self.ws();
        let hit = self.s[self.i..].starts_with(t.as_bytes());
        if hit {
            self.i += t.len();
        }
        hit
    }

    fn sum(&mut self) -> Option<Ratio> {
        let mut acc = self.product()?;
        loop {
            if self.eat("+ ") {
                acc = acc.add(self.product()?);
            } else if self.eat("- ") {
                acc = acc.add(self.product()?.neg());
            } else {
                return Some(acc);
            }
        }
    }

    fn product(&mut self) -> Option<Ratio> {
        let mut acc = self.atom()?;
        loop {
            if self.eat("*") {
                acc = acc.mul(self.atom()?)?;
            } else if self.eat("/") {
                acc = acc.div(self.atom()?)?;
            } else {
                return Some(acc);
            }
        }
    }

    fn atom(&mut self) -> Option<Ratio> {
        if self.eat("calc(") || self.eat("(") {
            let v = self.sum()?;
            return self.eat(")").then_some(v);
        }
        if self.eat("round(down,") {
            let v = self.sum()?;
            let step = if self.eat(",") {
                self.sum()?
            } else {
                return None;
            };
            return if self.eat(")") {
                v.floor_to(step)
            } else {
                None
            };
        }
        self.ws();
        let from = self.i;
        if self.s.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
            self.i += 1;
        }
        let n: i64 = std::str::from_utf8(&self.s[from..self.i])
            .ok()?
            .parse()
            .ok()?;
        if self.eat("%") {
            Ratio::whole(n * self.len).div(Ratio::whole(100))
        } else {
            self.eat("px");
            Some(Ratio::whole(n))
        }
    }
}
