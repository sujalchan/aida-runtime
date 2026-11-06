use super::*;

#[test]
fn clone_motion_and_visual_state_are_instance_local_and_fenced() {
    let mut state = fenced_state(100.0, 60.0);
    let original = state.current_instance();
    state.set_xy(10.0, 20.0);
    state.set_direction(0.0);
    let clone = state.create_clone(original, 1, 300).unwrap();
    state.set_current_instance(clone);
    state.set_xy(300.0, 0.0);
    state.set_direction(180.0);
    assert_eq!(state.position(original), Some((10.0, 20.0)));
    assert_eq!(state.direction_for_instance(original), Some(0.0));
    assert_eq!(state.position(clone), Some((255.0, 0.0)));
    assert_eq!(state.direction_for_instance(clone), Some(180.0));
    let layers = state.observable_snapshot(0).layers;
    assert_eq!(
        layers
            .iter()
            .find(|layer| layer.instance_id == clone)
            .unwrap()
            .direction,
        180.0
    );
    assert_eq!(layers.len(), 2);
    state.delete_current_clone();
    assert_eq!(state.observable_snapshot(1).layers.len(), 1);
    state.set_current_instance(original);
    let second = state.create_clone(original, 1, 300).unwrap();
    state.remove_clones();
    assert_eq!(state.observable_snapshot(2).layers.len(), 1);
    assert!(!state.instance_is_alive(second));
}

#[test]
fn clones_inherit_local_values_and_lists_but_mutate_independently() {
    let mut state = clone_test_state();
    state.set_current_instance(1);
    state.set(0, 20.0_f64.into());
    state.list_mut(0).add("original".into());

    let first = state.create_clone(1, 1, 300).unwrap();
    state.set_current_instance(first);
    assert_eq!(state.get(0), &ScratchValue::Number(20.0));
    assert_eq!(state.list(0).contents(), "seed original");
    state.set(0, (-4.0_f64).into());
    state.list_mut(0).add("first".into());

    let second = state.create_clone(first, 1, 300).unwrap();
    state.set_current_instance(second);
    assert_eq!(state.get(0), &ScratchValue::Number(-4.0));
    assert_eq!(state.list(0).contents(), "seed original first");
    state.set(0, (-1.0_f64).into());
    state.list_mut(0).add("second".into());
    state.set(1, 99.0_f64.into());

    state.set_current_instance(1);
    assert_eq!(state.get(0), &ScratchValue::Number(20.0));
    assert_eq!(state.list(0).contents(), "seed original");
    assert_eq!(state.get(1), &ScratchValue::Number(99.0));
    state.set_current_instance(first);
    assert_eq!(state.get(0), &ScratchValue::Number(-4.0));
    assert_eq!(state.list(0).contents(), "seed original first");
    state.set_current_instance(second);
    assert_eq!(state.get(0), &ScratchValue::Number(-1.0));
    assert_eq!(state.list(0).contents(), "seed original first second");
    assert_eq!(state.list(1).contents(), "");
}

#[test]
fn clone_positions_inherit_and_then_change_independently() {
    let mut state = ScratchState::new(vec![]);
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite1", false);
    let original = state.original_instance(1).unwrap();
    state.set_original_xy(1, -240.0, 0.0);
    let clone = state.create_clone(original, 1, 300).unwrap();

    assert_eq!(state.position(original), Some((-240.0, 0.0)));
    assert_eq!(state.position(clone), Some((-240.0, 0.0)));

    state.set_current_instance(clone);
    state.set_xy(12.0, 34.0);
    state.change_x_by(3.0);

    assert_eq!(state.position(clone), Some((15.0, 34.0)));
    assert_eq!(state.position(original), Some((-240.0, 0.0)));

    state.set_current_instance(0);
    state.set_xy(99.0, 99.0);
    assert_eq!(state.position(0), Some((0.0, 0.0)));
}

#[test]
fn clone_limit_and_delete_match_scratch_clone_lifecycle() {
    let mut state = clone_test_state();
    assert_eq!(state.create_clone(1, 1, 1), Some(2));
    assert_eq!(state.create_clone(1, 1, 1), None);
    assert_eq!(state.clone_count(), 1);

    state.set_current_instance(1);
    assert!(!state.delete_current_clone());
    assert!(state.instance_is_alive(1));
    state.set_current_instance(2);
    assert!(state.delete_current_clone());
    assert!(!state.instance_is_alive(2));
    assert_eq!(state.clone_count(), 0);

    assert_eq!(state.create_clone(1, 0, 300), None);
}

#[test]
fn standard_clone_limit_is_three_hundred_and_frees_on_delete() {
    let mut state = clone_test_state();
    let mut last = 0;
    for _ in 0..300 {
        last = state.create_clone(1, 1, 300).unwrap();
    }
    assert_eq!(state.clone_count(), 300);
    assert_eq!(state.create_clone(1, 1, 300), None);
    state.set_current_instance(last);
    assert!(state.delete_current_clone());
    assert!(state.create_clone(1, 1, 300).is_some());
    assert_eq!(state.clone_count(), 300);
}

#[test]
fn clones_can_create_children_from_their_own_state() {
    let mut state = clone_test_state();
    state.set_current_instance(1);
    state.set(0, 12.0_f64.into());
    let parent = state.create_clone(1, 1, 300).unwrap();
    state.set_current_instance(parent);
    state.set(0, 24.0_f64.into());
    let child = state.create_clone(parent, 1, 300).unwrap();
    state.set_current_instance(child);
    assert_eq!(state.get(0), &ScratchValue::Number(24.0));
    assert_eq!(state.clone_count(), 2);
}
