use std::{cell::Cell, rc::Rc};

use crate::{
    apply_input_batch, assert_expected_variables, parse_run_options,
    program::GeneratedProgram,
    project_window_title, run_headless,
    runtime::{
        Clock, PacingMechanism, PacingMode, ResumableScript, RuntimeInputEvent, RuntimeSnapshot,
        SchedulerStatus, ScratchState, ScratchValue, ScratchVariableMetadata, ScriptScheduler,
        ScriptStatus, VariableSnapshot, variable_snapshot_key,
    },
    wait_for_renderer,
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
