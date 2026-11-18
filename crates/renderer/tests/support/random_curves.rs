//! Seeded random curves for the draw/pick property tests: random compositions
//! of elementary functions, sampled unevenly, sometimes parametric, sometimes
//! with a gap, and an optional off-screen lead that runs the arc far before
//! the visible part. A case is a pure function of its seed, so a failure
//! names the seed that replays it.

/// SplitMix64: small, seedable and reproducible on every platform.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in [lo, hi).
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }

    /// Uniform in 0..n.
    pub fn index(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
}

/// An elementary function of one variable.
#[derive(Clone, Debug)]
pub enum Leaf {
    Sin { a: f64, b: f64 },
    Gauss { a: f64, b: f64 },
    Tanh { a: f64, b: f64 },
    Cubic { c: [f64; 4] },
    Abs { b: f64 },
    Saw { a: f64 },
    Steps { a: f64 },
    SqrtAbs,
    Log1pAbs,
    Lorentz { a: f64 },
    Chirp { a: f64 },
}

impl Leaf {
    fn random(rng: &mut Rng) -> Self {
        match rng.index(11) {
            0 => Leaf::Sin {
                a: rng.range(0.3, 9.0),
                b: rng.range(-3.0, 3.0),
            },
            1 => Leaf::Gauss {
                a: rng.range(0.2, 6.0),
                b: rng.range(-1.0, 1.0),
            },
            2 => Leaf::Tanh {
                a: rng.range(0.5, 12.0),
                b: rng.range(-1.0, 1.0),
            },
            3 => Leaf::Cubic {
                c: [
                    rng.range(-1.0, 1.0),
                    rng.range(-2.0, 2.0),
                    rng.range(-2.0, 2.0),
                    rng.range(-2.0, 2.0),
                ],
            },
            4 => Leaf::Abs {
                b: rng.range(-1.0, 1.0),
            },
            5 => Leaf::Saw {
                a: rng.range(0.5, 6.0),
            },
            6 => Leaf::Steps {
                a: rng.range(1.0, 8.0),
            },
            7 => Leaf::SqrtAbs,
            8 => Leaf::Log1pAbs,
            9 => Leaf::Lorentz {
                a: rng.range(1.0, 20.0),
            },
            _ => Leaf::Chirp {
                a: rng.range(1.0, 10.0),
            },
        }
    }

    fn eval(&self, x: f64) -> f64 {
        match *self {
            Leaf::Sin { a, b } => (a * x + b).sin(),
            Leaf::Gauss { a, b } => (-(a * (x - b)).powi(2)).exp(),
            Leaf::Tanh { a, b } => (a * (x - b)).tanh(),
            Leaf::Cubic { c } => c[0] + x * (c[1] + x * (c[2] + x * c[3])),
            Leaf::Abs { b } => (x - b).abs(),
            Leaf::Saw { a } => (a * x).rem_euclid(1.0),
            Leaf::Steps { a } => (a * x).floor() / a,
            Leaf::SqrtAbs => x.abs().sqrt(),
            Leaf::Log1pAbs => x.abs().ln_1p(),
            Leaf::Lorentz { a } => 1.0 / (1.0 + a * x * x),
            Leaf::Chirp { a } => (a * x * x).sin(),
        }
    }
}

/// A random composition: sums, products and compositions of leaves.
#[derive(Clone, Debug)]
pub enum Expr {
    Leaf(Leaf),
    Add(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Compose(Box<Expr>, Box<Expr>),
}

impl Expr {
    pub fn random(rng: &mut Rng, depth: u32) -> Self {
        if depth == 0 || rng.chance(0.3) {
            return Expr::Leaf(Leaf::random(rng));
        }
        let a = Box::new(Expr::random(rng, depth - 1));
        let b = Box::new(Expr::random(rng, depth - 1));
        match rng.index(3) {
            0 => Expr::Add(a, b),
            1 => Expr::Mul(a, b),
            _ => Expr::Compose(a, b),
        }
    }

    pub fn eval(&self, x: f64) -> f64 {
        match self {
            Expr::Leaf(leaf) => leaf.eval(x),
            Expr::Add(a, b) => a.eval(x) + b.eval(x),
            Expr::Mul(a, b) => a.eval(x) * b.eval(x),
            // Squash the inner value so a composition stays bounded.
            Expr::Compose(outer, inner) => outer.eval(inner.eval(x).tanh()),
        }
    }
}

/// One random polyline in data space over [0, 10]², the visible part kept
/// inside [`MARGIN`, 10 - `MARGIN`].
#[derive(Debug)]
pub struct Curve {
    pub xs: Vec<f64>,
    pub ys: Vec<f64>,
    /// What the curve is, for a failure message.
    pub what: String,
}

const MARGIN: f64 = 0.8;

/// `values` rescaled onto [lo, hi], NaN where the source is not finite.
fn rescale(values: &[f64], lo: f64, hi: f64) -> Vec<f64> {
    let finite = values.iter().copied().filter(|v| v.is_finite());
    let min = finite.clone().fold(f64::INFINITY, f64::min);
    let max = finite.fold(f64::NEG_INFINITY, f64::max);
    let span = (max - min).max(1e-9);
    values
        .iter()
        .map(|v| {
            if v.is_finite() {
                lo + (v - min) / span * (hi - lo)
            } else {
                f64::NAN
            }
        })
        .collect()
}

impl Curve {
    /// The curve of `seed`: 6 to 160 samples of `y = f(x)` or of a
    /// parametric `(f(t), g(t))`, uneven in the parameter, with one NaN gap
    /// one time in six. `lead_points` off-screen zig-zag points (above and
    /// below the view, about 1.6e5 px each at 1x) come first, so the visible
    /// part starts that far along the arc.
    pub fn random(seed: u64, lead_points: usize) -> Self {
        let mut rng = Rng::new(seed);
        let f = Expr::random(&mut rng, 3);
        let parametric = rng.chance(0.3);
        let g = Expr::random(&mut rng, 2);
        let n = 6 + rng.index(155);
        let mut ts: Vec<f64> = (0..n)
            .map(|i| {
                let even = i as f64 / (n - 1) as f64;
                let jitter = rng.range(-0.45, 0.45) / (n - 1) as f64;
                (even + jitter).clamp(0.0, 1.0)
            })
            .collect();
        ts.sort_by(f64::total_cmp);
        let u = |t: f64| t * 2.0 - 1.0;
        let raw_y: Vec<f64> = ts.iter().map(|&t| f.eval(u(t))).collect();
        let raw_x: Vec<f64> = if parametric {
            ts.iter().map(|&t| g.eval(u(t)) + 0.5 * u(t)).collect()
        } else {
            ts.clone()
        };
        let (x_lo, x_hi) = {
            let a = rng.range(MARGIN, 4.0);
            (a, rng.range(a + 2.0, 10.0 - MARGIN))
        };
        let (y_lo, y_hi) = {
            let a = rng.range(MARGIN, 4.0);
            (a, rng.range(a + 1.0, 10.0 - MARGIN))
        };
        let mut xs = rescale(&raw_x, x_lo, x_hi);
        let mut ys = rescale(&raw_y, y_lo, y_hi);
        let gap = rng.chance(1.0 / 6.0) && n > 8;
        if gap {
            let at = 2 + rng.index(n - 4);
            ys[at] = f64::NAN;
        }
        if lead_points > 0 {
            let first_y = ys[0];
            let mut lead_x: Vec<f64> = vec![-1.0; lead_points];
            let mut lead_y: Vec<f64> = (0..lead_points)
                .map(|i| if i % 2 == 0 { 1.0e4 } else { -1.0e4 })
                .collect();
            // Re-enter the view across the left edge at the first sample's
            // height.
            lead_x.push(-0.5);
            lead_y.push(first_y);
            lead_x.append(&mut xs);
            lead_y.append(&mut ys);
            xs = lead_x;
            ys = lead_y;
        }
        let what = format!(
            "seed {seed}: {n} samples, {}, lead {lead_points}{}; y = {f:?}{}",
            if parametric { "parametric" } else { "graph" },
            if gap { ", one NaN gap" } else { "" },
            if parametric {
                format!("; x = {g:?}")
            } else {
                String::new()
            },
        );
        Self { xs, ys, what }
    }
}
