use std::{cell::Cell, rc::Rc};

use crate::{
    parse_run_options,
    program::GeneratedProgram,
    runtime::{Clock, PacingMechanism, PacingMode, SchedulerStatus, ScratchValue, TickPacer},
};

#[derive(Clone)]
struct TestClock(Rc<Cell<u64>>);

impl Clock for TestClock {
    fn now_millis(&self) -> u64 {
        self.0.get()
    }
}

impl PacingMechanism for TestClock {
    fn now(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.0.get())
    }

    fn sleep(&mut self, duration: std::time::Duration) {
        self.0
            .set(self.0.get().saturating_add(duration.as_millis() as u64));
    }
}

#[test]
fn cli_rejects_invalid_fps_and_conflicting_modes() {
    for value in ["0", "-1", "nan", "inf", "not-a-number"] {
        let result = parse_run_options(["--fps".to_owned(), value.to_owned()]);
        assert!(result.is_err(), "accepted fps {value}");
    }
    assert!(parse_run_options(["--fps".to_owned()]).is_err());
    let error =
        parse_run_options(["--fps".to_owned(), "60".to_owned(), "--turbo".to_owned()]).unwrap_err();
    assert!(error.contains("cannot be used together"));
}

#[test]
fn cli_selects_default_sixty_hertz_and_turbo_modes() {
    assert_eq!(
        parse_run_options(std::iter::empty()).unwrap().pacing,
        PacingMode::Paced(std::time::Duration::from_nanos(33_333_333))
    );
    assert_eq!(
        parse_run_options(["--fps".to_owned(), "60".to_owned()])
            .unwrap()
            .pacing,
        PacingMode::Paced(std::time::Duration::from_nanos(16_666_667))
    );
    assert_eq!(
        parse_run_options(["--turbo".to_owned()]).unwrap().pacing,
        PacingMode::Turbo
    );
}

#[test]
fn cli_accepts_a_positive_scheduler_tick_limit() {
    let options = parse_run_options([
        "--turbo".to_owned(),
        "--max-ticks".to_owned(),
        "100".to_owned(),
    ])
    .unwrap();
    assert_eq!(options.max_ticks, Some(100));
    assert!(parse_run_options(["--max-ticks".to_owned(), "0".to_owned()]).is_err());
}

#[test]
fn clone_with_variables_fixture_runs_and_updates_shared_stage_list() {
    let clock = TestClock(Rc::new(Cell::new(0)));
    let mut program = GeneratedProgram::new_with_clock(Box::new(clock.clone()));
    let mut pacer = TickPacer::new(PacingMode::Turbo, clock.clone());
    for _ in 0..100 {
        let tick_started = pacer.begin_tick();
        match program.advance() {
            SchedulerStatus::Progressed => pacer.finish_tick(tick_started),
            SchedulerStatus::WaitingUntil(deadline) => {
                pacer.reset_schedule();
                let delay = program.duration_until(deadline);
                pacer.wait_for_scheduler(delay);
            }
            SchedulerStatus::Complete => break,
        }
    }

    assert!(program.is_complete());
    assert_eq!(
        program
            .state()
            .list_by_id(":YULw@c(j{pVxkwY@MM$")
            .unwrap()
            .contents(),
        "9 4 11 12 7"
    );
    assert_eq!(
        program
            .state()
            .get_by_id("`jEk@4|i[#Fk?(8x)AV.-my variable"),
        Some(&ScratchValue::Number(7.0))
    );
    assert_eq!(program.state().clone_count(), 1);
}
