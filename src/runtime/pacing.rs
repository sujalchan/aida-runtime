use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacingMode {
    Paced(Duration),
    Turbo,
}

impl PacingMode {
    pub const DEFAULT_FPS: f64 = 30.0;

    pub fn from_fps(fps: f64) -> Result<Self, String> {
        if !fps.is_finite() || fps <= 0.0 {
            return Err("FPS must be a positive finite number".to_owned());
        }
        let seconds = 1.0 / fps;
        let tick = Duration::try_from_secs_f64(seconds)
            .map_err(|_| "FPS is outside the supported timing range".to_owned())?;
        Ok(Self::Paced(tick))
    }
}

pub trait PacingMechanism {
    fn now(&self) -> Duration;
    fn sleep(&mut self, duration: Duration);
}

pub struct SystemPacingMechanism {
    started: Instant,
}

impl SystemPacingMechanism {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl PacingMechanism for SystemPacingMechanism {
    fn now(&self) -> Duration {
        self.started.elapsed()
    }

    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

pub struct TickPacer<M> {
    mode: PacingMode,
    mechanism: M,
    next_deadline: Option<Duration>,
}

impl<M: PacingMechanism> TickPacer<M> {
    pub fn new(mode: PacingMode, mechanism: M) -> Self {
        Self {
            mode,
            mechanism,
            next_deadline: None,
        }
    }

    /// start timing the work budget for this scheduler tick.
    pub fn begin_tick(&self) -> Option<Duration> {
        Some(self.mechanism.now())
    }

    pub fn now(&self) -> Duration {
        self.mechanism.now()
    }

    pub fn work_budget(&self) -> Duration {
        let interval = match self.mode {
            PacingMode::Paced(interval) => interval,
            PacingMode::Turbo => Duration::from_secs_f64(1.0 / PacingMode::DEFAULT_FPS),
        };
        interval.mul_f64(0.75)
    }

    pub fn finish_tick(&mut self, tick_started: Option<Duration>) {
        if let (PacingMode::Paced(target), Some(tick_started)) = (self.mode, tick_started) {
            let now = self.mechanism.now();
            let deadline = *self
                .next_deadline
                .get_or_insert_with(|| tick_started.saturating_add(target));
            if now.saturating_sub(tick_started) >= target {
                // skip missed frames instead of trying to catch up.
                self.next_deadline = Some(now.saturating_add(target));
                return;
            }

            if now < deadline {
                self.mechanism.sleep(deadline - now);
            }

            let after_sleep = self.mechanism.now();
            let next_deadline = deadline.saturating_add(target);
            self.next_deadline = Some(if after_sleep.saturating_sub(next_deadline) >= target {
                // recover from a long host stall without keeping a backlog.
                after_sleep.saturating_add(target)
            } else {
                next_deadline
            });
        }
    }

    /// wait for a scheduler deadline in both normal and turbo modes.
    pub fn wait_for_scheduler(&mut self, duration: Duration) {
        if !duration.is_zero() {
            self.mechanism.sleep(duration);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use super::{PacingMechanism, PacingMode, TickPacer};

    #[derive(Clone, Default)]
    struct MockMechanism {
        now: Rc<Cell<std::time::Duration>>,
        sleeps: Rc<Cell<usize>>,
    }

    impl MockMechanism {
        fn work(&self, duration: std::time::Duration) {
            self.now.set(self.now.get() + duration);
        }
    }

    impl PacingMechanism for MockMechanism {
        fn now(&self) -> std::time::Duration {
            self.now.get()
        }

        fn sleep(&mut self, duration: std::time::Duration) {
            self.sleeps.set(self.sleeps.get() + 1);
            self.work(duration);
        }
    }

    #[test]
    fn defaults_to_thirty_hertz_and_accepts_sixty_hertz() {
        assert_eq!(
            PacingMode::from_fps(PacingMode::DEFAULT_FPS).unwrap(),
            PacingMode::Paced(std::time::Duration::from_nanos(33_333_333))
        );
        assert_eq!(
            PacingMode::from_fps(60.0).unwrap(),
            PacingMode::Paced(std::time::Duration::from_nanos(16_666_667))
        );
    }

    #[test]
    fn elapsed_tick_work_is_subtracted_from_the_frame_delay() {
        let mechanism = MockMechanism::default();
        let view = mechanism.clone();
        let mut pacer = TickPacer::new(
            PacingMode::Paced(std::time::Duration::from_millis(20)),
            mechanism,
        );
        let started = pacer.begin_tick();
        view.work(std::time::Duration::from_millis(7));
        pacer.finish_tick(started);
        assert_eq!(view.now.get(), std::time::Duration::from_millis(20));
        assert_eq!(view.sleeps.get(), 1);
    }

    #[test]
    fn slow_ticks_do_not_build_a_catch_up_backlog() {
        let mechanism = MockMechanism::default();
        let view = mechanism.clone();
        let mut pacer = TickPacer::new(
            PacingMode::Paced(std::time::Duration::from_millis(20)),
            mechanism,
        );
        let started = pacer.begin_tick();
        view.work(std::time::Duration::from_millis(50));
        pacer.finish_tick(started);
        assert_eq!(
            pacer.begin_tick(),
            Some(std::time::Duration::from_millis(50))
        );
        let next_tick = pacer.begin_tick();
        pacer.finish_tick(next_tick);
        assert_eq!(
            pacer.begin_tick(),
            Some(std::time::Duration::from_millis(70))
        );
    }

    #[test]
    fn turbo_skips_frame_delay_but_waits_for_scratch_deadlines() {
        for mode in [
            PacingMode::Turbo,
            PacingMode::Paced(std::time::Duration::from_millis(20)),
        ] {
            let mechanism = MockMechanism::default();
            let view = mechanism.clone();
            let mut pacer = TickPacer::new(mode, mechanism);
            let started = pacer.begin_tick();
            view.work(std::time::Duration::from_millis(3));
            pacer.finish_tick(started);
            let after_tick = if mode == PacingMode::Turbo {
                std::time::Duration::from_millis(3)
            } else {
                std::time::Duration::from_millis(20)
            };
            assert_eq!(view.now.get(), after_tick);
            pacer.wait_for_scheduler(std::time::Duration::from_millis(25));
            assert_eq!(
                view.now.get(),
                after_tick + std::time::Duration::from_millis(25)
            );
        }
    }

    #[test]
    fn repeated_turbo_ticks_add_no_pacing_sleeps() {
        let mechanism = MockMechanism::default();
        let view = mechanism.clone();
        let mut pacer = TickPacer::new(PacingMode::Turbo, mechanism);
        for _ in 0..4 {
            let start = pacer.begin_tick();
            view.work(std::time::Duration::from_millis(1));
            pacer.finish_tick(start);
        }
        assert_eq!(view.sleeps.get(), 0);
    }

    #[test]
    fn pacing_compensates_for_small_sleep_overshoot_without_catch_up_bursts() {
        #[derive(Clone, Default)]
        struct OversleepMechanism {
            now: Rc<Cell<std::time::Duration>>,
            sleeps: Rc<Cell<usize>>,
        }

        impl PacingMechanism for OversleepMechanism {
            fn now(&self) -> std::time::Duration {
                self.now.get()
            }

            fn sleep(&mut self, duration: std::time::Duration) {
                self.sleeps.set(self.sleeps.get() + 1);
                self.now
                    .set(self.now.get() + duration + std::time::Duration::from_millis(2));
            }
        }

        let mechanism = OversleepMechanism::default();
        let view = mechanism.clone();
        let mut pacer = TickPacer::new(
            PacingMode::Paced(std::time::Duration::from_millis(20)),
            mechanism,
        );
        for _ in 0..5 {
            let started = pacer.begin_tick();
            pacer.finish_tick(started);
        }
        assert_eq!(view.now.get(), std::time::Duration::from_millis(102));
        assert_eq!(view.sleeps.get(), 5);
    }

    #[test]
    fn turbo_does_not_read_the_frame_clock() {
        let mechanism = MockMechanism::default();
        let view = mechanism.clone();
        let mut pacer = TickPacer::new(PacingMode::Turbo, mechanism);
        for _ in 0..5 {
            let started = pacer.begin_tick();
            pacer.finish_tick(started);
        }
        assert_eq!(view.sleeps.get(), 0);
        assert_eq!(view.now.get(), std::time::Duration::ZERO);
    }

    #[test]
    #[ignore = "manual wall-clock pacing measurement"]
    fn measure_system_pacing_jitter() {
        let target = PacingMode::from_fps(30.0).unwrap();
        let mut pacer = TickPacer::new(target, super::SystemPacingMechanism::new());
        let mut previous = None;
        let mut intervals = Vec::with_capacity(60);
        for _ in 0..=60 {
            let started = pacer.begin_tick().unwrap();
            if let Some(previous) = previous {
                intervals.push(started.saturating_sub(previous).as_nanos() as f64);
            }
            previous = Some(started);
            pacer.finish_tick(Some(started));
        }

        intervals.sort_by(f64::total_cmp);
        let mean = intervals.iter().sum::<f64>() / intervals.len() as f64;
        let variance = intervals
            .iter()
            .map(|interval| (interval - mean).powi(2))
            .sum::<f64>()
            / intervals.len() as f64;
        println!(
            "30 fps tick intervals: mean={:.3} ms, p50={:.3} ms, min={:.3} ms, max={:.3} ms, sd={:.3} ms",
            mean / 1_000_000.0,
            intervals[intervals.len() / 2] / 1_000_000.0,
            intervals[0] / 1_000_000.0,
            intervals[intervals.len() - 1] / 1_000_000.0,
            variance.sqrt() / 1_000_000.0,
        );
    }
}
