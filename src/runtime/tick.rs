use std::time::Duration;

use super::{PacingMode, SchedulerStatus};

// allow one extra fair round so pen writes can share a presentation without running away.
const MAX_PEN_BATCH_ROUNDS_PER_PRESENT: usize = 2;

/// run fair scheduler rounds until the frame work budget or an unbatchable redraw is reached.
pub fn run_tick(
    mode: PacingMode,
    started: Duration,
    budget: Duration,
    mut now: impl FnMut() -> Duration,
    mut redraw_requested: impl FnMut() -> bool,
    mut pen_marks_pending: impl FnMut() -> bool,
    mut advance_round: impl FnMut() -> SchedulerStatus,
) -> SchedulerStatus {
    let mut last_status = SchedulerStatus::Progressed;
    let mut pen_batch_rounds = 0;
    loop {
        if now().saturating_sub(started) >= budget {
            return last_status;
        }
        last_status = advance_round();
        match last_status {
            SchedulerStatus::Progressed => {}
            SchedulerStatus::WaitingUntil(_)
            | SchedulerStatus::WaitingForSignal
            | SchedulerStatus::Complete => return last_status,
        }
        // present pen-only batches after two fair rounds, while other redraws keep their boundary.
        if mode != PacingMode::Turbo && redraw_requested() {
            if !pen_marks_pending() {
                return last_status;
            }
            pen_batch_rounds += 1;
            if pen_batch_rounds >= MAX_PEN_BATCH_ROUNDS_PER_PRESENT {
                return last_status;
            }
        } else {
            pen_batch_rounds = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc, time::Duration};

    use super::run_tick;
    use crate::runtime::{PacingMode, SchedulerStatus};

    #[test]
    fn normal_mode_stops_after_a_redraw_round() {
        let steps = Cell::new(0);
        let mode = PacingMode::Paced(Duration::from_millis(20));
        let status = run_tick(
            mode,
            Duration::ZERO,
            Duration::from_millis(15),
            || Duration::from_millis(1),
            || steps.get() >= 1,
            || false,
            || {
                steps.set(steps.get() + 1);
                SchedulerStatus::Progressed
            },
        );
        assert_eq!(status, SchedulerStatus::Progressed);
        assert_eq!(steps.get(), 1);
    }

    #[test]
    fn turbo_ignores_redraw_until_work_budget_expires() {
        let time = Rc::new(Cell::new(Duration::ZERO));
        let step_time = Rc::clone(&time);
        let steps = Cell::new(0);
        let status = run_tick(
            PacingMode::Turbo,
            Duration::ZERO,
            Duration::from_millis(15),
            || time.get(),
            || true,
            || false,
            || {
                steps.set(steps.get() + 1);
                step_time.set(step_time.get() + Duration::from_millis(5));
                SchedulerStatus::Progressed
            },
        );
        assert_eq!(status, SchedulerStatus::Progressed);
        assert_eq!(steps.get(), 3);
    }

    #[test]
    fn paced_mode_repeats_fair_rounds_until_the_work_budget_without_redraws() {
        let time = Rc::new(Cell::new(Duration::ZERO));
        let step_time = Rc::clone(&time);
        let rounds = Cell::new(0);
        let status = run_tick(
            PacingMode::Paced(Duration::from_millis(20)),
            Duration::ZERO,
            Duration::from_millis(15),
            || time.get(),
            || false,
            || false,
            || {
                rounds.set(rounds.get() + 1);
                step_time.set(time.get() + Duration::from_millis(5));
                SchedulerStatus::Progressed
            },
        );
        assert_eq!(status, SchedulerStatus::Progressed);
        assert_eq!(rounds.get(), 3);
    }

    #[test]
    fn pending_pen_marks_allow_one_extra_ordered_round_before_presentation() {
        let time = Rc::new(Cell::new(Duration::ZERO));
        let step_time = Rc::clone(&time);
        let rounds = Cell::new(0);
        let status = run_tick(
            PacingMode::Paced(Duration::from_millis(20)),
            Duration::ZERO,
            Duration::from_millis(15),
            || time.get(),
            || true,
            || true,
            || {
                rounds.set(rounds.get() + 1);
                step_time.set(time.get() + Duration::from_millis(5));
                SchedulerStatus::Progressed
            },
        );
        assert_eq!(status, SchedulerStatus::Progressed);
        assert_eq!(rounds.get(), 2);
    }

    #[test]
    fn sleeping_scripts_end_the_tick_without_early_resume() {
        let calls = Cell::new(0);
        let status = run_tick(
            PacingMode::Paced(Duration::from_millis(20)),
            Duration::ZERO,
            Duration::from_millis(15),
            || Duration::from_millis(1),
            || false,
            || false,
            || {
                calls.set(calls.get() + 1);
                SchedulerStatus::WaitingUntil(100)
            },
        );
        assert_eq!(status, SchedulerStatus::WaitingUntil(100));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn a_warp_round_can_request_redraw_but_finishes_to_its_safety_yield() {
        let time = Rc::new(Cell::new(Duration::ZERO));
        let worker_time = Rc::clone(&time);
        let steps = Cell::new(0);
        let status = run_tick(
            PacingMode::Paced(Duration::from_millis(20)),
            Duration::ZERO,
            Duration::from_millis(15),
            || time.get(),
            || steps.get() > 0,
            || false,
            || {
                // a generated warp continuation only returns at its safety yield.
                worker_time.set(Duration::from_millis(501));
                steps.set(steps.get() + 1);
                SchedulerStatus::Progressed
            },
        );
        assert_eq!(status, SchedulerStatus::Progressed);
        assert_eq!(steps.get(), 1);
    }

    #[test]
    fn headless_presentation_consumes_a_visible_state_snapshot() {
        let mut state = crate::runtime::ScratchState::new(Vec::new());
        state.register_target(0, "Sprite1", false);
        state.begin_tick();
        let requested = Cell::new(false);
        let status = run_tick(
            PacingMode::Paced(Duration::from_millis(20)),
            Duration::ZERO,
            Duration::from_millis(15),
            || Duration::from_millis(1),
            || requested.get(),
            || false,
            || {
                state.set_xy(24.0, -9.0);
                state.request_redraw();
                requested.set(state.redraw_requested());
                SchedulerStatus::Progressed
            },
        );
        assert_eq!(status, SchedulerStatus::Progressed);
        let snapshot = state.observable_snapshot(1);
        assert_eq!((snapshot.layers[0].x, snapshot.layers[0].y), (24.0, -9.0));
        state.presentation_consumed();
        assert!(!state.redraw_requested());
    }
}
