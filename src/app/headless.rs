use std::{sync::mpsc, time::Duration};

use aida_ast::load_sb3;

use super::cli::RunOptions;
use super::worker::stop_requested;
use crate::runtime::{ObservableEvent, PacingMode, RuntimeSnapshot, SchedulerStatus, ScratchValue};
use crate::{graphics, program, runtime};

pub(crate) fn run_headless(options: RunOptions) -> Result<(), String> {
    let mut program = program::GeneratedProgram::new();
    if let Some(username) = options.username.as_ref() {
        program.set_username(username.clone());
    }
    if let Some(project_path) = options.project.as_ref() {
        let loaded = load_sb3(project_path, "resources/projects")
            .map_err(|error| format!("failed to load project: {error}"))?;
        if program::GeneratedProgram::uses_sound() {
            let sounds = runtime::SoundLibrary::load_project(&loaded.project, &loaded.assets_dir)
                .map_err(|error| format!("failed to load sounds: {error}"))?;
            program.set_sound_library(sounds);
        }
        for (target, geometry) in graphics::fence_geometries(&loaded.project, &loaded.assets_dir) {
            program.set_target_fence_geometry(target, geometry);
        }
    }
    let (log_sender, log_receiver) = if options.log_observables {
        let (sender, receiver) = mpsc::channel();
        (Some(sender), Some(receiver))
    } else {
        (None, None)
    };
    program.set_observable_event_sink(log_sender);
    let ticks = run_program(&mut program, options.pacing, options.max_ticks, None)?;
    report_tick_limit(&program, options.max_ticks, ticks);
    let snapshot = program.observable_snapshot(ticks);
    if let Some(receiver) = log_receiver {
        for event in receiver.try_iter() {
            print_observable_event(&event);
        }
    }
    if options.log_variables {
        print_variables(&snapshot);
    }
    assert_expected_variables(&snapshot, &options.expected_variables)
}

fn run_program(
    program: &mut program::GeneratedProgram,
    pacing: PacingMode,
    max_ticks: Option<usize>,
    stop: Option<&mpsc::Receiver<()>>,
) -> Result<usize, String> {
    let mut pacer = runtime::TickPacer::new(pacing, runtime::SystemPacingMechanism::new());
    let mut ticks = 0;
    loop {
        if max_ticks.is_some_and(|limit| ticks >= limit) || stop.is_some_and(stop_requested) {
            break;
        }
        let tick_started = pacer.begin_tick();
        let status = super::worker::advance_tick(program, pacing, tick_started, &pacer, None);
        ticks += 1;
        if program.presentation_requested() {
            // headless presentation consumes the same tick snapshot without creating a window.
            let _snapshot = program.observable_snapshot(ticks);
            let _pen_commands = program.take_pending_pen_commands();
            program.presentation_consumed();
        }
        if max_ticks == Some(ticks) && !program.is_complete() {
            break;
        }
        match status {
            SchedulerStatus::Progressed => pacer.finish_tick(tick_started),
            SchedulerStatus::WaitingUntil(deadline) if pacing == PacingMode::Turbo => {
                pacer.wait_for_scheduler(program.duration_until(deadline));
            }
            SchedulerStatus::WaitingUntil(_) => pacer.finish_tick(tick_started),
            SchedulerStatus::WaitingForSignal if pacing == PacingMode::Turbo => {
                std::thread::sleep(Duration::from_millis(8));
            }
            SchedulerStatus::WaitingForSignal => pacer.finish_tick(tick_started),
            SchedulerStatus::Complete => break,
        }
    }
    Ok(ticks)
}

fn report_tick_limit(program: &program::GeneratedProgram, max_ticks: Option<usize>, ticks: usize) {
    if max_ticks == Some(ticks) && !program.is_complete() {
        eprintln!("stopped after {ticks} scheduler ticks (tick limit reached)");
    }
}

fn print_observable_event(event: &ObservableEvent) {
    match event {
        ObservableEvent::Output { label, value } => println!("observable output {label}: {value}"),
        ObservableEvent::VariableChange {
            key,
            old_value,
            new_value,
        } => {
            println!("variable change {key}: {old_value} -> {new_value}");
        }
    }
}

fn print_variables(snapshot: &RuntimeSnapshot) {
    for variable in &snapshot.variables {
        println!("variable {} = {:?}", variable.key, variable.value);
    }
    for list in &snapshot.lists {
        println!("list {} = {:?}", list.key, list.items);
    }
}

pub(crate) fn assert_expected_variables(
    snapshot: &RuntimeSnapshot,
    expected: &[(String, ScratchValue)],
) -> Result<(), String> {
    for (key, expected_value) in expected {
        let actual = snapshot
            .variables
            .iter()
            .find(|variable| &variable.key == key)
            .map(|variable| &variable.value)
            .ok_or_else(|| format!("expected variable key `{key}` was not observed"))?;
        if actual != expected_value {
            return Err(format!(
                "variable `{key}`: expected {expected_value:?}, observed {actual:?}"
            ));
        }
    }
    Ok(())
}
