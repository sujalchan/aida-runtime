use std::{
    collections::HashMap,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use super::window::RendererEvent;
use crate::runtime::{PacingMode, PenCommand, RuntimeInputEvent, RuntimeSnapshot, SchedulerStatus};
use crate::{program, runtime};

pub(super) struct WorkerContext<'a> {
    pub(super) stop: &'a mpsc::Receiver<()>,
    pub(super) latest: &'a Arc<RwLock<Arc<RuntimeSnapshot>>>,
    pub(super) pending_pen_commands: &'a Arc<RwLock<Vec<PenCommand>>>,
    pub(super) proxy: &'a winit::event_loop::EventLoopProxy<RendererEvent>,
    pub(super) notification_pending: &'a AtomicBool,
    pub(super) execution_start: &'a mpsc::Receiver<()>,
    pub(super) acknowledgements: &'a mpsc::Receiver<usize>,
    pub(super) inputs: &'a mpsc::Receiver<RuntimeInputEvent>,
}

pub(super) fn run_worker(
    program: &mut program::GeneratedProgram,
    pacing: PacingMode,
    max_ticks: Option<usize>,
    test_mode: bool,
    context: &WorkerContext<'_>,
) -> Result<usize, String> {
    let _ = context.execution_start.recv();
    let mut pacer = runtime::TickPacer::new(pacing, runtime::SystemPacingMechanism::new());
    let mut ticks = 0;
    let mut stopped_externally = false;
    let mut next_live_monitor_update = Instant::now();
    let live_monitor_interval = match pacing {
        PacingMode::Paced(interval) => interval,
        PacingMode::Turbo => Duration::from_millis(33),
    };
    loop {
        if max_ticks.is_some_and(|limit| ticks >= limit) {
            break;
        }
        if stop_requested(context.stop) {
            stopped_externally = true;
            break;
        }
        let tick_started = pacer.begin_tick();
        let was_complete = program.is_complete();
        let status = advance_tick(program, pacing, tick_started, &pacer, Some(context.inputs));
        ticks += 1;
        let live_monitor_due =
            program.has_live_sensing_monitors() && Instant::now() >= next_live_monitor_update;
        let presentation_requested = program.presentation_requested()
            || live_monitor_due
            || (matches!(status, SchedulerStatus::Complete) && !was_complete);
        if presentation_requested {
            publish_snapshot(program, ticks, context);
            if live_monitor_due {
                next_live_monitor_update = Instant::now() + live_monitor_interval;
            }
        }
        if presentation_requested && pacing != PacingMode::Turbo {
            if !wait_for_renderer(context.acknowledgements, ticks, context.stop) {
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
                    match context.stop.recv_timeout(remaining) {
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
            SchedulerStatus::WaitingForSignal if pacing == PacingMode::Turbo => {
                match context.stop.recv_timeout(Duration::from_millis(8)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                        stopped_externally = true;
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
            SchedulerStatus::WaitingForSignal => pacer.finish_tick(tick_started),
            SchedulerStatus::Complete if test_mode => break,
            SchedulerStatus::Complete if pacing == PacingMode::Turbo => {
                // keep the graphical input and monitor pump alive without a busy loop.
                match context.stop.recv_timeout(Duration::from_millis(33)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                        stopped_externally = true;
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
            SchedulerStatus::Complete => pacer.finish_tick(tick_started),
        }
    }
    if stopped_externally {
        publish_snapshot(program, ticks, context);
        program.stop_all();
    }
    publish_snapshot(program, ticks, context);
    Ok(ticks)
}

pub(super) fn advance_tick(
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
    // batch pen commands only while unpresented marks remain queued.
    let pen_marks_pending = std::cell::Cell::new(false);
    runtime::run_tick(
        pacing,
        started,
        budget,
        || pacer.now(),
        || redraw_requested.get(),
        || pen_marks_pending.get(),
        || {
            let status = program.advance_frame();
            redraw_requested.set(program.redraw_requested());
            pen_marks_pending.set(program.state().has_pending_pen_commands());
            status
        },
    )
}

pub(crate) fn apply_input_batch(
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

pub(crate) fn wait_for_renderer(
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

pub(super) fn stop_requested(receiver: &mpsc::Receiver<()>) -> bool {
    matches!(
        receiver.try_recv(),
        Ok(()) | Err(mpsc::TryRecvError::Disconnected)
    )
}

pub(super) fn publish_snapshot(
    program: &mut program::GeneratedProgram,
    ticks: usize,
    context: &WorkerContext<'_>,
) {
    let snapshot = program.renderer_snapshot(ticks);
    let pen_commands = program.take_pending_pen_commands();
    enqueue_pen_commands(context.pending_pen_commands, pen_commands);
    let snapshot = Arc::new(snapshot);
    *context
        .latest
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = snapshot;
    if !context.notification_pending.swap(true, Ordering::AcqRel) {
        let _ = context.proxy.send_event(RendererEvent::SnapshotUpdated);
    }
}

// retain incremental pen operations even while renderer snapshots are coalesced.
pub(crate) fn enqueue_pen_commands(pending: &RwLock<Vec<PenCommand>>, commands: Vec<PenCommand>) {
    if !commands.is_empty() {
        pending
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend(commands);
    }
}
