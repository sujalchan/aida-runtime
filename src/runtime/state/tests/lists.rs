use super::*;

#[test]
fn list_monitor_edits_update_the_owned_stage_or_sprite_list() {
    let mut state = clone_test_state();
    let sprite_instance = state.original_instance(1).unwrap();
    let sprite_key = list_snapshot_key("Sprite", "items", "sprite-list", Some(sprite_instance));
    let stage_key = list_snapshot_key("Stage", "items", "stage-list", None);

    state.apply_input(RuntimeInputEvent::ListAppend {
        key: stage_key.clone(),
    });
    state.apply_input(RuntimeInputEvent::ListSetItem {
        key: stage_key.clone(),
        index: 1,
        value: "shared".to_owned(),
    });
    state.apply_input(RuntimeInputEvent::ListReplace {
        key: stage_key,
        items: vec!["imported".to_owned(), "second".to_owned()],
    });
    state.apply_input(RuntimeInputEvent::ListAppend {
        key: sprite_key.clone(),
    });
    state.apply_input(RuntimeInputEvent::ListSetItem {
        key: sprite_key.clone(),
        index: 2,
        value: "local".to_owned(),
    });
    state.apply_input(RuntimeInputEvent::ListDeleteItem {
        key: sprite_key,
        index: 1,
    });

    assert_eq!(state.list(1).contents(), "imported second");
    state.set_current_instance(sprite_instance);
    assert_eq!(state.list(0).contents(), "local");
}

#[test]
fn stage_numeric_batch_restores_authoritative_list_and_keeps_snapshots() {
    let mut state = ScratchState::with_project_data(
        vec![],
        vec![],
        vec![ScratchList::new(vec!["old".into(), 2.0.into()])],
        vec![ScratchListMetadata::for_target(
            "id", "items", "Stage", 0, true,
        )],
        vec![true],
    );
    let snapshot = state.observable_snapshot(0);
    let result = state.with_stage_numeric_list_batch(0, |state, list| {
        assert_eq!(list.replace_number_and_equals(1.9, 7.0, 7.0), (true, true));
        assert_eq!(
            list.replace_number_and_equals(0.0, 8.0, 8.0),
            (false, false)
        );
        assert_eq!(list.replace_number_and_equals(2.0, 2.0, 2.0), (false, true));
        state.list_changed_result(0, true);
        42
    });
    assert_eq!(result, 42);
    assert_eq!(state.list(0).length(), 2);
    assert_eq!(state.list(0).get(1.0), &ScratchValue::Number(7.0));
    assert_eq!(snapshot.lists[0].items[0], ScratchValue::Text("old".into()));
    assert!(state.presentation_requested());
    let key = list_snapshot_key("Stage", "items", "id", None);
    state.apply_input(RuntimeInputEvent::ListSetItem {
        key,
        index: 1,
        value: "edited".into(),
    });
    assert_eq!(state.list(0).get(1.0), &ScratchValue::Text("edited".into()));
}

#[test]
fn compact_stage_list_snapshots_and_monitor_edits_follow_batch_changes() {
    let mut state = ScratchState::with_project_data(
        vec![],
        vec![],
        vec![ScratchList::new(
            (0..200).map(|n| ScratchValue::Number(n as f64)).collect(),
        )],
        vec![ScratchListMetadata::for_target(
            "id", "items", "Stage", 0, true,
        )],
        vec![true],
    );
    let before = state.observable_snapshot(0);
    state.with_stage_numeric_list_batch(0, |state, list| {
        let (changed, matches) = list.replace_number_and_equals(175.0, 999.0, 999.0);
        assert!(changed && matches);
        state.list_changed_result(0, changed);
    });
    let scrolled_snapshot = state.observable_snapshot(1);
    assert_eq!(before.lists[0].items[174], ScratchValue::Number(174.0));
    assert_eq!(
        scrolled_snapshot.lists[0].items[174],
        ScratchValue::Number(999.0)
    );
    state.with_stage_numeric_list_batch(0, |_, list| {
        assert_eq!(
            list.replace_number_and_equals(175.0, 888.0, 888.0),
            (true, true)
        );
    });
    assert_eq!(
        scrolled_snapshot.lists[0].items[174],
        ScratchValue::Number(999.0)
    );
    let key = list_snapshot_key("Stage", "items", "id", None);
    state.apply_input(RuntimeInputEvent::ListSetItem {
        key,
        index: 175,
        value: "edited".into(),
    });
    assert_eq!(
        state.list(0).get(175.0),
        &ScratchValue::Text("edited".into())
    );
    assert_eq!(state.list(0).length(), 200);
}

#[test]
fn visible_list_changes_dirty_presentation_without_stopping_scripts() {
    let mut state = ScratchState::with_project_data(
        Vec::new(),
        Vec::new(),
        vec![ScratchList::new(vec!["before".into()])],
        vec![ScratchListMetadata::for_target(
            "items", "items", "Stage", 0, true,
        )],
        vec![true],
    );
    state.register_target(0, "Stage", true);
    state.begin_tick();
    let changed = state.list_mut(0).add("after".into());
    state.list_changed_result(0, changed);
    assert!(state.presentation_requested());
    assert!(!state.redraw_requested());
}

#[test]
fn unchanged_or_clone_local_list_mutations_do_not_dirty_the_visible_monitor() {
    let mut state = ScratchState::with_project_data(
        Vec::new(),
        Vec::new(),
        vec![ScratchList::new(vec!["original".into()])],
        vec![ScratchListMetadata::for_target(
            "items", "items", "Sprite1", 1, false,
        )],
        vec![true],
    );
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite1", false);
    let original = state.original_instance(1).unwrap();
    let clone = state.create_clone(original, 1, 300).unwrap();

    state.begin_tick();
    state.set_current_instance(original);
    state.list_changed_result(0, false);
    assert!(!state.presentation_requested());

    state.set_current_instance(clone);
    let changed = state.list_mut(0).add("clone only".into());
    state.list_changed_result(0, changed);
    assert!(!state.presentation_requested());
}
