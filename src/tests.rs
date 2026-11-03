use std::{cell::Cell, rc::Rc};

use crate::{
    parse_run_options,
    program::GeneratedProgram,
    runtime::{Clock, PacingMechanism, PacingMode, SchedulerStatus, ScratchValue},
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
fn broadcast_fixture_restarts_the_receiver_and_waits_for_the_restarted_run() {
    let clock = TestClock(Rc::new(Cell::new(0)));
    let mut program = GeneratedProgram::new_with_clock(Box::new(clock.clone()));
    assert_eq!(program.advance(), SchedulerStatus::Progressed);
    assert_eq!(
        program
            .state()
            .get_by_id("`jEk@4|i[#Fk?(8x)AV.-my variable"),
        Some(&ScratchValue::Text("0".to_owned()))
    );
    assert_eq!(program.advance(), SchedulerStatus::Progressed);
    assert_eq!(
        program
            .state()
            .get_by_id("`jEk@4|i[#Fk?(8x)AV.-my variable"),
        Some(&ScratchValue::Text("0".to_owned()))
    );

    let deadline = loop {
        match program.advance() {
            SchedulerStatus::Progressed => {}
            SchedulerStatus::WaitingUntil(deadline) => break deadline,
            SchedulerStatus::Complete => panic!("broadcast fixture completed before message2"),
        }
    };
    clock.0.set(deadline);
    assert_eq!(program.advance(), SchedulerStatus::Progressed);
    assert_eq!(program.advance(), SchedulerStatus::Progressed);
    assert_eq!(
        program
            .state()
            .get_by_id("`jEk@4|i[#Fk?(8x)AV.-my variable"),
        Some(&ScratchValue::Number(1.0))
    );

    for _ in 0..20 {
        assert_eq!(program.advance(), SchedulerStatus::Progressed);
    }
    assert_eq!(
        program
            .state()
            .get_by_id("`jEk@4|i[#Fk?(8x)AV.-my variable"),
        Some(&ScratchValue::Number(96.0))
    );
    assert_eq!(program.active_script_count(), 1);
}

#[test]
#[ignore = "manual release-mode execution benchmark for the currently generated project"]
fn benchmark_generated_project_scheduler_throughput() {
    use std::time::Instant;

    const TICKS: usize = 1_000_000;
    const SAMPLES: usize = 7;
    let mut elapsed = Vec::with_capacity(SAMPLES);

    for _ in 0..SAMPLES {
        let clock = TestClock(Rc::new(Cell::new(0)));
        let mut program = GeneratedProgram::new_with_clock(Box::new(clock));
        let started = Instant::now();
        for _ in 0..TICKS {
            let _ = std::hint::black_box(program.advance());
        }
        elapsed.push(started.elapsed());

        let value = program
            .state()
            .get_by_id("`jEk@4|i[#Fk?(8x)AV.-my variable")
            .expect("addition-loop variable exists");
        assert_eq!(value, &ScratchValue::Number((TICKS - 1) as f64));
    }

    elapsed.sort_unstable();
    let median = elapsed[SAMPLES / 2];
    println!(
        "generated-project scheduler benchmark: ticks={TICKS}, samples={SAMPLES}, median={median:?}, ticks_per_second={:.0}",
        TICKS as f64 / median.as_secs_f64()
    );
}
