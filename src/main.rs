#[rustfmt::skip]
mod program;
#[allow(dead_code)]
mod graphics;
mod renderer;
mod runtime;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use aida_ast::load_sb3;
use runtime::{
    ObservableEvent, PacingMode, RuntimeInputEvent, RuntimeSnapshot, SchedulerStatus, ScratchValue,
};
use winit::event_loop::{ControlFlow, EventLoop};

#[derive(Debug, Clone)]
struct RunOptions {
    pacing: PacingMode,
    max_ticks: Option<usize>,
    graphical: bool,
    dev_mode: bool,
    test_mode: bool,
    hd_pen_mode: bool,
    project: Option<PathBuf>,
    log_observables: bool,
    log_variables: bool,
    expected_variables: Vec<(String, ScratchValue)>,
}

fn main() {
    let options = match parse_run_options(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!(
                "error: {message}\nusage: aida-runtime [--headless | --graphical --project <file.sb3>] [--test] [--dev] [--hd-pen] [--fps <number> | --turbo] [--max-ticks <count>] [--log-observables] [--log-variables] [--expect-variable <key>=<json-value>]"
            );
            std::process::exit(2);
        }
    };
    let result = if options.graphical {
        run_graphical(options)
    } else {
        run_headless(options)
    };
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run_headless(options: RunOptions) -> Result<(), String> {
    let mut program = program::GeneratedProgram::new();
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

fn run_graphical(options: RunOptions) -> Result<(), String> {
    let project_path = options
        .project
        .as_ref()
        .ok_or_else(|| "--graphical requires --project <file.sb3>".to_owned())?;
    let loaded = load_sb3(project_path, "resources/projects")
        .map_err(|error| format!("failed to load project: {error}"))?;
    let project = Arc::new(loaded.project);
    let window_title = project_window_title(project_path, options.hd_pen_mode);

    // winit requires its event loop and window creation to stay on the main
    // thread on macos and for cross-platform compatibility.
    let event_loop = EventLoop::<renderer::RendererEvent>::with_user_event()
        .build()
        .map_err(|error| format!("failed to create the window event loop: {error}"))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let latest = Arc::new(RwLock::new(Arc::new(RuntimeSnapshot::default())));
    let notification_pending = Arc::new(AtomicBool::new(false));
    let (observable_sender, observable_receiver) = if options.log_observables {
        let (sender, receiver) = mpsc::channel();
        (Some(sender), Some(receiver))
    } else {
        (None, None)
    };
    let (stop_sender, stop_receiver) = mpsc::channel();
    let (execution_start_sender, execution_start_receiver) = mpsc::channel();
    let (ack_sender, ack_receiver) = mpsc::channel();
    let (input_sender, input_receiver) = mpsc::channel();
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    let worker_latest = Arc::clone(&latest);
    let worker_notification_pending = Arc::clone(&notification_pending);
    let worker_options = options.clone();
    let worker_proxy = proxy.clone();
    let worker = thread::Builder::new()
        .name("aida-scratch-execution".to_owned())
        .spawn(move || {
            let mut program = program::GeneratedProgram::new();
            program.set_observable_event_sink(observable_sender);
            publish_snapshot(
                &program,
                0,
                &worker_latest,
                &worker_proxy,
                &worker_notification_pending,
            );
            let _ = ready_sender.send(());
            let result = run_worker(
                &mut program,
                worker_options.pacing,
                worker_options.max_ticks,
                &stop_receiver,
                &worker_latest,
                &worker_proxy,
                &worker_notification_pending,
                &execution_start_receiver,
                &ack_receiver,
                &input_receiver,
            );
            if let Err(error) = result {
                eprintln!("execution worker failed: {error}");
            }
            let _ = worker_proxy.send_event(renderer::RendererEvent::ExecutionFinished);
        })
        .map_err(|error| format!("failed to start execution worker: {error}"))?;
    ready_receiver
        .recv()
        .map_err(|error| format!("execution worker did not start: {error}"))?;

    let mut app = renderer::RendererApp::new(
        project,
        loaded.assets_dir,
        window_title,
        latest,
        notification_pending,
        observable_receiver,
        stop_sender.clone(),
        execution_start_sender.clone(),
        ack_sender,
        input_sender,
        options.expected_variables,
        options.dev_mode,
        options.test_mode,
        options.hd_pen_mode,
    );
    let loop_result = event_loop.run_app(&mut app);
    // Release the worker if the event loop exits before the first frame is presented.
    let _ = execution_start_sender.send(());
    let app_error = app.take_error();
    let _ = stop_sender.send(());
    let worker_result = worker
        .join()
        .map_err(|_| "execution worker panicked".to_owned());
    loop_result.map_err(|error| format!("window event loop failed: {error}"))?;
    worker_result?;
    if let Some(error) = app_error {
        return Err(error);
    }
    Ok(())
}

fn project_window_title(project_path: &std::path::Path, hd_pen_mode: bool) -> String {
    let project_name = project_path
        .file_stem()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "Scratch project".into());
    let prefix = if hd_pen_mode { "AIDA HD" } else { "AIDA" };
    format!("{prefix} - {project_name}")
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
        let status = advance_tick(program, pacing, tick_started, &pacer, None);
        ticks += 1;
        if program.presentation_requested() {
            // headless presentation consumes the same tick snapshot without creating a window.
            let _snapshot = program.observable_snapshot(ticks);
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
            SchedulerStatus::Complete => break,
        }
    }
    Ok(ticks)
}

fn run_worker(
    program: &mut program::GeneratedProgram,
    pacing: PacingMode,
    max_ticks: Option<usize>,
    stop: &mpsc::Receiver<()>,
    latest: &Arc<RwLock<Arc<RuntimeSnapshot>>>,
    proxy: &winit::event_loop::EventLoopProxy<renderer::RendererEvent>,
    notification_pending: &AtomicBool,
    execution_start: &mpsc::Receiver<()>,
    acknowledgements: &mpsc::Receiver<usize>,
    inputs: &mpsc::Receiver<RuntimeInputEvent>,
) -> Result<usize, String> {
    let _ = execution_start.recv();
    let mut pacer = runtime::TickPacer::new(pacing, runtime::SystemPacingMechanism::new());
    let mut ticks = 0;
    let mut stopped_externally = false;
    loop {
        if max_ticks.is_some_and(|limit| ticks >= limit) {
            break;
        }
        if stop_requested(stop) {
            stopped_externally = true;
            break;
        }
        let tick_started = pacer.begin_tick();
        let status = advance_tick(program, pacing, tick_started, &pacer, Some(inputs));
        ticks += 1;
        let presentation_requested = program.presentation_requested();
        if presentation_requested {
            publish_snapshot(program, ticks, latest, proxy, notification_pending);
        }
        if presentation_requested && pacing != PacingMode::Turbo {
            if !wait_for_renderer(acknowledgements, ticks, stop) {
                stopped_externally = true;
                break;
            }
            program.presentation_consumed();
        } else if presentation_requested {
            program.presentation_consumed();
        }
        if max_ticks == Some(ticks) && !program.is_complete() {
            break;
        }
        match status {
            SchedulerStatus::Progressed => pacer.finish_tick(tick_started),
            SchedulerStatus::WaitingUntil(deadline) => {
                if pacing == PacingMode::Turbo {
                    let remaining = program.duration_until(deadline);
                    match stop.recv_timeout(remaining) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                            stopped_externally = true;
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                } else {
                    pacer.finish_tick(tick_started);
                }
            }
            SchedulerStatus::Complete => break,
        }
    }
    if stopped_externally {
        program.stop_all();
    }
    publish_snapshot(program, ticks, latest, proxy, notification_pending);
    Ok(ticks)
}

fn advance_tick(
    program: &mut program::GeneratedProgram,
    pacing: PacingMode,
    started: Option<std::time::Duration>,
    pacer: &runtime::TickPacer<runtime::SystemPacingMechanism>,
    inputs: Option<&mpsc::Receiver<RuntimeInputEvent>>,
) -> SchedulerStatus {
    let started = started.unwrap_or_else(|| pacer.now());
    let budget = pacer.work_budget();
    if let Some(inputs) = inputs {
        apply_input_batch(inputs.try_iter(), |event| {
            program.apply_input(event);
        });
    }
    program.begin_tick();
    let redraw_requested = std::cell::Cell::new(false);
    runtime::run_tick(
        pacing,
        started,
        budget,
        || pacer.now(),
        || redraw_requested.get(),
        || {
            let status = program.advance_frame();
            redraw_requested.set(program.redraw_requested());
            status
        },
    )
}

fn apply_input_batch(
    events: impl IntoIterator<Item = RuntimeInputEvent>,
    mut apply: impl FnMut(RuntimeInputEvent),
) {
    let mut pending_variable_edits: Option<HashMap<String, f64>> = None;
    for event in events {
        match event {
            RuntimeInputEvent::VariableValue { key, value } => {
                pending_variable_edits
                    .get_or_insert_with(HashMap::new)
                    .insert(key, value);
            }
            event => apply(event),
        }
    }
    if let Some(edits) = pending_variable_edits {
        for (key, value) in edits {
            apply(RuntimeInputEvent::VariableValue { key, value });
        }
    }
}

fn wait_for_renderer(
    acknowledgements: &mpsc::Receiver<usize>,
    tick: usize,
    stop: &mpsc::Receiver<()>,
) -> bool {
    loop {
        match acknowledgements.recv_timeout(std::time::Duration::from_millis(10)) {
            Ok(acknowledged) if acknowledged >= tick => return true,
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {
                if stop_requested(stop) {
                    return false;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return false;
            }
        }
    }
}

fn stop_requested(receiver: &mpsc::Receiver<()>) -> bool {
    matches!(
        receiver.try_recv(),
        Ok(()) | Err(mpsc::TryRecvError::Disconnected)
    )
}

fn publish_snapshot(
    program: &program::GeneratedProgram,
    ticks: usize,
    latest: &Arc<RwLock<Arc<RuntimeSnapshot>>>,
    proxy: &winit::event_loop::EventLoopProxy<renderer::RendererEvent>,
    notification_pending: &AtomicBool,
) {
    let snapshot = Arc::new(program.observable_snapshot(ticks));
    *latest
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = snapshot;
    if !notification_pending.swap(true, Ordering::AcqRel) {
        let _ = proxy.send_event(renderer::RendererEvent::SnapshotUpdated);
    }
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

fn assert_expected_variables(
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

fn parse_run_options(args: impl IntoIterator<Item = String>) -> Result<RunOptions, String> {
    let mut fps = None;
    let mut turbo = false;
    let mut max_ticks = None;
    let mut graphical = false;
    let mut headless = false;
    let mut dev_mode = false;
    let mut test_mode = false;
    let mut hd_pen_mode = false;
    let mut project = None;
    let mut log_observables = false;
    let mut log_variables = false;
    let mut expected_variables = Vec::new();
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
                fps = Some(PacingMode::from_fps(value)?);
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
            "--headless" => {
                if headless || graphical {
                    return Err("choose either --headless or --graphical once".to_owned());
                }
                headless = true;
            }
            "--graphical" => {
                if graphical || headless {
                    return Err("choose either --headless or --graphical once".to_owned());
                }
                graphical = true;
            }
            "--dev" => dev_mode = true,
            "--hd-pen" => {
                if hd_pen_mode {
                    return Err("--hd-pen may only be specified once".to_owned());
                }
                hd_pen_mode = true;
            }
            "--test" => {
                if test_mode {
                    return Err("--test may only be specified once".to_owned());
                }
                test_mode = true;
            }
            "--project" => {
                if project.is_some() {
                    return Err("--project may only be specified once".to_owned());
                }
                project = Some(
                    args.next()
                        .ok_or("--project requires an .sb3 project path")?
                        .into(),
                );
            }
            "--log-observables" => {
                if log_observables {
                    return Err("--log-observables may only be specified once".to_owned());
                }
                log_observables = true;
            }
            "--log-variables" => {
                if log_variables {
                    return Err("--log-variables may only be specified once".to_owned());
                }
                log_variables = true;
            }
            "--expect-variable" => {
                let expression = args.next().ok_or("--expect-variable requires KEY=JSON")?;
                expected_variables.push(parse_expected_variable(&expression)?);
            }
            _ => return Err(format!("unknown option {argument:?}")),
        }
    }

    let pacing = if turbo {
        PacingMode::Turbo
    } else if let Some(fps) = fps {
        fps
    } else {
        PacingMode::from_fps(PacingMode::DEFAULT_FPS)?
    };
    if hd_pen_mode && !graphical {
        return Err("--hd-pen requires --graphical".to_owned());
    }
    Ok(RunOptions {
        pacing,
        max_ticks,
        graphical,
        dev_mode,
        test_mode,
        hd_pen_mode,
        project,
        log_observables,
        log_variables,
        expected_variables,
    })
}

fn parse_expected_variable(value: &str) -> Result<(String, ScratchValue), String> {
    let (key, value) = value
        .split_once('=')
        .ok_or("--expect-variable requires KEY=JSON")?;
    if key.is_empty() {
        return Err("--expect-variable requires a non-empty variable key".to_owned());
    }
    let value: serde_json::Value = serde_json::from_str(value)
        .map_err(|error| format!("invalid expected variable value: {error}"))?;
    let value = match value {
        serde_json::Value::Number(number) => ScratchValue::Number(
            number
                .as_f64()
                .ok_or("expected numeric variable value is out of range")?,
        ),
        serde_json::Value::String(text) => ScratchValue::Text(text),
        serde_json::Value::Bool(value) => ScratchValue::Boolean(value),
        _ => return Err("expected variable value must be a number, string, or boolean".to_owned()),
    };
    Ok((key.to_owned(), value))
}

#[cfg(test)]
mod tests;
