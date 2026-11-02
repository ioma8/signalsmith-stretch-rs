//! Deterministic PCG32 phase randomization.

use crate::sample::{Sample, sf};

pub(super) struct Pcg32 {
    state: u64,
    increment: u64,
}

impl Pcg32 {
    pub(super) fn new(seed: u64) -> Self {
        let mut rng = Self {
            state: 0,
            increment: (seed << 1) | 1,
        };
        rng.next_u32();
        rng.state = rng.state.wrapping_add(seed);
        rng.next_u32();
        rng
    }

    fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(self.increment);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    fn unit<S: Sample>(&mut self) -> S {
        sf(f64::from(self.next_u32()) / 4_294_967_296.0)
    }

    pub(super) fn range<S: Sample>(&mut self, low: S, high: S) -> S {
        low + (high - low) * self.unit()
    }
}
