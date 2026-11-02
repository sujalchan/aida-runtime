mod program;
mod runtime;

#[derive(Debug)]
struct RunOptions {
    pacing: runtime::PacingMode,
    max_ticks: Option<usize>,
}

fn main() {
    let options = match parse_run_options(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!(
                "error: {message}\nusage: aida-runtime [--fps <number> | --turbo] [--max-ticks <count>]"
            );
            std::process::exit(2);
        }
    };
    let mut program = program::GeneratedProgram::new();
    let mut pacer = runtime::TickPacer::new(options.pacing, runtime::SystemPacingMechanism::new());
    let mut ticks = 0;
    loop {
        if options.max_ticks.is_some_and(|limit| ticks >= limit) {
            break;
        }
        let tick_started = pacer.begin_tick();
        let status = program.advance();
        ticks += 1;
        if options.max_ticks == Some(ticks) && !program.is_complete() {
            break;
        }
        match status {
            runtime::SchedulerStatus::Progressed => pacer.finish_tick(tick_started),
            runtime::SchedulerStatus::WaitingUntil(deadline) => {
                pacer.reset_schedule();
                let delay = program.duration_until(deadline);
                pacer.wait_for_scheduler(delay);
            }
            runtime::SchedulerStatus::Complete => break,
        }
    }
    if options.max_ticks == Some(ticks) && !program.is_complete() {
        eprintln!("stopped after {ticks} scheduler ticks (tick limit reached)");
    }
}

fn parse_run_options(args: impl IntoIterator<Item = String>) -> Result<RunOptions, String> {
    let mut fps = None;
    let mut turbo = false;
    let mut max_ticks = None;
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--fps" => {
                if turbo {
                    return Err("--fps and --turbo cannot be used together".to_owned());
                }
                if fps.is_some() {
                    return Err("--fps may only be specified once".to_owned());
                }
                let value = args.next().ok_or("--fps requires a number")?;
                let value = value.parse::<f64>().map_err(|_| {
                    format!("invalid FPS value {value:?}; expected a positive number")
                })?;
                fps = Some(runtime::PacingMode::from_fps(value)?);
            }
            "--turbo" => {
                if fps.is_some() {
                    return Err("--fps and --turbo cannot be used together".to_owned());
                }
                if turbo {
                    return Err("--turbo may only be specified once".to_owned());
                }
                turbo = true;
            }
            "--max-ticks" => {
                if max_ticks.is_some() {
                    return Err("--max-ticks may only be specified once".to_owned());
                }
                let value = args
                    .next()
                    .ok_or("--max-ticks requires a positive integer")?;
                let value = value.parse::<usize>().map_err(|_| {
                    format!("invalid tick limit {value:?}; expected a positive integer")
                })?;
                if value == 0 {
                    return Err("--max-ticks must be greater than zero".to_owned());
                }
                max_ticks = Some(value);
            }
            _ => return Err(format!("unknown option {argument:?}")),
        }
    }

    let pacing = if turbo {
        runtime::PacingMode::Turbo
    } else if let Some(fps) = fps {
        fps
    } else {
        runtime::PacingMode::from_fps(runtime::PacingMode::DEFAULT_FPS)?
    };
    Ok(RunOptions { pacing, max_ticks })
}

#[cfg(test)]
mod generated_program_tests {
    use std::{cell::Cell, rc::Rc};

    use crate::{
        program::GeneratedProgram,
        runtime::{Clock, PacingMechanism, PacingMode, SchedulerStatus, ScratchValue, TickPacer},
    };

    use super::parse_run_options;

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
        let error = parse_run_options(["--fps".to_owned(), "60".to_owned(), "--turbo".to_owned()])
            .unwrap_err();
        assert!(error.contains("cannot be used together"));
    }

    #[test]
    fn cli_selects_default_sixty_hertz_and_turbo_modes() {
        assert_eq!(
            parse_run_options(std::iter::empty()).unwrap().pacing,
            crate::runtime::PacingMode::Paced(std::time::Duration::from_nanos(33_333_333))
        );
        assert_eq!(
            parse_run_options(["--fps".to_owned(), "60".to_owned()])
                .unwrap()
                .pacing,
            crate::runtime::PacingMode::Paced(std::time::Duration::from_nanos(16_666_667))
        );
        assert_eq!(
            parse_run_options(["--turbo".to_owned()]).unwrap().pacing,
            crate::runtime::PacingMode::Turbo
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
    fn generated_list_fixture_runs_through_scheduler_and_preserves_scopes() {
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
            program.state().list_by_id("stage-list").unwrap().contents(),
            "BX"
        );
        assert_eq!(
            program.state().list_by_id("sprite-list").unwrap().length(),
            4
        );
        assert_eq!(
            program
                .state()
                .list_by_name("Stage", "items")
                .unwrap()
                .contents(),
            "BX"
        );
        assert_eq!(
            program
                .state()
                .list_by_name("Sprite1", "items")
                .unwrap()
                .length(),
            4
        );
        assert!(program.state().list_visible(1));
        assert!(!program.state().list_visible(0));
        assert_eq!(
            program.state().get_by_id("observed-id"),
            Some(&ScratchValue::Boolean(true))
        );
        assert_eq!(
            program.state().get_by_id("contents-id"),
            Some(&ScratchValue::Text("BX".into()))
        );
        assert_eq!(
            program.state().get_by_id("length-id"),
            Some(&ScratchValue::Number(2.0))
        );
        assert_eq!(
            program.state().get_by_id("item-id"),
            Some(&ScratchValue::Text("seed".into()))
        );
        assert_eq!(
            program.state().get_by_id("number-id"),
            Some(&ScratchValue::Number(1.0))
        );
    }
}
