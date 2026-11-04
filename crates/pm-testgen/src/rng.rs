//! A fixed, documented pseudo-random generator.
//!
//! This is deliberately implemented here rather than taken from a crate.
//! Generated logs are test *ground truth*: if the underlying algorithm changed
//! between dependency versions, every golden file in the repository would
//! silently become wrong. SplitMix64 is small, well documented, and frozen —
//! seed `n` produces the same stream in ten years as it does today.
//!
//! This generator is for test-data construction only. It is not used anywhere
//! in the analysis engine, which is fully deterministic.
//!
//! Reference: Steele, Lea & Flood, "Fast splittable pseudorandom number
//! generators", OOPSLA 2014.

/// A deterministic SplitMix64 generator.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Creates a generator seeded with `seed`.
    ///
    /// The same seed always produces the same stream.
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Returns the next raw 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Returns a value in `0..n`, without modulo bias.
    ///
    /// Panics if `n` is zero.
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0, "below() requires a positive bound");
        // Rejection sampling: discard values in the final partial bucket so
        // every outcome is equally likely.
        let zone = u64::MAX - (u64::MAX % n);
        loop {
            let v = self.next_u64();
            if v < zone {
                return v % n;
            }
        }
    }

    /// Returns a float in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        // Use the top 53 bits, which is exactly the mantissa width of f64.
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Returns `true` with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    /// Shuffles `items` in place (Fisher-Yates).
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below((i + 1) as u64) as usize;
            items.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of a hand-rolled generator: a fixed seed must produce a
    /// fixed stream, permanently. If this test ever fails, every generated
    /// golden file in the repository has silently changed meaning.
    ///
    /// These values are **not** taken from this implementation's output. They
    /// were computed from the published SplitMix64 specification by an
    /// independent implementation, so this test verifies the algorithm rather
    /// than merely freezing whatever the code happens to do.
    #[test]
    fn matches_published_splitmix64_reference() {
        let mut r = Rng::new(42);
        let got: Vec<u64> = (0..4).map(|_| r.next_u64()).collect();
        assert_eq!(
            got,
            vec![
                0xBDD7_3226_2FEB_6E95,
                0x28EF_E333_B266_F103,
                0x4752_6757_130F_9F52,
                0x581C_E1FF_0E4A_E394,
            ]
        );
    }

    #[test]
    fn same_seed_same_sequence() {
        let a: Vec<u64> = (0..50).map(|_| Rng::new(7).next_u64()).collect();
        let b: Vec<u64> = (0..50).map(|_| Rng::new(7).next_u64()).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn below_stays_in_range() {
        let mut r = Rng::new(99);
        for _ in 0..1_000 {
            assert!(r.below(7) < 7);
        }
    }

    #[test]
    fn below_covers_every_outcome() {
        let mut r = Rng::new(3);
        let mut seen = [false; 5];
        for _ in 0..500 {
            seen[r.below(5) as usize] = true;
        }
        assert!(seen.iter().all(|&s| s), "some outcomes never occurred");
    }

    #[test]
    fn unit_stays_in_range() {
        let mut r = Rng::new(11);
        for _ in 0..1_000 {
            let v = r.unit();
            assert!((0.0..1.0).contains(&v));
        }
    }

    #[test]
    fn shuffle_preserves_contents() {
        let mut r = Rng::new(5);
        let mut items: Vec<u32> = (0..20).collect();
        r.shuffle(&mut items);
        items.sort_unstable();
        assert_eq!(items, (0..20).collect::<Vec<_>>());
    }

    #[test]
    fn shuffle_is_deterministic() {
        let run = || {
            let mut r = Rng::new(5);
            let mut items: Vec<u32> = (0..20).collect();
            r.shuffle(&mut items);
            items
        };
        assert_eq!(run(), run());
    }
}
