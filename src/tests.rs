use std::{cell::Cell, rc::Rc, time::Duration};

use crate::{
    apply_input_batch, assert_expected_variables, enqueue_pen_commands, parse_run_options,
    program::GeneratedProgram,
    project_window_title, run_headless,
    runtime::{
        Clock, PacingMechanism, PacingMode, PenCommand, PenStroke, ResumableScript,
        RuntimeInputEvent, RuntimeSnapshot, SchedulerStatus, ScratchState, ScratchValue,
        ScratchVariableMetadata, ScriptScheduler, ScriptStatus, VariableSnapshot,
        variable_snapshot_key,
    },
    wait_for_renderer,
};

#[test]
fn pen_command_batches_accumulate_across_coalesced_snapshots_in_order() {
    let pending = std::sync::RwLock::new(Vec::new());
    enqueue_pen_commands(&pending, vec![PenCommand::Clear]);
    enqueue_pen_commands(
        &pending,
        vec![PenCommand::Strokes(vec![PenStroke {
            start: [1.0, 2.0],
            end: [3.0, 4.0],
            color: [1.0, 0.0, 0.0, 1.0],
            diameter: 2.0,
            pixel_offset: 0.0,
        }])],
    );
    let commands = std::mem::take(
        &mut *pending
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );

    assert!(matches!(
        commands.as_slice(),
        [PenCommand::Clear, PenCommand::Strokes(_)]
    ));
}

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
fn cli_accepts_a_username_override_once() {
    let options = parse_run_options(["--username".to_owned(), "AdaUser".to_owned()]).unwrap();
    assert_eq!(options.username.as_deref(), Some("AdaUser"));
    assert!(parse_run_options(["--username".to_owned()]).is_err());
    assert!(
        parse_run_options([
            "--username".to_owned(),
            "one".to_owned(),
            "--username".to_owned(),
            "two".to_owned(),
        ])
        .is_err()
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
fn cli_defaults_to_headless_and_accepts_observation_options() {
    let default = parse_run_options(std::iter::empty()).unwrap();
    assert!(!default.graphical);
    assert!(!default.hd_pen_mode);
    assert!(default.project.is_none());

    let graphical = parse_run_options([
        "--graphical".to_owned(),
        "--project".to_owned(),
        "project.sb3".to_owned(),
        "--hd-pen".to_owned(),
        "--log-observables".to_owned(),
        "--log-variables".to_owned(),
        "--expect-variable".to_owned(),
        "Stage::score#score-id@shared=42".to_owned(),
    ])
    .unwrap();
    assert!(graphical.graphical);
    assert!(graphical.hd_pen_mode);
    assert!(graphical.log_observables);
    assert!(graphical.log_variables);
    assert_eq!(
        graphical.expected_variables[0].0,
        "Stage::score#score-id@shared"
    );
    assert_eq!(
        graphical.expected_variables[0].1,
        ScratchValue::Number(42.0)
    );
    assert!(parse_run_options(["--graphical".to_owned(), "--headless".to_owned()]).is_err());
}

#[test]
fn cli_requires_graphical_mode_for_hd_pen_and_rejects_duplicates() {
    assert!(parse_run_options(["--hd-pen".to_owned()]).is_err());
    assert!(
        parse_run_options([
            "--graphical".to_owned(),
            "--hd-pen".to_owned(),
            "--hd-pen".to_owned(),
        ])
        .unwrap_err()
        .contains("may only be specified once")
    );
}

#[test]
fn project_window_title_includes_name_and_hd_pen_mode() {
    let project = std::path::Path::new(r"C:\projects\pi-series.sb3");
    assert_eq!(project_window_title(project, false), "AIDA - pi-series");
    assert_eq!(project_window_title(project, true), "AIDA HD - pi-series");
}

#[test]
fn cli_enables_development_monitor_controls() {
    let options = parse_run_options([
        "--graphical".to_owned(),
        "--dev".to_owned(),
        "--project".to_owned(),
        "project.sb3".to_owned(),
    ])
    .unwrap();
    assert!(options.graphical);
    assert!(options.dev_mode);

    let default = parse_run_options(std::iter::empty()).unwrap();
    assert!(!default.dev_mode);
}

#[test]
fn headless_runner_executes_without_creating_a_window() {
    let options = parse_run_options([
        "--headless".to_owned(),
        "--turbo".to_owned(),
        "--max-ticks".to_owned(),
        "1".to_owned(),
    ])
    .unwrap();
    assert!(run_headless(options).is_ok());
}

#[test]
fn headless_variable_assertions_match_scoped_snapshot_keys() {
    let snapshot = RuntimeSnapshot {
        tick: 3,
        variables: vec![VariableSnapshot {
            key: "Sprite::score#score-id@2".to_owned(),
            id: "score-id".to_owned(),
            name: "score".to_owned(),
            target: "Sprite".to_owned(),
            instance_id: Some(2),
            value: ScratchValue::Number(9.0),
            monitor_visible: true,
        }],
        ..RuntimeSnapshot::default()
    };
    assert!(
        assert_expected_variables(
            &snapshot,
            &[(
                "Sprite::score#score-id@2".to_owned(),
                ScratchValue::Number(9.0)
            )]
        )
        .is_ok()
    );
    assert!(
        assert_expected_variables(
            &snapshot,
            &[(
                "Sprite::score#score-id@2".to_owned(),
                ScratchValue::Number(10.0)
            )]
        )
        .unwrap_err()
        .contains("observed Number(9.0)")
    );
}

#[test]
fn normal_redraw_gate_waits_for_the_matching_renderer_submission() {
    let (ack_tx, ack_rx) = std::sync::mpsc::channel();
    let (stop_tx, stop_rx) = std::sync::mpsc::channel();
    ack_tx.send(4).unwrap();
    ack_tx.send(5).unwrap();
    assert!(wait_for_renderer(&ack_rx, 5, &stop_rx));
    drop(stop_tx);
}

#[test]
fn broadcast_fixture_returns_between_rounds_and_publishes_monitor_changes() {
    let clock = TestClock(Rc::new(Cell::new(0)));
    let mut program = GeneratedProgram::new_with_clock(Box::new(clock));
    program.begin_tick();

    assert_eq!(program.advance_frame(), SchedulerStatus::Progressed);
    assert!(program.presentation_requested());
    let snapshot = program.observable_snapshot(1);
    assert!(snapshot.variables.iter().any(|variable| {
        variable.id == "`jEk@4|i[#Fk?(8x)AV.-my variable"
            && variable.value == ScratchValue::Text("0".to_owned())
    }));
}

#[test]
fn sensing_monitor_values_survive_stop_and_mouse_values_remain_live() {
    let mut program = GeneratedProgram::new();
    assert!(program.has_live_sensing_monitors());
    program.apply_input(RuntimeInputEvent::MousePosition { x: 41.0, y: -17.0 });

    let mouse_key = "__reporter/Stage/sensing_mousex/sensing_mousex";
    let first = program.observable_snapshot(1);
    assert!(first.reporters.iter().any(|reporter| {
        reporter.key == mouse_key && reporter.value == ScratchValue::Number(41.0)
    }));
    let weekday = first
        .reporters
        .iter()
        .find(|reporter| reporter.key == "__reporter/Stage/sensing_current/current_dayofweek")
        .expect("weekday monitor is present");
    assert!(matches!(weekday.value, ScratchValue::Number(value) if (1.0..=7.0).contains(&value)));

    program.stop_all();
    let stopped = program.observable_snapshot(2);
    assert!(stopped.reporters.iter().any(|reporter| {
        reporter.key == mouse_key && reporter.value == ScratchValue::Number(41.0)
    }));

    program.begin_tick();
    program.apply_input(RuntimeInputEvent::MousePosition { x: 12.0, y: 8.0 });
    let after_input = program.observable_snapshot(3);
    assert!(after_input.reporters.iter().any(|reporter| {
        reporter.key == mouse_key && reporter.value == ScratchValue::Number(12.0)
    }));
    assert!(program.presentation_requested());
}

#[derive(Clone)]
struct FrameClock(Rc<Cell<Duration>>);

impl Clock for FrameClock {
    fn now_millis(&self) -> u64 {
        self.0.get().as_millis().min(u64::MAX as u128) as u64
    }
}

#[test]
fn think_fixture_resumes_timed_blocks_with_stage_progress_and_stops_in_order() {
    // exact frame timestamps separate scheduler behavior from host timer jitter.
    for fps in [30u64, 60] {
        let clock = FrameClock(Rc::new(Cell::new(Duration::ZERO)));
        let mut program = GeneratedProgram::new_with_clock(Box::new(clock.clone()));
        let mode = PacingMode::from_fps(fps as f64).unwrap();
        let frame_interval = Duration::from_secs_f64(1.0 / fps as f64);

        for frame in 0..=fps * 6 {
            let now = Duration::from_nanos(
                (u128::from(frame) * 1_000_000_000 / u128::from(fps)) as u64,
            );
            clock.0.set(now);
            program.begin_tick();
            let redraw_requested = std::cell::Cell::new(false);
            let status = crate::runtime::run_tick(
                mode,
                now,
                frame_interval.mul_f64(0.75),
                || clock.0.get(),
                || redraw_requested.get(),
                || {
                    let status = program.advance_frame();
                    redraw_requested.set(program.redraw_requested());
                    status
                },
            );
            if program.presentation_requested() {
                program.presentation_consumed();
            }

            let value = program.state().get_shared(0).clone();
            if frame == fps * 2 {
                assert_eq!(value, ScratchValue::Number((fps * 2) as f64));
                assert_eq!(
                    program.state().backdrop_number_name(true),
                    ScratchValue::Number(2.0)
                );
                let bubble = program
                    .observable_snapshot(frame as usize)
                    .layers
                    .into_iter()
                    .find(|layer| layer.target == "Sprite1")
                    .and_then(|layer| layer.bubble)
                    .expect("second timed think bubble is visible at its start");
                assert_eq!(bubble.kind, crate::runtime::BubbleKind::Think);
                assert!(bubble.text.starts_with("Lorem ipsum"));
            } else if frame == fps * 4 {
                assert_eq!(value, ScratchValue::Number((fps * 4) as f64));
                let bubble = program
                    .observable_snapshot(frame as usize)
                    .layers
                    .into_iter()
                    .find(|layer| layer.target == "Sprite1")
                    .and_then(|layer| layer.bubble)
                    .expect("timed say bubble is visible at its start");
                assert_eq!(bubble.kind, crate::runtime::BubbleKind::Say);
                assert_eq!(bubble.text, "Hello!");
            } else if frame == fps * 6 {
                assert_eq!(status, SchedulerStatus::Complete);
                assert_eq!(value, ScratchValue::Number((fps * 6 + 1) as f64));
                assert_eq!(
                    program.state().backdrop_number_name(true),
                    ScratchValue::Number(2.0)
                );
                assert!(program.is_complete());
                assert!(program
                    .observable_snapshot(frame as usize)
                    .layers
                    .iter()
                    .all(|layer| layer.bubble.is_none()));
                let stopped_value = program.state().get_shared(0).clone();
                assert_eq!(program.advance_frame(), SchedulerStatus::Complete);
                assert_eq!(program.state().get_shared(0), &stopped_value);
            }

            if status == SchedulerStatus::Complete {
                break;
            }
        }
    }
}

#[test]
fn motion_point_fixture_runs_each_clone_on_its_own_target_and_position() {
    let clock = TestClock(Rc::new(Cell::new(0)));
    let mut program = GeneratedProgram::new_with_clock(Box::new(clock.clone()));
    let mut snapshot = RuntimeSnapshot::default();

    for tick in 0..100 {
        program.begin_tick();
        match program.advance_frame() {
            SchedulerStatus::Progressed => {}
            SchedulerStatus::WaitingUntil(deadline) => clock.0.set(deadline),
            SchedulerStatus::Complete => break,
        }
        snapshot = program.observable_snapshot(tick);
    }

    let layer = |target: &str, original: bool| {
        snapshot
            .layers
            .iter()
            .find(|layer| layer.target == target && layer.original == original)
            .unwrap_or_else(|| panic!("missing {target} {original:?} layer: {:?}", snapshot.layers))
    };
    assert_eq!(
        (layer("Sprite1", true).x, layer("Sprite1", true).y),
        (240.0, 0.0)
    );
    assert_eq!(
        (layer("Sprite2", true).x, layer("Sprite2", true).y),
        (-240.0, 0.0)
    );
    assert_eq!(
        (layer("Sprite1", false).x, layer("Sprite1", false).y),
        (0.0, -180.0)
    );
    assert_eq!(
        (layer("Sprite2", false).x, layer("Sprite2", false).y),
        (0.0, 180.0)
    );
    assert_eq!(layer("Sprite1", false).size, 100.0);
    assert_eq!(layer("Sprite2", false).size, 100.0);
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

struct SliderCounter {
    target_instance: usize,
    remaining: usize,
    values: Rc<std::cell::RefCell<Vec<f64>>>,
}

impl ResumableScript for SliderCounter {
    fn step(&mut self, state: &mut ScratchState, _clock: &dyn Clock) -> ScriptStatus {
        let current = match state.get(0) {
            ScratchValue::Number(value) => *value,
            value => panic!("counter slider should be numeric, got {value:?}"),
        };
        let next = current + 1.0;
        state.set(0, ScratchValue::Number(next));
        self.values.borrow_mut().push(next);
        self.remaining -= 1;
        if self.remaining == 0 {
            ScriptStatus::Complete
        } else {
            ScriptStatus::Yield
        }
    }

    fn target_instance(&self) -> usize {
        self.target_instance
    }
}

fn verify_slider_edit_reloads_running_counter(is_stage: bool) {
    let target_index = usize::from(!is_stage);
    let target_name = if is_stage { "Stage" } else { "Sprite" };
    let id = if is_stage {
        "stage-slider"
    } else {
        "sprite-slider"
    };
    let metadata =
        ScratchVariableMetadata::for_target(id, "limit", target_name, target_index, is_stage)
            .with_monitor_visible(true);
    let mut state = ScratchState::with_metadata(vec![ScratchValue::Number(10.0)], vec![metadata]);
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite", false);
    let instance = state.original_instance(target_index).unwrap();
    let key = variable_snapshot_key(target_name, "limit", id, (!is_stage).then_some(instance));
    let values = Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut scheduler = ScriptScheduler::new(vec![SliderCounter {
        target_instance: instance,
        remaining: 5,
        values: Rc::clone(&values),
    }]);

    assert_eq!(scheduler.advance(&mut state), SchedulerStatus::Progressed);
    assert_eq!(&*values.borrow(), &[11.0]);

    let (sender, receiver) = std::sync::mpsc::channel();
    sender
        .send(RuntimeInputEvent::VariableValue {
            key: key.clone(),
            value: 40.0,
        })
        .unwrap();
    sender
        .send(RuntimeInputEvent::VariableValue { key, value: 2.0 })
        .unwrap();
    apply_input_batch(receiver.try_iter(), |event| {
        state.apply_input(event);
    });
    assert!(state.presentation_requested());
    let snapshot = state.observable_snapshot(1);
    assert!(snapshot.variables.iter().any(|variable| {
        variable.value == ScratchValue::Number(2.0)
            && variable.key
                == variable_snapshot_key(target_name, "limit", id, (!is_stage).then_some(instance))
    }));

    assert_eq!(scheduler.advance(&mut state), SchedulerStatus::Progressed);
    assert_eq!(&*values.borrow(), &[11.0, 3.0]);
    let snapshot = state.observable_snapshot(2);
    assert!(snapshot.variables.iter().any(|variable| {
        variable.value == ScratchValue::Number(3.0)
            && variable.key
                == variable_snapshot_key(target_name, "limit", id, (!is_stage).then_some(instance))
    }));
}

#[test]
fn slider_edits_reload_running_global_and_sprite_local_variable_caches() {
    verify_slider_edit_reloads_running_counter(true);
    verify_slider_edit_reloads_running_counter(false);
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
