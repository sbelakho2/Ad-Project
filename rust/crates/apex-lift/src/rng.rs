//! Deterministic SplitMix64: bootstrap resampling and Thompson sampling must
//! be reproducible from the ledger seed, so every draw is seeded, never
//! ambient.

/// SplitMix64 generator with a fixed, recordable seed.
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1): 53 random bits scaled exactly.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform index in `0..bound` (bound > 0).
    pub fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }

    /// Standard normal draw via the Box-Muller transform.
    pub fn next_normal(&mut self) -> f64 {
        let u1 = self.next_f64().max(f64::MIN_POSITIVE);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn splitmix_is_deterministic_and_in_range() {
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut rng = SplitMix64::new(7);
        for _ in 0..1000 {
            let value = rng.next_f64();
            assert!((0.0..1.0).contains(&value), "{value}");
            assert!(rng.below(7) < 7);
        }
    }

    #[test]
    fn normal_draws_have_reasonable_moments() {
        let mut rng = SplitMix64::new(2026);
        let draws: Vec<f64> = (0..20_000).map(|_| rng.next_normal()).collect();
        let mean = draws.iter().sum::<f64>() / draws.len() as f64;
        let var = draws.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / draws.len() as f64;
        assert!(mean.abs() < 0.05, "{mean}");
        assert!((var - 1.0).abs() < 0.05, "{var}");
    }
}
