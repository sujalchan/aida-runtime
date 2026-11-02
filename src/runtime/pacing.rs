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
}

impl<M: PacingMechanism> TickPacer<M> {
    pub fn new(mode: PacingMode, mechanism: M) -> Self {
        Self { mode, mechanism }
    }

    pub fn now(&self) -> Duration {
        self.mechanism.now()
    }

    pub fn finish_tick(&mut self, tick_started: Duration) {
        if let PacingMode::Paced(target) = self.mode {
            let elapsed = self.mechanism.now().saturating_sub(tick_started);
            if elapsed < target {
                self.mechanism.sleep(target - elapsed);
            }
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
        let started = pacer.now();
        view.work(std::time::Duration::from_millis(7));
        pacer.finish_tick(started);
        assert_eq!(pacer.now(), std::time::Duration::from_millis(20));
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
        let started = pacer.now();
        view.work(std::time::Duration::from_millis(50));
        pacer.finish_tick(started);
        assert_eq!(pacer.now(), std::time::Duration::from_millis(50));
        let next_tick = pacer.now();
        pacer.finish_tick(next_tick);
        assert_eq!(pacer.now(), std::time::Duration::from_millis(70));
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
            let started = pacer.now();
            view.work(std::time::Duration::from_millis(3));
            pacer.finish_tick(started);
            let after_tick = if mode == PacingMode::Turbo {
                std::time::Duration::from_millis(3)
            } else {
                std::time::Duration::from_millis(20)
            };
            assert_eq!(pacer.now(), after_tick);
            pacer.wait_for_scheduler(std::time::Duration::from_millis(25));
            assert_eq!(
                pacer.now(),
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
            let start = pacer.now();
            view.work(std::time::Duration::from_millis(1));
            pacer.finish_tick(start);
        }
        assert_eq!(view.sleeps.get(), 0);
    }
}
