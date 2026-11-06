//! Authoritative Scratch project state and its implementation.

use super::operators::to_text;
use super::{
    CachedListIndex, PenColorParameter, PenColorState, PenCommand, PenDot, PenStroke,
    RealtimeAudio, ScratchInput, ScratchList, ScratchValue, ScratchValueView, SoundLibrary,
    SoundParameters, SoundRuntime, scratch_number_operand, scratch_random, scratch_round,
    scratch_string, scratch_to_number,
};
use std::collections::HashSet;
use std::fmt::Debug;
use std::sync::{Arc, mpsc::Sender};

mod clones;
mod lists;
mod monitors;
mod pen;
mod sensing;
mod snapshots;
mod sound;
mod targets;
mod variables;

use lists::ListMonitorEdit;
use sensing::{RuntimeInputState, default_username};
use snapshots::SpeechBubble;
use targets::{ScratchCostumeMetadata, ScratchTargetInstance};

pub use monitors::{
    ScratchListMetadata, ScratchVariableMetadata, list_snapshot_key, variable_snapshot_key,
};
pub use sensing::{RuntimeInputEvent, pointer_click_activates};
pub use snapshots::{
    BubbleKind, BubbleSnapshot, LayerSnapshot, ListSnapshot, ObservableEvent, ReporterSnapshot,
    RuntimeSnapshot, VariableSnapshot,
};
pub use sound::ScratchSoundEffect;
pub use targets::{ScratchRotationStyle, SpriteFenceGeometry, scratch_motion_coordinate};

/// canonical project state read and advanced by the generated scheduler.
#[derive(Debug)]
pub struct ScratchState {
    values: Vec<ScratchValue>,
    variables: Vec<ScratchVariableMetadata>,
    lists: Vec<ScratchList>,
    list_metadata: Vec<ScratchListMetadata>,
    list_visibility: Vec<bool>,
    instances: Vec<ScratchTargetInstance>,
    current_instance: usize,
    pending_clones: Vec<usize>,
    target_names: Vec<(usize, String, bool)>,
    target_costumes: Vec<Vec<ScratchCostumeMetadata>>,
    fence_geometry: Vec<Option<SpriteFenceGeometry>>,
    event_sink: Option<Sender<ObservableEvent>>,
    input: RuntimeInputState,
    redraw_requested: bool,
    presentation_requested: bool,
    backdrop_names: Vec<String>,
    backdrop_index: usize,
    pending_backdrop_events: Vec<String>,
    project_timer_started_at_millis: u64,
    project_timer_epoch: u64,
    project_timer_monitor_visible: bool,
    username: String,
    loudness: f64,
    pending_pen_commands: Vec<PenCommand>,
    sound_runtime: SoundRuntime,
}

impl ScratchState {
    /// clear the previous frame's request before stepping generated scripts.
    pub fn begin_tick(&mut self) {
        self.redraw_requested = false;
        self.presentation_requested = false;
    }

    pub fn request_redraw(&mut self) {
        self.redraw_requested = true;
        self.presentation_requested = true;
    }

    /// request an observable snapshot without forcing a scheduler redraw.
    pub fn request_presentation(&mut self) {
        self.presentation_requested = true;
    }

    pub fn redraw_requested(&self) -> bool {
        self.redraw_requested
    }

    pub fn presentation_consumed(&mut self) {
        self.redraw_requested = false;
        self.presentation_requested = false;
    }

    pub fn presentation_requested(&self) -> bool {
        self.presentation_requested
    }

    /// create state without variable names.
    pub fn new(values: Vec<ScratchValue>) -> Self {
        Self {
            values,
            variables: Vec::new(),
            lists: Vec::new(),
            list_metadata: Vec::new(),
            list_visibility: Vec::new(),
            instances: vec![ScratchTargetInstance {
                target: 0,
                original: true,
                alive: true,
                is_stage: false,
                x: 0.0,
                y: 0.0,
                direction: 90.0,
                rotation_style: ScratchRotationStyle::AllAround,
                size: 100.0,
                visible: true,
                values: Vec::new(),
                lists: Vec::new(),
                bubble: None,
                pen_color: PenColorState::default(),
                pen_down: false,
                pen_size: 1.0,
                costume_index: 0,
                fence_offsets: None,
                sound_pitch: 0.0,
                sound_pan: 0.0,
                volume: 100.0,
            }],
            current_instance: 0,
            pending_clones: Vec::new(),
            target_names: Vec::new(),
            target_costumes: Vec::new(),
            fence_geometry: Vec::new(),
            event_sink: None,
            input: RuntimeInputState::default(),
            redraw_requested: false,
            presentation_requested: false,
            backdrop_names: Vec::new(),
            backdrop_index: 0,
            pending_backdrop_events: Vec::new(),
            project_timer_started_at_millis: 0,
            project_timer_epoch: 0,
            project_timer_monitor_visible: false,
            username: default_username(),
            loudness: -1.0,
            pending_pen_commands: Vec::new(),
            sound_runtime: SoundRuntime::default(),
        }
    }

    /// create state with metadata aligned to the value slots.
    pub fn with_metadata(
        values: Vec<ScratchValue>,
        variables: Vec<ScratchVariableMetadata>,
    ) -> Self {
        assert_eq!(values.len(), variables.len());
        Self {
            values,
            variables,
            lists: Vec::new(),
            list_metadata: Vec::new(),
            list_visibility: Vec::new(),
            instances: vec![ScratchTargetInstance {
                target: 0,
                original: true,
                alive: true,
                is_stage: false,
                x: 0.0,
                y: 0.0,
                direction: 90.0,
                rotation_style: ScratchRotationStyle::AllAround,
                size: 100.0,
                visible: true,
                values: Vec::new(),
                lists: Vec::new(),
                bubble: None,
                pen_color: PenColorState::default(),
                pen_down: false,
                pen_size: 1.0,
                costume_index: 0,
                fence_offsets: None,
                sound_pitch: 0.0,
                sound_pan: 0.0,
                volume: 100.0,
            }],
            current_instance: 0,
            pending_clones: Vec::new(),
            target_names: Vec::new(),
            target_costumes: Vec::new(),
            fence_geometry: Vec::new(),
            event_sink: None,
            input: RuntimeInputState::default(),
            redraw_requested: false,
            presentation_requested: false,
            backdrop_names: Vec::new(),
            backdrop_index: 0,
            pending_backdrop_events: Vec::new(),
            project_timer_started_at_millis: 0,
            project_timer_epoch: 0,
            project_timer_monitor_visible: false,
            username: default_username(),
            loudness: -1.0,
            pending_pen_commands: Vec::new(),
            sound_runtime: SoundRuntime::default(),
        }
    }

    /// create project state with shared variables, lists, metadata, and visibility.
    pub fn with_project_data(
        values: Vec<ScratchValue>,
        variables: Vec<ScratchVariableMetadata>,
        lists: Vec<ScratchList>,
        list_metadata: Vec<ScratchListMetadata>,
        list_visibility: Vec<bool>,
    ) -> Self {
        assert_eq!(values.len(), variables.len());
        assert_eq!(lists.len(), list_metadata.len());
        assert_eq!(lists.len(), list_visibility.len());
        Self {
            values,
            variables,
            lists,
            list_metadata,
            list_visibility,
            instances: vec![ScratchTargetInstance {
                target: 0,
                original: true,
                alive: true,
                is_stage: false,
                x: 0.0,
                y: 0.0,
                direction: 90.0,
                rotation_style: ScratchRotationStyle::AllAround,
                size: 100.0,
                visible: true,
                values: Vec::new(),
                lists: Vec::new(),
                bubble: None,
                pen_color: PenColorState::default(),
                pen_down: false,
                pen_size: 1.0,
                costume_index: 0,
                fence_offsets: None,
                sound_pitch: 0.0,
                sound_pan: 0.0,
                volume: 100.0,
            }],
            current_instance: 0,
            pending_clones: Vec::new(),
            target_names: Vec::new(),
            target_costumes: Vec::new(),
            fence_geometry: Vec::new(),
            event_sink: None,
            input: RuntimeInputState::default(),
            redraw_requested: false,
            presentation_requested: false,
            backdrop_names: Vec::new(),
            backdrop_index: 0,
            pending_backdrop_events: Vec::new(),
            project_timer_started_at_millis: 0,
            project_timer_epoch: 0,
            project_timer_monitor_visible: false,
            username: default_username(),
            loudness: -1.0,
            pending_pen_commands: Vec::new(),
            sound_runtime: SoundRuntime::default(),
        }
    }
}

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
