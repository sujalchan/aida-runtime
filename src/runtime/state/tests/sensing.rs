use super::*;

#[test]
fn pointer_click_timing_excludes_drags_overlays_and_outside_presses() {
    assert!(pointer_click_activates(true, true, false, false, false));
    assert!(!pointer_click_activates(false, true, false, false, false));
    assert!(!pointer_click_activates(true, true, false, true, false));
    assert!(!pointer_click_activates(true, false, false, false, false));
    assert!(!pointer_click_activates(true, true, false, false, true));
    assert!(!pointer_click_activates(true, true, true, false, false));
    assert!(pointer_click_activates(false, true, true, false, false));
    assert!(!pointer_click_activates(false, true, true, true, false));
}

#[test]
fn unavailable_loudness_is_negative_one_and_injectable_for_edge_hat_checks() {
    let mut state = ScratchState::new(Vec::new());
    assert_eq!(state.loudness(), -1.0);
    assert!(state.loudness() > -10.0);
    state.set_loudness(-14.5);
    assert_ne!(
        state.loudness().partial_cmp(&-10.0),
        Some(std::cmp::Ordering::Greater)
    );
    state.set_loudness(f64::NAN);
    assert_eq!(state.loudness(), -1.0);
}

#[test]
fn project_timer_is_shared_and_resettable_without_scheduler_steps() {
    let mut state = ScratchState::new(Vec::new());
    state.reset_project_timer(250);
    assert_eq!(state.project_timer_millis(1_250), 1_000);
    assert_eq!(state.project_timer_seconds(1_750), 1.5);
    state.reset_project_timer(2_000);
    assert_eq!(state.project_timer_millis(2_750), 750);
    assert_eq!(
        state
            .observable_snapshot_with_timer(4, 2_750)
            .project_timer_millis,
        750
    );
    assert!(!state.presentation_requested());
    state.set_project_timer_monitor_visible(true);
    state.begin_tick();
    state.reset_project_timer(3_000);
    assert!(state.presentation_requested());
}

#[test]
fn keyboard_events_match_scratch_names_and_key_argument_conversion() {
    let mut state = ScratchState::new(Vec::new());
    state.begin_tick();
    assert!(!state.redraw_requested());
    state.set_xy(12.0, -4.0);
    state.request_redraw();
    assert!(state.redraw_requested());
    state.presentation_consumed();
    assert!(!state.redraw_requested());

    assert_eq!(
        state.apply_input(RuntimeInputEvent::Key {
            name: "A".to_owned(),
            pressed: true,
        }),
        Some("a".to_owned())
    );
    assert_eq!(
        state.apply_input(RuntimeInputEvent::Key {
            name: "a".to_owned(),
            pressed: true,
        }),
        Some("a".to_owned())
    );

    assert!(state.key_pressed("a"));
    assert!(state.key_pressed(65.0));
    assert!(state.key_pressed(65.5));
    assert!(!state.key_pressed("65"));
    assert!(state.key_pressed("any"));

    for (host, scratch) in [
        ("ArrowUp", "up arrow"),
        ("Down", "down arrow"),
        ("ArrowLeft", "left arrow"),
        ("Right", "right arrow"),
        (" ", "space"),
        ("Enter", "enter"),
        ("7", "7"),
        ("6", "6"),
        ("?", "?"),
        ("æ", "æ"),
    ] {
        assert!(
            state
                .apply_input(RuntimeInputEvent::Key {
                    name: host.to_owned(),
                    pressed: true,
                })
                .is_some(),
            "host key {host:?} should be recognized"
        );
        assert!(
            state.key_pressed(scratch),
            "Scratch key {scratch:?} should be held"
        );
    }
    assert!(state.key_pressed(32.0));
    assert!(state.key_pressed(38.0));
    assert!(state.key_pressed(40.0));
    assert!(state.key_pressed(37.0));
    assert!(state.key_pressed(39.0));
    assert!(state.key_pressed(55.0));
    assert!(state.key_pressed("65"));

    // multi-character named keys and non-bmp characters are not Scratch keys.
    assert_eq!(
        state.apply_input(RuntimeInputEvent::Key {
            name: "Shift".to_owned(),
            pressed: true,
        }),
        None
    );
    assert_eq!(
        state.apply_input(RuntimeInputEvent::Key {
            name: "F1".to_owned(),
            pressed: true,
        }),
        None
    );
    assert_eq!(
        state.apply_input(RuntimeInputEvent::Key {
            name: "👋".to_owned(),
            pressed: true,
        }),
        None
    );
    assert!(state.key_pressed("any"));

    state.apply_input(RuntimeInputEvent::MousePosition { x: 21.0, y: -7.0 });
    state.apply_input(RuntimeInputEvent::MouseButton { pressed: true });
    assert_eq!((state.mouse_x(), state.mouse_y()), (21.0, -7.0));
    assert!(state.mouse_down());
    state.apply_input(RuntimeInputEvent::Key {
        name: "a".to_owned(),
        pressed: false,
    });
    assert!(!state.key_pressed("a"));
    assert!(!state.key_pressed(65.0));
    assert!(state.key_pressed("any"));
    state.apply_input(RuntimeInputEvent::ClearKeys);
    assert!(!state.key_pressed("any"));
    assert!(!state.key_pressed("up arrow"));
}

#[test]
fn keyboard_release_and_focus_loss_clear_only_held_key_state() {
    let mut state = ScratchState::new(vec![0.0_f64.into()]);
    state.apply_input(RuntimeInputEvent::Key {
        name: "ArrowUp".to_owned(),
        pressed: true,
    });
    state.apply_input(RuntimeInputEvent::Key {
        name: "ArrowLeft".to_owned(),
        pressed: true,
    });
    state.apply_input(RuntimeInputEvent::Key {
        name: "Up".to_owned(),
        pressed: false,
    });
    assert!(!state.key_pressed("up arrow"));
    assert!(state.key_pressed("left arrow"));
    state.apply_input(RuntimeInputEvent::ClearKeys);
    assert!(!state.key_pressed("left arrow"));
    assert_eq!(state.get(0), &ScratchValue::Number(0.0));
}

#[test]
fn sensing_values_are_scratch_values_and_variable_visibility_is_snapshotted() {
    let mut state = ScratchState::with_project_data(
        vec![ScratchValue::Number(5.0)],
        vec![
            ScratchVariableMetadata::for_target("v", "score", "Stage", 0, true)
                .with_monitor_visible(true),
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    state.register_target(0, "Stage", true);
    assert!(matches!(state.days_since_2000(), day if day > 9_000.0));
    assert!((1.0..=12.0).contains(&state.current_month()));
    assert!((1.0..=7.0).contains(&state.current_day_of_week()));
    assert_eq!(state.loudness(), -1.0);
    assert_eq!(state.get(0), &ScratchValue::Number(5.0));

    let username = state.username().to_owned();
    assert!(username.strip_prefix("player").is_some_and(|suffix| {
        suffix.len() == 4 && suffix.bytes().all(|byte| byte.is_ascii_digit())
    }));
    state.set_username("AdaUser".to_owned());
    assert_eq!(state.username(), "AdaUser");

    state.begin_tick();
    state.set_variable_visible(0, false);
    let snapshot = state.observable_snapshot(1);
    assert!(!snapshot.variables[0].monitor_visible);
    assert!(state.presentation_requested());
    assert_eq!(snapshot.variables[0].value, ScratchValue::Number(5.0));
}

#[test]
fn sensing_of_reads_live_original_target_properties_and_scoped_scalar_values() {
    let variables = vec![
        ScratchVariableMetadata::for_target("stage-shared", "shared", "Stage", 0, true),
        ScratchVariableMetadata::for_target("sprite-one-shared", "shared", "Sprite1", 1, false),
        ScratchVariableMetadata::for_target("sprite-two-shared", "shared", "Sprite2", 2, false),
        ScratchVariableMetadata::for_target("stage-x", "x position", "Stage", 0, true),
        ScratchVariableMetadata::for_target("sprite-two-x", "x position", "Sprite2", 2, false),
        ScratchVariableMetadata::for_target("stage-volume", "volume", "Stage", 0, true),
        ScratchVariableMetadata::for_target("stage-global", "global only", "Stage", 0, true),
        ScratchVariableMetadata::for_target(
            "sprite-two-backdrop",
            "backdrop name",
            "Sprite2",
            2,
            false,
        ),
    ];
    let mut state = ScratchState::with_project_data(
        vec![
            ScratchValue::Text("stage value".to_owned()),
            ScratchValue::Text("sprite one value".to_owned()),
            ScratchValue::Number(20.0),
            ScratchValue::Number(999.0),
            ScratchValue::Number(777.0),
            ScratchValue::Number(999.0),
            ScratchValue::Text("global value".to_owned()),
            ScratchValue::Text("sprite property".to_owned()),
        ],
        variables,
        vec![ScratchList::new(vec!["entry".into()])],
        vec![ScratchListMetadata::for_target(
            "sprite-two-list",
            "items",
            "Sprite2",
            2,
            false,
        )],
        vec![false],
    );
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite1", false);
    state.register_target(2, "Sprite2", false);
    state.register_target(3, "Sprite2 copy", false);
    state.set_backdrops(vec!["backdrop1".into(), "backdrop2".into()], 0);
    state.set_target_costumes(0, vec!["backdrop1".into(), "backdrop2".into()], 0);
    state.set_target_costumes(1, vec!["one".into()], 0);
    state.set_target_costumes(2, vec!["costume1".into(), "costume2".into()], 0);
    state.set_target_costumes(3, vec!["copy".into()], 0);
    state.set_original_xy(2, 10.5, -20.25);
    state.set_original_visual(2, 45.0, 80.0, true);
    state.set_original_volume(0, 70.0);
    state.set_original_volume(2, 40.0);
    state.set_original_xy(3, 300.0, 301.0);

    let stage = state.original_instance(0).unwrap();
    state.set_current_instance(stage);
    state.switch_backdrop("backdrop2");
    state.set_sound_volume(55.0, false, 0);
    state.set_shared(0, ScratchValue::Text("updated stage value".to_owned()));

    let sprite1 = state.original_instance(1).unwrap();
    state.set_current_instance(sprite1);
    state.set(1, ScratchValue::Text("updated sprite one".to_owned()));

    let sprite2 = state.original_instance(2).unwrap();
    state.set_current_instance(sprite2);
    state.set(2, ScratchValue::Text("updated sprite two".to_owned()));
    state.switch_costume_to("costume2".into());
    state.set_size(125.0);
    state.set_sound_volume(35.0, false, 0);

    assert_eq!(
        state.sensing_of_target_slot(0, "backdrop #", None),
        ScratchValue::Number(2.0)
    );
    assert_eq!(
        state.sensing_of_target_slot(0, "background #", None),
        ScratchValue::Number(2.0)
    );
    assert_eq!(
        state.sensing_of_target_slot(0, "backdrop name", None),
        ScratchValue::Text("backdrop2".into())
    );
    assert_eq!(
        state.sensing_of_target_slot(0, "volume", Some(5)),
        ScratchValue::Number(55.0)
    );
    assert_eq!(
        state.sensing_of_target_slot(0, "shared", Some(0)),
        ScratchValue::Text("updated stage value".into())
    );

    assert_eq!(
        state.sensing_of_target_slot(1, "shared", Some(1)),
        ScratchValue::Text("updated sprite one".into())
    );
    assert_eq!(
        state.sensing_of_target_slot(2, "x position", Some(4)),
        ScratchValue::Number(10.5)
    );
    assert_eq!(
        state.sensing_of_target_slot(2, "y position", None),
        ScratchValue::Number(-20.25)
    );
    assert_eq!(
        state.sensing_of_target_slot(2, "direction", None),
        ScratchValue::Number(45.0)
    );
    assert_eq!(
        state.sensing_of_target_slot(2, "costume #", None),
        ScratchValue::Number(2.0)
    );
    assert_eq!(
        state.sensing_of_target_slot(2, "costume name", None),
        ScratchValue::Text("costume2".into())
    );
    assert_eq!(
        state.sensing_of_target_slot(2, "size", None),
        ScratchValue::Number(125.0)
    );
    assert_eq!(
        state.sensing_of_target_slot(2, "volume", None),
        ScratchValue::Number(35.0)
    );
    assert_eq!(
        state.sensing_of_target_slot(2, "shared", Some(2)),
        ScratchValue::Text("updated sprite two".into())
    );

    // A sprite's local variable wins only for that exact target. Stage values,
    // lists, and case-folded names are not considered by this lookup.
    assert_eq!(
        state.sensing_of_target_slot(2, "global only", None),
        ScratchValue::Number(0.0)
    );
    assert_eq!(
        state.sensing_of_target_slot(2, "items", None),
        ScratchValue::Number(0.0)
    );
    assert_eq!(
        state.sensing_of_target_slot(2, "Shared", None),
        ScratchValue::Number(0.0)
    );
    assert_eq!(
        state.sensing_of_target_slot(2, "backdrop name", Some(7)),
        ScratchValue::Text("sprite property".into())
    );
    assert_eq!(
        state.sensing_of_target_slot(0, "costume #", None),
        ScratchValue::Number(0.0)
    );

    let clone = state.create_clone(sprite2, 2, 300).unwrap();
    state.set_current_instance(clone);
    state.set_xy(900.0, 901.0);
    state.set(2, ScratchValue::Text("clone value".to_owned()));
    assert_eq!(
        state.sensing_of_by_name("Sprite2", "x position"),
        ScratchValue::Number(10.5)
    );
    assert_eq!(
        state.sensing_of_by_name("Sprite2", "shared"),
        ScratchValue::Text("updated sprite two".into())
    );
    assert_eq!(
        state.sensing_of_by_name("_stage_", "backdrop name"),
        ScratchValue::Text("backdrop2".into())
    );
    assert_eq!(
        state.sensing_of_by_name("Sprite2 copy", "x position"),
        ScratchValue::Number(300.0)
    );
    assert_eq!(
        state.sensing_of_by_name("sprite2", "x position"),
        ScratchValue::Number(0.0)
    );
    assert_eq!(
        state.sensing_of_by_name(123.0, "x position"),
        ScratchValue::Number(0.0)
    );
    assert_eq!(
        state.sensing_of_by_name("Missing", "x position"),
        ScratchValue::Number(0.0)
    );
}

#[test]
fn sensing_of_dynamic_target_uses_scratch_string_conversion() {
    let mut state = ScratchState::new(Vec::new());
    state.register_target(0, "Stage", true);
    state.register_target(1, "123", false);
    state.set_original_xy(1, 18.0, -9.0);

    assert_eq!(
        state.sensing_of_by_name(123.0, "x position"),
        ScratchValue::Number(18.0)
    );
    assert_eq!(
        state.sensing_of_by_name(true, "x position"),
        ScratchValue::Number(0.0)
    );
    assert_eq!(
        state.sensing_of_by_name(ScratchValue::Undefined, "x position"),
        ScratchValue::Number(0.0)
    );
}

#[test]
#[ignore = "manual release-mode sensing_of lookup benchmark"]
fn benchmark_static_sensing_of_target_reads() {
    use std::time::Instant;

    let mut state = ScratchState::new(Vec::new());
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite1", false);
    state.register_target(2, "Sprite2", false);
    state.register_target(3, "Sprite3", false);
    state.set_original_xy(2, 123.0, -45.0);

    let reads = 1_000_000;
    let started = Instant::now();
    let mut static_result = ScratchValue::Number(0.0);
    for _ in 0..reads {
        static_result = std::hint::black_box(state.sensing_of_target_slot(2, "x position", None));
    }
    let static_elapsed = started.elapsed();

    let started = Instant::now();
    let mut dynamic_result = ScratchValue::Number(0.0);
    for _ in 0..reads {
        dynamic_result = std::hint::black_box(state.sensing_of_by_name("Sprite2", "x position"));
    }
    let dynamic_elapsed = started.elapsed();

    assert_eq!(static_result, dynamic_result);
    eprintln!("{reads} sensing_of reads: static {static_elapsed:?}, dynamic {dynamic_elapsed:?}");
}
