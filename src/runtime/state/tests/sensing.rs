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
