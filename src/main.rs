#[rustfmt::skip]
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
mod tests;
