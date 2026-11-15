//! Small deterministic random generator, the same as in the mockup.

/// Seed of the first series. Every further series adds its running number.
pub const DEMO_SEED: u32 = 7;

const MULTIPLIER: u64 = 9301;
const INCREMENT: u64 = 49297;
const MODULUS: u64 = 233_280;

/// Linear congruential generator of `docs/mockup/nas-stats-mockup.html`.
///
/// The state stays an integer, so every platform yields the same numbers.
#[derive(Debug, Clone)]
pub(crate) struct DemoRng {
    state: u64,
}

impl DemoRng {
    pub(crate) fn new(seed: u32) -> Self {
        Self {
            state: u64::from(seed) % MODULUS,
        }
    }

    /// Next value in `0.0..1.0`.
    pub(crate) fn next_f64(&mut self) -> f64 {
        self.state = (self.state * MULTIPLIER + INCREMENT) % MODULUS;
        self.state as f64 / MODULUS as f64
    }

    /// Next value in `-0.5..0.5`.
    pub(crate) fn centered(&mut self) -> f64 {
        self.next_f64() - 0.5
    }
}

/// Hands out one generator per series, seeded in output order.
#[derive(Debug, Default)]
pub(crate) struct Seeds {
    next: u32,
}

impl Seeds {
    pub(crate) fn series(&mut self) -> DemoRng {
        let rng = DemoRng::new(DEMO_SEED + self.next);
        self.next += 1;
        rng
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_mockup() {
        // seed 7: (7 * 9301 + 49297) % 233280 = 114404
        let mut rng = DemoRng::new(DEMO_SEED);
        assert_eq!(rng.next_f64(), 114_404.0 / 233_280.0);
    }

    #[test]
    fn stays_in_range() {
        let mut rng = DemoRng::new(DEMO_SEED);
        for _ in 0..1000 {
            let v = rng.next_f64();
            assert!((0.0..1.0).contains(&v));
        }
    }

    #[test]
    fn seeds_count_up() {
        let mut seeds = Seeds::default();
        let mut first = seeds.series();
        let mut second = seeds.series();
        assert_eq!(first.next_f64(), DemoRng::new(7).next_f64());
        assert_eq!(second.next_f64(), DemoRng::new(8).next_f64());
    }
}
