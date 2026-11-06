use super::*;

#[test]
fn observable_events_and_snapshots_preserve_scoped_identity() {
    let mut state = clone_test_state();
    let (sender, receiver) = std::sync::mpsc::channel();
    state.set_event_sink(Some(sender));
    state.set_current_instance(1);
    state.set(0, ScratchValue::Text("changed".to_owned()));
    state.emit_observable_output("say", &"hello");
    let clone = state.create_clone(1, 1, 300).unwrap();
    let snapshot = state.observable_snapshot(7);

    assert!(
        matches!(receiver.try_recv(), Ok(ObservableEvent::VariableChange { key, .. }) if key == "Sprite::score#sprite-local@1")
    );
    assert_eq!(
        receiver.try_recv().unwrap(),
        ObservableEvent::Output {
            label: "say".to_owned(),
            value: "\"hello\"".to_owned()
        }
    );
    assert!(
        snapshot
            .variables
            .iter()
            .any(|value| value.key == "Sprite::score#sprite-local@1"
                && value.value == ScratchValue::Text("changed".to_owned()))
    );
    assert!(
        snapshot
            .variables
            .iter()
            .any(|value| value.key == format!("Sprite::score#sprite-local@{clone}"))
    );
    assert!(
        snapshot
            .layers
            .iter()
            .any(|layer| layer.instance_id == clone && !layer.original)
    );
    assert_eq!(snapshot.tick, 7);
}

#[test]
fn speech_bubbles_are_instance_local_and_expire_at_their_deadline() {
    let mut state = ScratchState::new(Vec::new());
    state.register_target(1, "Sprite1", false);
    let original = state.original_instance(1).unwrap();
    state.set_current_instance(original);
    state.set_speech_bubble(BubbleKind::Say, "hello".to_owned(), Some(100));
    let clone = state.create_clone(original, 1, 300).unwrap();

    state.set_current_instance(clone);
    state.set_speech_bubble(BubbleKind::Think, "hmm".to_owned(), None);
    let snapshot = state.observable_snapshot(0);
    assert_eq!(snapshot.layers[0].bubble.as_ref().unwrap().text, "hello");
    assert_eq!(snapshot.layers[1].bubble.as_ref().unwrap().text, "hmm");

    state.expire_speech_bubbles(99);
    assert!(state.observable_snapshot(1).layers[0].bubble.is_some());
    state.expire_speech_bubbles(100);
    let snapshot = state.observable_snapshot(2);
    assert!(snapshot.layers[0].bubble.is_none());
    assert_eq!(snapshot.layers[1].bubble.as_ref().unwrap().text, "hmm");
}

// renderer snapshots omit hidden list items so their shared storage stays reusable.
#[test]
fn renderer_snapshots_do_not_retain_hidden_list_storage() {
    let capacity = 172_800;
    let mut state = ScratchState::with_project_data(
        Vec::new(),
        Vec::new(),
        vec![ScratchList::new(
            (0..capacity)
                .map(|value| ScratchValue::Number(value as f64))
                .collect(),
        )],
        vec![ScratchListMetadata::for_target(
            "stage-list",
            "number",
            "Stage",
            0,
            true,
        )],
        vec![false],
    );
    let original_capacity = state.list(0).shared_items().capacity();
    let snapshot = state.observable_snapshot_filtered(1, false);
    assert!(snapshot.lists[0].items.is_empty());
    assert_eq!(Arc::strong_count(&state.list(0).shared_items()), 2);
    assert!(state.list_mut(0).delete_all());
    assert_eq!(state.list(0).shared_items().capacity(), original_capacity);

    let mut next_item = 0.0;
    state.list_mut(0).add_generated_numbers(capacity, || {
        let value = next_item;
        next_item += 1.0;
        value
    });
    assert_eq!(state.list(0).length(), capacity);
    assert_eq!(state.list(0).get(1.0), &ScratchValue::Number(0.0));
    assert_eq!(
        state.list(0).get((capacity / 2 + 1) as f64),
        &ScratchValue::Number((capacity / 2) as f64)
    );
    assert_eq!(
        state.list(0).get(capacity as f64),
        &ScratchValue::Number((capacity - 1) as f64)
    );

    state.set_list_visible(0, true);
    let visible_snapshot = state.observable_snapshot(2);
    assert!(visible_snapshot.lists[0].visible);
    assert_eq!(visible_snapshot.lists[0].items.len(), capacity);
    assert_eq!(
        visible_snapshot.lists[0].items[0],
        ScratchValue::Number(0.0)
    );
    assert_eq!(
        visible_snapshot.lists[0].items[capacity / 2],
        ScratchValue::Number((capacity / 2) as f64)
    );
    assert_eq!(
        visible_snapshot.lists[0].items[capacity - 1],
        ScratchValue::Number((capacity - 1) as f64)
    );
}

#[test]
fn renderer_snapshot_skips_hidden_compact_items_until_visible() {
    let mut state = ScratchState::with_project_data(
        vec![],
        vec![],
        vec![ScratchList::new(vec![1.0.into(), 2.0.into()])],
        vec![ScratchListMetadata::for_target(
            "id", "items", "Stage", 0, true,
        )],
        vec![false],
    );
    state.list_mut(0).with_numeric_replacements(|batch| {
        batch.replace_number_and_equals(1.0, 7.0, 7.0);
    });
    assert!(!state.list(0).has_materialized_numeric_items());
    let hidden = state.observable_snapshot_filtered(1, false);
    assert!(!hidden.lists[0].visible);
    assert!(hidden.lists[0].items.is_empty());
    assert!(!state.list(0).has_materialized_numeric_items());
    let full = state.observable_snapshot(2);
    assert_eq!(full.lists[0].items.as_slice(), &[7.0.into(), 2.0.into()]);
    state.set_list_visible(0, true);
    let visible = state.observable_snapshot_filtered(3, false);
    assert!(visible.lists[0].visible);
    assert_eq!(visible.lists[0].items.as_slice(), &[7.0.into(), 2.0.into()]);
}

#[test]
fn only_changed_visible_state_requests_a_redraw() {
    let mut state = ScratchState::with_metadata(
        vec![1.0_f64.into(), 1.0_f64.into()],
        vec![
            ScratchVariableMetadata::for_target("visible", "visible", "Stage", 0, true)
                .with_monitor_visible(true),
            ScratchVariableMetadata::for_target("hidden", "hidden", "Stage", 0, true),
        ],
    );
    state.register_target(0, "Stage", true);
    state.begin_tick();
    state.set_shared(1, 2.0_f64.into());
    assert!(!state.redraw_requested());
    assert!(!state.presentation_requested());
    state.set_shared(0, 1.0_f64.into());
    assert!(!state.redraw_requested());
    assert!(!state.presentation_requested());
    state.set_shared(0, 2.0_f64.into());
    assert!(!state.redraw_requested());
    assert!(state.presentation_requested());
}

#[test]
fn unchanged_sprite_positions_and_bubbles_do_not_request_redraw() {
    let mut state = ScratchState::new(Vec::new());
    state.register_target(0, "Sprite1", false);
    state.begin_tick();
    state.set_xy(0.0, 0.0);
    assert!(!state.redraw_requested());
    state.set_speech_bubble(BubbleKind::Say, "hello".to_owned(), None);
    assert!(state.redraw_requested());
    state.presentation_consumed();
    state.set_xy(0.0, 0.0);
    state.set_speech_bubble(BubbleKind::Say, "hello".to_owned(), None);
    assert!(!state.redraw_requested());
    state.change_x_by(1.0);
    assert!(state.redraw_requested());
}
