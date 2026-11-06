use std::{
    sync::{Arc, RwLock, atomic::AtomicBool, mpsc},
    thread,
    time::Duration,
};

use aida_ast::load_sb3;
use winit::event_loop::{ControlFlow, EventLoop};

use super::cli::RunOptions;
use crate::runtime::{PacingMode, PenCommand, RuntimeSnapshot};
use crate::{graphics, program, runtime};

pub(crate) fn run(options: RunOptions) -> Result<(), String> {
    if options.graphical {
        run_graphical(options)
    } else {
        super::headless::run_headless(options)
    }
}

use super::{window, worker};

fn run_graphical(options: RunOptions) -> Result<(), String> {
    let project_path = options
        .project
        .as_ref()
        .ok_or_else(|| "--graphical requires --project <file.sb3>".to_owned())?;
    let loaded = load_sb3(project_path, "resources/projects")
        .map_err(|error| format!("failed to load project: {error}"))?;
    let sound_library = if program::GeneratedProgram::uses_sound() {
        Some(
            runtime::SoundLibrary::load_project(&loaded.project, &loaded.assets_dir)
                .map_err(|error| format!("failed to load sounds: {error}"))?,
        )
    } else {
        None
    };
    let fence_geometries = graphics::fence_geometries(&loaded.project, &loaded.assets_dir);
    let project = Arc::new(loaded.project);
    let window_title = project_window_title(project_path, options.hd_pen_mode);

    // winit requires its event loop and window creation to stay on the main
    // thread on macos and for cross-platform compatibility.
    let event_loop = EventLoop::<window::RendererEvent>::with_user_event()
        .build()
        .map_err(|error| format!("failed to create the window event loop: {error}"))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let latest = Arc::new(RwLock::new(Arc::new(RuntimeSnapshot::default())));
    let pending_pen_commands = Arc::new(RwLock::new(Vec::<PenCommand>::new()));
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
    let worker_pen_commands = Arc::clone(&pending_pen_commands);
    let worker_notification_pending = Arc::clone(&notification_pending);
    let worker_options = options.clone();
    let worker_proxy = proxy.clone();
    let worker_sound_library = sound_library;
    let worker = thread::Builder::new()
        .name("aida-scratch-execution".to_owned())
        .spawn(move || {
            let mut program = program::GeneratedProgram::new();
            if let Some(sound_library) = worker_sound_library {
                program.set_sound_library(sound_library);
                if let Err(error) = program.enable_realtime_audio() {
                    eprintln!(
                        "audio output unavailable; using the controlled playback clock: {error}"
                    );
                }
            }
            for (target, geometry) in fence_geometries {
                program.set_target_fence_geometry(target, geometry);
            }
            if let Some(username) = worker_options.username.as_ref() {
                program.set_username(username.clone());
            }
            program.set_observable_event_sink(observable_sender);
            let worker_context = worker::WorkerContext {
                stop: &stop_receiver,
                latest: &worker_latest,
                pending_pen_commands: &worker_pen_commands,
                proxy: &worker_proxy,
                notification_pending: &worker_notification_pending,
                execution_start: &execution_start_receiver,
                acknowledgements: &ack_receiver,
                inputs: &input_receiver,
            };
            worker::publish_snapshot(&mut program, 0, &worker_context);
            let _ = ready_sender.send(());
            let result = worker::run_worker(
                &mut program,
                worker_options.pacing,
                worker_options.max_ticks,
                worker_options.test_mode,
                &worker_context,
            );
            if let Err(error) = result {
                eprintln!("execution worker failed: {error}");
            }
            let _ = worker_proxy.send_event(window::RendererEvent::ExecutionFinished);
        })
        .map_err(|error| format!("failed to start execution worker: {error}"))?;
    ready_receiver
        .recv()
        .map_err(|error| format!("execution worker did not start: {error}"))?;

    let mut app = window::RendererApp::new(window::RendererAppConfig {
        project,
        assets_dir: loaded.assets_dir,
        window_title,
        channels: window::RendererAppChannels {
            latest,
            pending_pen_commands,
            notification_pending,
            observable_events: observable_receiver,
            stop_sender: stop_sender.clone(),
            execution_start_sender: execution_start_sender.clone(),
            acknowledgement_sender: ack_sender,
            input_sender,
        },
        requested_frame_interval: match options.pacing {
            PacingMode::Paced(interval) => interval,
            PacingMode::Turbo => Duration::ZERO,
        },
        expected_variables: options.expected_variables,
        dev_mode: options.dev_mode,
        test_mode: options.test_mode,
        hd_pen_mode: options.hd_pen_mode,
    });
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

pub(crate) fn project_window_title(project_path: &std::path::Path, hd_pen_mode: bool) -> String {
    let project_name = project_path
        .file_stem()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "Scratch project".into());
    let prefix = if hd_pen_mode { "AIDA HD" } else { "AIDA" };
    format!("{prefix} - {project_name}")
}
