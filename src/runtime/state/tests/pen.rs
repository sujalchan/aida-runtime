use super::*;

#[test]
fn pen_commands_preserve_points_strokes_stamps_and_clear_order() {
    let mut state = ScratchState::new(Vec::new());
    state.register_target(0, "Sprite1", false);
    state.set_original_xy(0, 10.0, 20.0);
    state.set_target_costumes(0, vec!["one".to_owned(), "two".to_owned()], 0);
    state.set_pen_rgb_color("#00ff00");
    state.set_pen_size(6.0);
    state.pen_down();
    state.set_xy(30.0, 40.0);
    state.switch_costume_to(ScratchValue::Text("two".to_owned()));
    state.stamp();
    state.clear_pen();
    state.set_xy(50.0, 60.0);
    let commands = state.take_pending_pen_commands();

    assert!(matches!(
        commands.as_slice(),
        [
            PenCommand::Dots(points),
            PenCommand::Strokes(first),
            PenCommand::Stamp { target_name, position: [30.0, 40.0], costume_index: 1, .. },
            PenCommand::Clear,
            PenCommand::Strokes(last)
        ] if points == &[PenDot { position: [10.0, 20.0], color: [0.0, 1.0, 0.0, 1.0], diameter: 6.0, pixel_offset: 0.0 }] && first == &[
            PenStroke { start: [10.0, 20.0], end: [30.0, 40.0], color: [0.0, 1.0, 0.0, 1.0], diameter: 6.0, pixel_offset: 0.0 }
        ] && target_name == "Sprite1" && last == &[
            PenStroke { start: [30.0, 40.0], end: [50.0, 60.0], color: [0.0, 1.0, 0.0, 1.0], diameter: 6.0, pixel_offset: 0.0 }
        ]
    ));
}

#[test]
fn clones_copy_pen_state_and_keep_later_changes_independent() {
    let mut state = ScratchState::new(Vec::new());
    state.register_target(0, "Sprite1", false);
    state.set_target_costumes(0, vec!["costume1".to_owned(), "Apple".to_owned()], 1);
    state.set_pen_rgb_color("#00ff00");
    state.set_pen_size(6.0);
    state.pen_down();
    let clone = state.create_clone(0, 0, 300).unwrap();
    state.set_current_instance(clone);
    state.set_xy(5.0, 7.0);
    state.set_pen_rgb_color("#ff0000");
    state.set_pen_size(2.0);
    state.set_xy(8.0, 9.0);
    state.set_current_instance(0);
    state.set_xy(2.0, 3.0);
    let commands = state.take_pending_pen_commands();

    assert!(
        matches!(commands.as_slice(), [PenCommand::Dots(points), PenCommand::Strokes(strokes)] if points == &[
            PenDot { position: [0.0, 0.0], color: [0.0, 1.0, 0.0, 1.0], diameter: 6.0, pixel_offset: 0.0 }
        ] && strokes == &[
            PenStroke { start: [0.0, 0.0], end: [5.0, 7.0], color: [0.0, 1.0, 0.0, 1.0], diameter: 6.0, pixel_offset: 0.0 },
            PenStroke { start: [5.0, 7.0], end: [8.0, 9.0], color: [1.0, 0.0, 0.0, 1.0], diameter: 2.0, pixel_offset: 0.0 },
            PenStroke { start: [0.0, 0.0], end: [2.0, 3.0], color: [0.0, 1.0, 0.0, 1.0], diameter: 6.0, pixel_offset: 0.0 }
        ])
    );
    assert_eq!(state.instances[clone].costume_index, 1);
}

#[test]
fn alternating_pen_marks_preserve_order_through_batching() {
    let mut state = ScratchState::new(Vec::new());
    let dot = PenDot {
        position: [1.0, 2.0],
        color: [0.2, 0.3, 0.4, 0.5],
        diameter: 3.0,
        pixel_offset: 0.0,
    };
    let stroke = PenStroke {
        start: [1.0, 2.0],
        end: [3.0, 4.0],
        color: dot.color,
        diameter: dot.diameter,
        pixel_offset: 0.0,
    };
    for _ in 0..20 {
        state.push_pen_dot(dot);
        state.push_pen_stroke(stroke);
    }
    state.clear_pen();
    state.push_pen_dot(dot);
    let commands = state.take_pending_pen_commands();
    assert!(
        matches!(commands.as_slice(), [PenCommand::Mixed(marks), PenCommand::Clear, PenCommand::Dots(last)] if marks.len() == 40 && marks.chunks_exact(2).all(|pair| pair[0] == dot.as_stroke() && pair[1] == stroke) && last == &[dot])
    );
}
