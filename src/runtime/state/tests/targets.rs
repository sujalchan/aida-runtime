use super::*;

#[test]
fn empty_backdrop_input_keeps_current_backdrop_and_starts_its_hat() {
    let mut state = ScratchState::new(Vec::new());
    state.set_backdrops(vec!["one".to_owned(), "two".to_owned()], 1);
    assert_eq!(state.observable_snapshot(0).backdrop_name, "two");
    state.begin_tick();

    state.switch_backdrop("");

    assert_eq!(
        state.backdrop_number_name(false),
        ScratchValue::Text("two".to_owned())
    );
    assert_eq!(state.backdrop_number_name(true), ScratchValue::Number(2.0));
    assert_eq!(state.take_pending_backdrop_events(), vec!["two"]);
    assert!(!state.redraw_requested());
    assert_eq!(state.observable_snapshot(1).backdrop_name, "two");
}

#[test]
fn backdrop_name_takes_priority_over_special_values_and_events_fire_without_change() {
    let mut state = ScratchState::new(Vec::new());
    state.set_backdrops(vec!["first".to_owned(), "next backdrop".to_owned()], 0);

    state.begin_tick();
    state.switch_backdrop("first");
    assert!(state.redraw_requested());
    assert_eq!(state.take_pending_backdrop_events(), vec!["first"]);

    state.switch_backdrop("next backdrop");

    assert_eq!(
        state.backdrop_number_name(false),
        ScratchValue::Text("next backdrop".to_owned())
    );
    assert_eq!(state.take_pending_backdrop_events(), vec!["next backdrop"]);
    state.switch_backdrop("next backdrop");
    assert_eq!(state.backdrop_number_name(true), ScratchValue::Number(2.0));
    assert_eq!(state.take_pending_backdrop_events(), vec!["next backdrop"]);
    assert_eq!(state.observable_snapshot(1).backdrop_name, "next backdrop");
}

#[test]
fn backdrop_numeric_inputs_and_special_names_follow_scratch_indexing() {
    let mut state = ScratchState::new(Vec::new());
    state.set_backdrops(
        vec!["first".to_owned(), "middle".to_owned(), "last".to_owned()],
        0,
    );

    state.switch_backdrop(2.0_f64);
    assert_eq!(
        state.backdrop_number_name(false),
        ScratchValue::Text("middle".to_owned())
    );
    state.switch_backdrop("3");
    assert_eq!(
        state.backdrop_number_name(false),
        ScratchValue::Text("last".to_owned())
    );
    state.switch_backdrop("previous backdrop");
    assert_eq!(
        state.backdrop_number_name(false),
        ScratchValue::Text("middle".to_owned())
    );
    state.switch_backdrop("next backdrop");
    assert_eq!(
        state.backdrop_number_name(false),
        ScratchValue::Text("last".to_owned())
    );
    state.switch_backdrop("random backdrop");
    assert_ne!(
        state.backdrop_number_name(false),
        ScratchValue::Text("last".to_owned())
    );
}

#[test]
fn sprite_fencing_uses_costume_bounds_at_all_four_stage_edges() {
    let mut state = fenced_state(100.0, 60.0);
    state.set_xy(300.0, 0.0);
    assert_eq!(state.position(state.current_instance()), Some((275.0, 0.0)));
    state.set_xy(-300.0, 0.0);
    assert_eq!(
        state.position(state.current_instance()),
        Some((-275.0, 0.0))
    );
    state.set_xy(0.0, 250.0);
    assert_eq!(state.position(state.current_instance()), Some((0.0, 195.0)));
    state.set_xy(0.0, -250.0);
    assert_eq!(
        state.position(state.current_instance()),
        Some((0.0, -195.0))
    );
}

#[test]
fn sprite_fencing_uses_reduced_margin_for_small_costumes_and_rotation() {
    let mut small = fenced_state(10.0, 8.0);
    small.set_xy(300.0, 0.0);
    assert_eq!(small.position(small.current_instance()), Some((241.0, 0.0)));

    let mut rotated = fenced_state(100.0, 60.0);
    rotated.set_direction(0.0);
    rotated.set_xy(300.0, 250.0);
    assert_eq!(
        rotated.position(rotated.current_instance()),
        Some((255.0, 215.0))
    );
}

#[test]
fn motion_set_x_and_y_use_scratch_numbers_and_preserve_the_other_axis() {
    let mut state = fenced_state(100.0, 60.0);
    state.set_xy(7.0, 8.0);
    let (_, y) = state.position(state.current_instance()).unwrap();
    state.set_xy(scratch_to_number("12"), y);
    assert_eq!(state.position(state.current_instance()), Some((12.0, 8.0)));
    let (x, _) = state.position(state.current_instance()).unwrap();
    state.set_xy(x, scratch_to_number("not a number"));
    assert_eq!(state.position(state.current_instance()), Some((12.0, 0.0)));

    let original = state.current_instance();
    let clone = state.create_clone(original, 1, 300).unwrap();
    state.set_current_instance(clone);
    let (x, y) = state.position(clone).unwrap();
    state.set_xy(x, scratch_to_number("-4"));
    assert_eq!(state.position(clone), Some((12.0, -4.0)));
    assert_eq!(state.position(original), Some((12.0, 0.0)));
    assert_eq!(y, 0.0);
}

// move steps follows stored scratch direction and the same fenced position path.
#[test]
fn move_steps_uses_scratch_heading_and_keeps_target_instance_identity() {
    let mut state = fenced_state(40.0, 30.0);
    let original = state.current_instance();
    state.move_steps(10.0);
    assert_eq!(state.position(original), Some((10.0, 0.0)));
    state.set_direction(0.0);
    state.move_steps(10.0);
    assert_eq!(state.position(original), Some((10.0, 10.0)));

    let clone = state.create_clone(original, 1, 300).unwrap();
    state.set_current_instance(clone);
    state.set_direction(-90.0);
    state.move_steps(5.0);
    assert_eq!(state.position(clone), Some((5.0, 10.0)));
    assert_eq!(state.position(original), Some((10.0, 10.0)));
}

// edge bounce uses the transformed costume outline and fences the resulting pose.
#[test]
fn edge_bounce_reflects_from_the_nearest_contact_and_keeps_style_state() {
    let mut state = fenced_state(20.0, 20.0);
    let target = state.instances[state.current_instance].target;
    state.set_target_fence_geometry(
        target,
        SpriteFenceGeometry {
            width: 20.0,
            height: 20.0,
            rotation_center_x: 10.0,
            rotation_center_y: 10.0,
            bounds_hull: Some(vec![[0.5, 0.5], [19.5, 0.5], [19.5, 19.5], [0.5, 19.5]]),
        },
    );
    state.set_rotation_style("left-right");
    state.set_direction(0.0);
    state.set_xy(0.0, 180.0);
    state.bounce_on_edge();
    assert_eq!(state.direction(), 180.0);
    assert_eq!(state.position(state.current_instance()), Some((0.0, 171.0)));
    let layer = state.observable_snapshot(0).layers.pop().unwrap();
    assert_eq!(layer.rotation_style, ScratchRotationStyle::LeftRight);
}

// report coordinates at scratch's integer precision without altering stored doubles.
#[test]
fn motion_reporters_round_only_values_within_scratch_precision() {
    let mut state = fenced_state(20.0, 20.0);
    let instance = state.current_instance();
    state.set_xy(12.0000000001, -4.125);
    assert_eq!(state.position(instance), Some((12.0000000001, -4.125)));
    assert_eq!(state.x_position(), 12.0);
    assert_eq!(state.y_position(), -4.125);
}

// reuse rotated costume bounds until the sprite transform or costume changes.
#[test]
fn fence_offsets_reuse_and_invalidate_cached_transform_geometry() {
    let mut state = ScratchState::new(Vec::new());
    let first_geometry = SpriteFenceGeometry {
        width: 120.0,
        height: 80.0,
        rotation_center_x: 60.0,
        rotation_center_y: 40.0,
        bounds_hull: None,
    };
    state.set_target_fence_geometry(0, first_geometry.clone());
    state.set_xy(20.0, 30.0);
    let original_cache = state.instances[0].fence_offsets.unwrap();

    state.set_xy(25.0, 35.0);
    assert_eq!(
        state.instances[0].fence_offsets.unwrap().points,
        original_cache.points
    );

    state.set_direction(0.0);
    state.set_xy(25.0, 35.0);
    let rotated_cache = state.instances[0].fence_offsets.unwrap();
    assert_eq!(rotated_cache.direction, 0.0);
    assert_ne!(rotated_cache.points, original_cache.points);

    let second_geometry = SpriteFenceGeometry {
        width: 200.0,
        ..first_geometry.clone()
    };
    state.set_target_fence_geometry(0, second_geometry.clone());
    state.set_xy(25.0, 35.0);
    assert_eq!(
        state.instances[0].fence_offsets.unwrap().geometry,
        [
            second_geometry.width,
            second_geometry.height,
            second_geometry.rotation_center_x,
            second_geometry.rotation_center_y,
        ]
    );
}

#[test]
fn motion_updates_are_visible_in_sprite_layer_snapshots() {
    let mut state = ScratchState::new(Vec::new());
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite1", false);
    state.set_current_instance(state.original_instance(1).unwrap());

    state.set_xy(0.0, 0.0);
    state.change_x_by(10.0);
    state.change_y_by(10.0);
    state.request_redraw();

    let snapshot = state.observable_snapshot(3);
    assert_eq!(snapshot.layers.len(), 1);
    assert_eq!((snapshot.layers[0].x, snapshot.layers[0].y), (10.0, 10.0));
    assert!(state.redraw_requested());
}

#[test]
fn keyboard_reporter_uses_the_executing_instances_current_costume_name() {
    let mut state = ScratchState::new(Vec::new());
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite1", false);
    state.set_target_costumes(1, vec!["w".to_owned(), "other".to_owned()], 0);
    let original = state.original_instance(1).unwrap();
    let clone = state.create_clone(original, 1, 300).unwrap();
    state.apply_input(RuntimeInputEvent::Key {
        name: "W".to_owned(),
        pressed: true,
    });

    state.set_current_instance(original);
    assert!(state.key_pressed(state.costume_number_name(false)));
    state.set_current_instance(clone);
    assert!(state.key_pressed(state.costume_number_name(false)));

    state.switch_costume_to(ScratchValue::Text("other".to_owned()));
    assert!(!state.key_pressed(state.costume_number_name(false)));
    state.set_current_instance(original);
    assert!(state.key_pressed(state.costume_number_name(false)));
}

fn sized_costume_state(costumes: Vec<(String, Option<[f64; 2]>)>) -> ScratchState {
    let mut state = ScratchState::new(Vec::new());
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite1", false);
    state.set_target_costumes_with_sizes(1, costumes, 0);
    state.set_current_instance(state.original_instance(1).unwrap());
    state
}

fn stored_size(state: &ScratchState) -> f64 {
    state.instances[state.current_instance].size
}

#[test]
fn costume_size_matches_scratch_bounds_and_keeps_fractional_storage() {
    let mut state = sized_costume_state(vec![("costume1".to_owned(), Some([61.49876, 117.72954]))]);

    state.set_size(-100.0);
    let minimum = stored_size(&state);
    assert!((minimum - 8.1302452277).abs() < 1e-9);
    state.change_size_by(10.0);
    assert!((stored_size(&state) - (minimum + 10.0)).abs() < 1e-12);
    assert_eq!(state.size_reporter(), 18.0);

    state.set_size(1.0e21);
    assert!((stored_size(&state) - 458.6784251429).abs() < 1e-9);
    state.change_size_by(-10.0);
    assert_eq!(state.size_reporter(), 449.0);
}

#[test]
fn costume_dimensions_select_width_and_height_minimum_and_maximum_bounds() {
    let mut state = sized_costume_state(vec![
        ("width minimum".to_owned(), Some([10.0, 20.0])),
        ("height minimum".to_owned(), Some([20.0, 10.0])),
        ("width maximum".to_owned(), Some([100.0, 10.0])),
        ("height maximum".to_owned(), Some([10.0, 100.0])),
    ]);

    state.set_size(-100.0);
    assert_eq!(stored_size(&state), 50.0);
    state.switch_costume_to("height minimum".into());
    state.set_size(-100.0);
    assert_eq!(stored_size(&state), 50.0);
    state.switch_costume_to("width maximum".into());
    state.set_size(f64::INFINITY);
    assert_eq!(stored_size(&state), 720.0);
    state.switch_costume_to("height maximum".into());
    state.set_size(f64::INFINITY);
    assert_eq!(stored_size(&state), 540.0);
}

#[test]
fn costume_switch_preserves_size_then_clamps_against_the_new_costume() {
    let mut state = sized_costume_state(vec![
        ("small".to_owned(), Some([10.0, 20.0])),
        ("large".to_owned(), Some([100.0, 100.0])),
    ]);
    state.set_size(400.0);
    assert_eq!(stored_size(&state), 400.0);

    state.switch_costume_to("large".into());
    assert_eq!(stored_size(&state), 400.0);
    state.set_size(1.0e21);
    assert_eq!(stored_size(&state), 540.0);
}

#[test]
fn costume_switch_uses_scratch_numeric_rounding_and_ignores_unknown_choices() {
    let mut state = sized_costume_state(vec![
        ("one".to_owned(), Some([100.0, 100.0])),
        ("two".to_owned(), Some([100.0, 100.0])),
    ]);

    state.switch_costume_to(1.5.into());
    assert_eq!(state.costume_number_name(true), ScratchValue::Number(2.0));
    state.switch_costume_to("0.5".into());
    assert_eq!(state.costume_number_name(true), ScratchValue::Number(1.0));
    state.switch_costume_to("unknown costume".into());
    assert_eq!(state.costume_number_name(true), ScratchValue::Number(1.0));
    state.switch_costume_to(true.into());
    assert_eq!(state.costume_number_name(true), ScratchValue::Number(1.0));
    state.switch_costume_to("next costume".into());
    assert_eq!(state.costume_number_name(true), ScratchValue::Number(2.0));
    state.switch_costume_to(f64::INFINITY.into());
    assert_eq!(state.costume_number_name(true), ScratchValue::Number(1.0));
}

#[test]
fn next_costume_cycles_and_clone_size_and_costume_state_are_independent() {
    let mut state = sized_costume_state(vec![
        ("small".to_owned(), Some([10.0, 20.0])),
        ("large".to_owned(), Some([100.0, 100.0])),
    ]);
    let original = state.current_instance();
    state.set_size(400.0);
    let clone = state.create_clone(original, 1, 300).unwrap();
    assert_eq!(state.instances[clone].size, 400.0);

    state.set_current_instance(clone);
    state.set_size(-100.0);
    assert_eq!(stored_size(&state), 50.0);
    state.next_costume();
    assert_eq!(state.costume_number_name(true), ScratchValue::Number(2.0));
    assert_eq!(
        state.costume_number_name(false),
        ScratchValue::Text("large".to_owned())
    );
    state.set_size(1.0e21);
    assert_eq!(stored_size(&state), 540.0);

    state.set_current_instance(original);
    assert_eq!(stored_size(&state), 400.0);
    assert_eq!(
        state.costume_number_name(false),
        ScratchValue::Text("small".to_owned())
    );
}

#[test]
fn size_inputs_follow_scratch_number_conversion_and_reporter_rounding() {
    let mut state = sized_costume_state(vec![("bitmap".to_owned(), Some([100.0, 100.0]))]);

    state.set_size("25.5");
    assert_eq!(stored_size(&state), 25.5);
    assert_eq!(state.size_reporter(), 26.0);
    state.set_size("0x10");
    assert_eq!(stored_size(&state), 16.0);
    state.set_size(25.499);
    assert_eq!(state.size_reporter(), 25.0);

    for input in ["", "not a number"] {
        state.set_size(input);
        assert_eq!(stored_size(&state), 5.0);
    }
    state.set_size(true);
    assert_eq!(stored_size(&state), 5.0);
    state.set_size(false);
    assert_eq!(stored_size(&state), 5.0);
    state.set_size(f64::NAN);
    assert_eq!(stored_size(&state), 5.0);
    state.set_size(f64::NEG_INFINITY);
    assert_eq!(stored_size(&state), 5.0);
    state.set_size(f64::INFINITY);
    assert_eq!(stored_size(&state), 540.0);
    state.set_size("Infinity");
    assert_eq!(stored_size(&state), 540.0);
    state.set_size("-Infinity");
    assert_eq!(stored_size(&state), 5.0);
    state.set_size(-1.0e21);
    assert_eq!(stored_size(&state), 5.0);
}

#[test]
fn stage_and_missing_costume_metadata_do_not_change_size() {
    let mut state = sized_costume_state(vec![("unknown".to_owned(), None)]);
    state.set_size(250.0);
    assert_eq!(stored_size(&state), 250.0);

    state.set_current_instance(state.original_instance(0).unwrap());
    state.set_size(250.0);
    state.change_size_by(-50.0);
    assert_eq!(stored_size(&state), 100.0);
}

#[test]
fn zero_sized_costume_uses_scratch_zero_dimension_limits() {
    let mut state = sized_costume_state(vec![("empty".to_owned(), Some([0.0, 0.0]))]);
    state.set_size(-100.0);
    assert_eq!(stored_size(&state), 100.0);
    state.set_size(1000.0);
    assert_eq!(stored_size(&state), 1000.0);
}

#[test]
fn hidden_sprite_motion_updates_position_without_a_sprite_redraw() {
    let mut state = ScratchState::new(Vec::new());
    state.register_target(0, "Sprite1", false);
    state.set_original_visual(0, 90.0, 100.0, false);
    state.begin_tick();

    state.set_xy(12.0, -7.0);
    assert_eq!(state.position(state.current_instance()), Some((12.0, -7.0)));
    assert!(!state.redraw_requested());
    assert!(state.presentation_requested());
    let hidden = state.observable_snapshot(0);
    assert_eq!(hidden.layers.len(), 1);
    assert!(!hidden.layers[0].visible);
    assert_eq!((hidden.layers[0].x, hidden.layers[0].y), (12.0, -7.0));

    state.presentation_consumed();
    state.pen_down();
    state.set_xy(24.0, -14.0);
    assert!(state.redraw_requested());
    assert!(
        matches!(state.take_pending_pen_commands().last(), Some(PenCommand::Strokes(strokes))
            if strokes.last().is_some_and(|stroke| stroke.start == [12.0, -7.0] && stroke.end == [24.0, -14.0]))
    );
    let hidden = state.observable_snapshot(1);
    assert_eq!(hidden.layers.len(), 1);
    assert_eq!((hidden.layers[0].x, hidden.layers[0].y), (24.0, -14.0));

    state.set_visible(true);
    let snapshot = state.observable_snapshot(2);
    assert_eq!(
        snapshot.layers.last().map(|layer| (layer.x, layer.y)),
        Some((24.0, -14.0))
    );
}
