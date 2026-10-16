//! Deterministic pseudo-random numbers and the distributions the engine
//! samples from.
//!
//! The generator is xoshiro256** seeded through SplitMix64 (Blackman and
//! Vigna). Every stochastic decision in the engine (Thompson draws, holdout
//! tie-breaks, simulation) takes an explicit [`Rng`], so a run is replayable
//! from its seed and a test can pin it.
//!
//! * Normal deviates use the Box–Muller transform.
//! * Gamma deviates use Marsaglia and Tsang (2000), with the `shape < 1`
//!   boost `G(a) = G(a + 1) · U^(1/a)`.
//! * Beta deviates are `X / (X + Y)` for independent gammas.

/// A seeded xoshiro256** generator.
#[derive(Debug, Clone)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    /// A generator whose whole state is derived from `seed` by SplitMix64.
    pub fn seeded(seed: u64) -> Self {
        let mut sm = seed;
        let mut next = || {
            sm = sm.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = sm;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        let s = [next(), next(), next(), next()];
        Self { s }
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let [s0, s1, s2, s3] = self.s;
        let result = s1.wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s1 << 17;
        let n2 = s2 ^ s0;
        let n3 = s3 ^ s1;
        let n1 = s1 ^ n2;
        let n0 = s0 ^ n3;
        self.s = [n0, n1, n2 ^ t, n3.rotate_left(45)];
        result
    }

    /// Uniform on the open interval (0, 1): never exactly 0 or 1, so `ln`
    /// and powers of the draw stay finite.
    pub fn uniform(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    /// A uniform integer in `0..n` (Lemire's multiply-shift with rejection);
    /// `n == 0` yields 0.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        loop {
            let x = self.next_u64();
            let m = u128::from(x) * u128::from(n);
            let low = m as u64;
            if low >= n.wrapping_neg() % n {
                return (m >> 64) as u64;
            }
        }
    }

    /// A standard normal deviate (Box–Muller).
    pub fn normal(&mut self) -> f64 {
        let u1 = self.uniform();
        let u2 = self.uniform();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    /// A Gamma(`shape`, 1) deviate. A non-finite or non-positive shape is
    /// treated as a vanishingly small one rather than panicking.
    pub fn gamma(&mut self, shape: f64) -> f64 {
        let shape = if shape.is_finite() && shape > 1e-9 {
            shape
        } else {
            1e-9
        };
        if shape < 1.0 {
            let boost = self.uniform().powf(1.0 / shape);
            return self.gamma_ge_one(shape + 1.0) * boost;
        }
        self.gamma_ge_one(shape)
    }

    fn gamma_ge_one(&mut self, shape: f64) -> f64 {
        let d = shape - 1.0 / 3.0;
        let c = 1.0 / (9.0 * d).sqrt();
        loop {
            let x = self.normal();
            let v = 1.0 + c * x;
            if v <= 0.0 {
                continue;
            }
            let v = v * v * v;
            let u = self.uniform();
            if u < 1.0 - 0.0331 * x.powi(4) {
                return d * v;
            }
            if u.ln() < 0.5 * x * x + d * (1.0 - v + v.ln()) {
                return d * v;
            }
        }
    }

    /// A Beta(`a`, `b`) deviate.
    pub fn beta(&mut self, a: f64, b: f64) -> f64 {
        let x = self.gamma(a);
        let y = self.gamma(b);
        let total = x + y;
        if total > 0.0 {
            x / total
        } else {
            0.5
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_replays_the_same_stream() {
        let mut a = Rng::seeded(7);
        let mut b = Rng::seeded(7);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        assert_ne!(Rng::seeded(7).next_u64(), Rng::seeded(8).next_u64());
    }

    #[test]
    fn uniform_stays_strictly_inside_the_unit_interval() {
        let mut rng = Rng::seeded(1);
        let mut sum = 0.0;
        for _ in 0..20_000 {
            let u = rng.uniform();
            assert!(u > 0.0 && u < 1.0);
            sum += u;
        }
        assert!((sum / 20_000.0 - 0.5).abs() < 0.01);
    }

    #[test]
    fn below_is_bounded_and_roughly_uniform() {
        let mut rng = Rng::seeded(3);
        let mut counts = [0u32; 5];
        for _ in 0..50_000 {
            let k = rng.below(5) as usize;
            counts[k] += 1;
        }
        for c in counts {
            assert!((9_000..11_000).contains(&c), "{c}");
        }
        assert_eq!(rng.below(0), 0);
    }

    #[test]
    fn distributions_have_the_right_moments() {
        let mut rng = Rng::seeded(11);
        let n = 40_000;
        let (mut s, mut s2) = (0.0, 0.0);
        for _ in 0..n {
            let z = rng.normal();
            s += z;
            s2 += z * z;
        }
        assert!((s / n as f64).abs() < 0.03);
        assert!((s2 / n as f64 - 1.0).abs() < 0.05);

        for shape in [0.3, 1.0, 2.5, 9.0] {
            let mean: f64 = (0..n).map(|_| rng.gamma(shape)).sum::<f64>() / n as f64;
            assert!(
                (mean - shape).abs() < 0.05 * shape.max(1.0),
                "{shape} {mean}"
            );
        }
        let mean: f64 = (0..n).map(|_| rng.beta(2.0, 6.0)).sum::<f64>() / n as f64;
        assert!((mean - 0.25).abs() < 0.01, "{mean}");
    }

    #[test]
    fn hostile_shapes_do_not_panic() {
        let mut rng = Rng::seeded(5);
        for shape in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let g = rng.gamma(shape);
            assert!(g.is_finite() && g >= 0.0);
        }
        let b = rng.beta(0.0, 0.0);
        assert!((0.0..=1.0).contains(&b));
    }
}
