//! ScratchState regression tests grouped by behavior.

use super::{
    BubbleKind, ObservableEvent, PenDot, PenStroke, RuntimeInputEvent, ScratchListMetadata,
    ScratchRotationStyle, ScratchSoundEffect, ScratchState, ScratchValue, ScratchVariableMetadata,
    SpriteFenceGeometry, pointer_click_activates,
};
use crate::runtime::PenCommand;
use crate::runtime::{ScratchList, list_snapshot_key, scratch_to_number};
use std::sync::Arc;

fn clone_test_state() -> ScratchState {
    let mut state = ScratchState::with_project_data(
        vec![10.0_f64.into(), 3.0_f64.into()],
        vec![
            ScratchVariableMetadata::for_target("sprite-local", "score", "Sprite", 1, false),
            ScratchVariableMetadata::for_target("stage-global", "score", "Stage", 0, true),
        ],
        vec![
            ScratchList::new(vec!["seed".into()]),
            ScratchList::default(),
        ],
        vec![
            ScratchListMetadata::for_target("sprite-list", "items", "Sprite", 1, false),
            ScratchListMetadata::for_target("stage-list", "items", "Stage", 0, true),
        ],
        vec![false, false],
    );
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite", false);
    state
}

fn fenced_state(width: f64, height: f64) -> ScratchState {
    let mut state = ScratchState::new(Vec::new());
    state.register_target(1, "Sprite", false);
    state.set_target_fence_geometry(
        1,
        SpriteFenceGeometry {
            width,
            height,
            rotation_center_x: width / 2.0,
            rotation_center_y: height / 2.0,
            bounds_hull: None,
        },
    );
    state.set_current_instance(state.original_instance(1).unwrap());
    state
}

mod clones;
mod lists;
mod pen;
mod sensing;
mod snapshots;
mod sound;
mod targets;
mod variables;
