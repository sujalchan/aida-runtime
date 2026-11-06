use super::*;

#[test]
fn looks_up_variables_by_id_and_target_scoped_name() {
    let state = ScratchState::with_metadata(
        vec![
            ScratchValue::Number(10.0),
            ScratchValue::Number(20.0),
            ScratchValue::Number(30.0),
            ScratchValue::Boolean(true),
        ],
        vec![
            ScratchVariableMetadata::new("global", "score", "Stage", true),
            ScratchVariableMetadata::new("sprite_a", "score", "SpriteA", false),
            ScratchVariableMetadata::new("sprite_b", "score", "SpriteB", false),
            ScratchVariableMetadata::new("stage_only", "stage_only", "Stage", true),
        ],
    );

    assert_eq!(
        state.get_by_id("sprite_b"),
        Some(&ScratchValue::Number(30.0))
    );
    assert_eq!(
        state.get_by_name("SpriteA", "score"),
        Some(&ScratchValue::Number(20.0))
    );
    assert_eq!(
        state.get_by_name("SpriteB", "score"),
        Some(&ScratchValue::Number(30.0))
    );
    assert_eq!(
        state.get_by_name("SpriteA", "stage_only"),
        Some(&ScratchValue::Boolean(true))
    );
}

#[test]
fn deferred_variable_writes_require_unobserved_slots() {
    let mut state = ScratchState::with_project_data(
        vec![0.0.into(), 0.0.into()],
        vec![
            ScratchVariableMetadata::for_target("hidden", "hidden", "Stage", 0, true)
                .with_monitor_visible(false),
            ScratchVariableMetadata::for_target("visible", "visible", "Stage", 0, true)
                .with_monitor_visible(true),
        ],
        vec![],
        vec![],
        vec![],
    );
    assert!(state.can_defer_variable_writes(&[0]));
    assert!(!state.can_defer_variable_writes(&[0, 1]));
    assert!(!state.can_defer_variable_writes(&[2]));
    let (sender, _receiver) = std::sync::mpsc::channel();
    state.set_event_sink(Some(sender));
    assert!(!state.can_defer_variable_writes(&[0]));
}

#[test]
fn existing_local_number_preserves_events_monitor_and_clone_scope() {
    let mut state = ScratchState::with_metadata(
        vec![ScratchValue::Text("initial".into())],
        vec![
            ScratchVariableMetadata::for_target("local", "score", "Sprite", 1, false)
                .with_monitor_visible(true),
        ],
    );
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite", false);
    let original = state.original_instance(1).unwrap();
    state.set_current_instance(original);
    // The first nonnumeric write uses the general fallback.
    state.set_existing_local_number(0, 2.0);
    assert_eq!(state.get(0), &ScratchValue::Number(2.0));
    state.presentation_consumed();
    state.set_existing_local_number(0, 3.0);
    assert_eq!(state.get(0), &ScratchValue::Number(3.0));
    assert!(state.presentation_requested());
    let clone = state.create_clone(original, 1, 300).unwrap();
    state.set_current_instance(clone);
    state.presentation_consumed();
    state.set_existing_local_number(0, 4.0);
    assert!(!state.presentation_requested());
    assert_eq!(state.get(0), &ScratchValue::Number(4.0));
    state.set_current_instance(original);
    assert_eq!(state.get(0), &ScratchValue::Number(3.0));
    let (tx, rx) = std::sync::mpsc::channel();
    state.set_event_sink(Some(tx));
    state.set_existing_local_number(0, 5.0);
    assert_eq!(state.get(0), &ScratchValue::Number(5.0));
    assert!(matches!(
        rx.try_recv(),
        Ok(ObservableEvent::VariableChange { .. })
    ));
}

#[test]
fn shared_variable_fast_access_ignores_clone_local_storage() {
    let mut state = clone_test_state();
    let clone = state.create_clone(1, 1, 300).unwrap();
    state.set_current_instance(clone);

    state.set_shared(1, 17.0_f64.into());

    assert_eq!(state.get_shared(1), &ScratchValue::Number(17.0));
    assert_eq!(state.get(1), &ScratchValue::Number(17.0));
}

#[test]
fn shared_change_preserves_scratch_numeric_conversion() {
    let mut state = ScratchState::new(vec![
        ScratchValue::Text(" 2.5 ".to_owned()),
        ScratchValue::Boolean(true),
        ScratchValue::Number(f64::NAN),
        ScratchValue::Number(f64::INFINITY),
        ScratchValue::Number(-0.0),
    ]);

    state.change_shared(0, scratch_to_number(1.5));
    state.change_shared(1, scratch_to_number("2"));
    state.change_shared(2, scratch_to_number(1.0));
    state.change_shared(3, scratch_to_number(f64::NEG_INFINITY));
    state.change_shared(4, scratch_to_number(-0.0));

    assert_eq!(state.get_shared(0), &ScratchValue::Number(4.0));
    assert_eq!(state.get_shared(1), &ScratchValue::Number(3.0));
    assert_eq!(state.get_shared(2), &ScratchValue::Number(1.0));
    assert!(matches!(state.get_shared(3), ScratchValue::Number(value) if value.is_nan()));
    assert!(matches!(state.get_shared(4), ScratchValue::Number(value) if value.is_sign_negative()));
}

// exercise the compiled numeric change path, including its first dynamic-value fallback.
#[test]
fn numeric_change_fast_paths_preserve_scope_and_monitor_updates() {
    let mut state = ScratchState::with_metadata(
        vec![
            ScratchValue::Text(" 2.5 ".to_owned()),
            ScratchValue::Text("3".to_owned()),
        ],
        vec![
            ScratchVariableMetadata::for_target("stage", "shared", "Stage", 0, true)
                .with_monitor_visible(true),
            ScratchVariableMetadata::for_target("local", "score", "Sprite", 1, false)
                .with_monitor_visible(true),
        ],
    );
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite", false);
    let original = state.original_instance(1).unwrap();
    state.begin_tick();

    state.change_shared_number(0, 1.0);
    assert_eq!(state.get_shared(0), &ScratchValue::Number(3.5));
    assert!(state.presentation_requested());
    state.presentation_consumed();
    state.change_shared_number(0, 1.0);
    assert_eq!(state.get_shared(0), &ScratchValue::Number(4.5));
    assert!(state.presentation_requested());

    state.set_current_instance(original);
    state.change_local_number(1, 2.0);
    assert_eq!(state.get(1), &ScratchValue::Number(5.0));
    let clone = state.create_clone(original, 1, 300).unwrap();
    state.set_current_instance(clone);
    state.change_local_number(1, 4.0);
    assert_eq!(state.get(1), &ScratchValue::Number(9.0));
    state.set_current_instance(original);
    assert_eq!(state.get(1), &ScratchValue::Number(5.0));
}

#[test]
fn variable_change_updates_local_instances_independently_and_shares_stage_values() {
    let mut state = clone_test_state();
    let original = state.original_instance(1).unwrap();
    state.set_current_instance(original);
    state.change_local_by(0, 2.0);
    let clone = state.create_clone(original, 1, 300).unwrap();

    state.set_current_instance(original);
    state.change_local_by(0, 3.0);
    state.set_current_instance(clone);
    state.change_local_by(0, 5.0);
    state.change_shared(1, 4.0);

    assert_eq!(state.get(0), &ScratchValue::Number(17.0));
    assert_eq!(state.get_shared(1), &ScratchValue::Number(7.0));
    let snapshot = state.observable_snapshot(1);
    assert!(snapshot.variables.iter().any(|variable| {
        variable.key == format!("Sprite::score#sprite-local@{clone}")
            && variable.value == ScratchValue::Number(17.0)
    }));
    state.set_current_instance(original);
    assert_eq!(state.get(0), &ScratchValue::Number(15.0));
    assert_eq!(state.get(1), &ScratchValue::Number(7.0));
}

#[test]
fn local_variable_change_preserves_scratch_numeric_conversion() {
    let mut state = ScratchState::with_metadata(
        vec![ScratchValue::Text(" 2.5 ".to_owned())],
        vec![ScratchVariableMetadata::new(
            "local", "score", "Sprite", false,
        )],
    );
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite", false);
    state.set_current_instance(state.original_instance(1).unwrap());

    state.change_local_by(0, 1.5);

    assert_eq!(state.get(0), &ScratchValue::Number(4.0));
}
