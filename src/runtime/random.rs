use std::{
    cell::RefCell,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{ScratchInput, ScratchValueView, scratch_to_number};

/// Small per-thread PRNG used by Scratch's random reporter.
pub struct ScratchRandom {
    state: u64,
}

impl ScratchRandom {
    pub fn seeded(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9e37_79b9_7f4a_7c15
            } else {
                seed
            },
        }
    }

    fn next_unit(&mut self) -> f64 {
        // xorshift64*: fast, deterministic, and sufficient for Scratch's
        // non-cryptographic random reporter.
        let mut value = self.state;
        value ^= value >> 12;
        value ^= value << 25;
        value ^= value >> 27;
        self.state = value;
        let bits = value.wrapping_mul(0x2545_f491_4f6c_dd1d);
        (bits >> 11) as f64 * (1.0 / ((1_u64 << 53) as f64))
    }

    #[allow(dead_code)]
    pub fn between(&mut self, from: f64, to: f64) -> f64 {
        self.between_with_integer_bounds(from, to, from.fract() == 0.0 && to.fract() == 0.0)
    }

    fn between_with_integer_bounds(&mut self, from: f64, to: f64, integer_bounds: bool) -> f64 {
        let (low, high) = if from <= to { (from, to) } else { (to, from) };
        if low == high {
            return low;
        }
        if integer_bounds && low.is_finite() && high.is_finite() {
            let range = high - low + 1.0;
            if range.is_finite() {
                return low + (self.next_unit() * range).floor();
            }
        }
        low + self.next_unit() * (high - low)
    }
}

impl Default for ScratchRandom {
    fn default() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(1);
        let time_seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or_default();
        let sequence = SEQUENCE.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed);
        Self::seeded(time_seed ^ sequence)
    }
}

thread_local! {
    static THREAD_RANDOM: RefCell<ScratchRandom> = RefCell::new(ScratchRandom::default());
}

/// Return a random Scratch number between the two inputs, inclusive when both
/// converted endpoints are integers.
pub fn scratch_random(from: impl ScratchInput, to: impl ScratchInput) -> f64 {
    let integer_bounds =
        is_scratch_integer(from.scratch_view()) && is_scratch_integer(to.scratch_view());
    let from = scratch_to_number(from);
    let to = scratch_to_number(to);
    THREAD_RANDOM.with(|random| {
        random
            .borrow_mut()
            .between_with_integer_bounds(from, to, integer_bounds)
    })
}

fn is_scratch_integer(value: ScratchValueView<'_>) -> bool {
    match value {
        ScratchValueView::Number(number) => number.is_nan() || number.fract() == 0.0,
        ScratchValueView::Boolean(_) => true,
        ScratchValueView::Text(text) => !text.contains('.'),
        ScratchValueView::Undefined => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_random_is_repeatable_and_bounds_are_inclusive() {
        let mut first = ScratchRandom::seeded(7);
        let mut second = ScratchRandom::seeded(7);
        for _ in 0..100 {
            let value = first.between(10.0, 1.0);
            assert_eq!(value, second.between(10.0, 1.0));
            assert!((1.0..=10.0).contains(&value));
            assert_eq!(value.fract(), 0.0);
        }
    }

    #[test]
    fn random_reporter_converts_bounds_using_scratch_number_rules() {
        let value = scratch_random("2", 2.0);
        assert_eq!(value, 2.0);
    }

    #[test]
    fn rounded_random_stage_coordinates_stay_within_stage_bounds() {
        let mut random = ScratchRandom::seeded(19);
        for _ in 0..10_000 {
            let x = super::super::scratch_round(random.between(-240.5, 240.5));
            let y = super::super::scratch_round(random.between(-179.5, 180.5));
            assert!((-240.0..=240.0).contains(&x));
            assert!((-180.0..=180.0).contains(&y));
        }
    }
}
