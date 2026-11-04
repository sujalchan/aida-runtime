#[derive(Debug, Clone, PartialEq)]
pub enum ScratchValue {
    Number(f64),
    Text(String),
    Boolean(bool),
}

use super::{
    PenColorState, PenCommand, ScratchList, scratch_number_operand, scratch_random, scratch_string,
    scratch_to_number,
};
use std::collections::HashSet;
use std::fmt::Debug;
use std::sync::{Arc, mpsc::Sender};
use time::OffsetDateTime;

fn default_username() -> String {
    let suffix = scratch_random(1000.0, 9999.0) as u16;
    format!("player{suffix:04}")
}

#[inline]
fn add_to_scratch_value(value: &mut ScratchValue, change: f64) {
    match value {
        ScratchValue::Number(number) => *number = scratch_number_operand(*number) + change,
        ScratchValue::Text(_) | ScratchValue::Boolean(_) => {
            *value = ScratchValue::Number(scratch_to_number(&*value) + change);
        }
    }
}

fn scratch_direction(direction: f64) -> f64 {
    direction - (((direction + 179.0) / 360.0).floor() * 360.0)
}

/// input collected by the presentation thread and consumed by generated code.
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeInputEvent {
    Key {
        name: String,
        pressed: bool,
    },
    MousePosition {
        x: f64,
        y: f64,
    },
    MouseButton {
        pressed: bool,
    },
    VariableValue {
        key: String,
        value: f64,
    },
    ListAppend {
        key: String,
    },
    ListSetItem {
        key: String,
        index: usize,
        value: String,
    },
    ListDeleteItem {
        key: String,
        index: usize,
    },
    ListReplace {
        key: String,
        items: Vec<String>,
    },
}

enum ListMonitorEdit {
    Append,
    SetItem { index: usize, value: String },
    DeleteItem { index: usize },
    Replace(Vec<String>),
}

#[derive(Debug, Default)]
struct RuntimeInputState {
    pressed_keys: HashSet<String>,
    mouse_x: f64,
    mouse_y: f64,
    mouse_down: bool,
}

impl RuntimeInputState {
    fn apply(&mut self, event: RuntimeInputEvent) -> (Option<String>, bool) {
        match event {
            RuntimeInputEvent::Key { name, pressed } => {
                let name = name.to_ascii_lowercase();
                if pressed {
                    let changed = self.pressed_keys.insert(name.clone());
                    (changed.then_some(name), changed)
                } else {
                    let changed = self.pressed_keys.remove(&name);
                    (None, changed)
                }
            }
            RuntimeInputEvent::MousePosition { x, y } => {
                let changed = self.mouse_x != x || self.mouse_y != y;
                self.mouse_x = x;
                self.mouse_y = y;
                (None, changed)
            }
            RuntimeInputEvent::MouseButton { pressed } => {
                let changed = self.mouse_down != pressed;
                self.mouse_down = pressed;
                (None, changed)
            }
            RuntimeInputEvent::VariableValue { .. }
            | RuntimeInputEvent::ListAppend { .. }
            | RuntimeInputEvent::ListSetItem { .. }
            | RuntimeInputEvent::ListDeleteItem { .. }
            | RuntimeInputEvent::ListReplace { .. } => (None, false),
        }
    }
}

/// a borrowed view of a scratch value accepted by runtime operators.
#[derive(Debug, Clone, Copy)]
pub enum ScratchValueView<'a> {
    Number(f64),
    Text(&'a str),
    Boolean(bool),
}

/// a value that a scratch operator can inspect without taking ownership.
pub trait ScratchInput {
    fn scratch_view(&self) -> ScratchValueView<'_>;
}

impl ScratchInput for ScratchValue {
    #[inline]
    fn scratch_view(&self) -> ScratchValueView<'_> {
        match self {
            Self::Number(number) => ScratchValueView::Number(*number),
            Self::Text(text) => ScratchValueView::Text(text),
            Self::Boolean(value) => ScratchValueView::Boolean(*value),
        }
    }
}

impl<T: ScratchInput + ?Sized> ScratchInput for &T {
    #[inline]
    fn scratch_view(&self) -> ScratchValueView<'_> {
        T::scratch_view(self)
    }
}

impl ScratchInput for f64 {
    #[inline]
    fn scratch_view(&self) -> ScratchValueView<'_> {
        ScratchValueView::Number(*self)
    }
}

impl ScratchInput for bool {
    #[inline]
    fn scratch_view(&self) -> ScratchValueView<'_> {
        ScratchValueView::Boolean(*self)
    }
}

impl ScratchInput for str {
    #[inline]
    fn scratch_view(&self) -> ScratchValueView<'_> {
        ScratchValueView::Text(self)
    }
}

impl ScratchInput for String {
    #[inline]
    fn scratch_view(&self) -> ScratchValueView<'_> {
        ScratchValueView::Text(self)
    }
}

impl From<f64> for ScratchValue {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<bool> for ScratchValue {
    fn from(value: bool) -> Self {
        Self::Boolean(value)
    }
}

impl From<&str> for ScratchValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for ScratchValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

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
    target_costumes: Vec<(usize, Vec<String>)>,
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
    pending_pen_commands: Vec<PenCommand>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObservableEvent {
    Output {
        label: String,
        value: String,
    },
    VariableChange {
        key: String,
        old_value: String,
        new_value: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariableSnapshot {
    pub key: String,
    pub id: String,
    pub name: String,
    pub target: String,
    pub instance_id: Option<usize>,
    pub value: ScratchValue,
    pub monitor_visible: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReporterSnapshot {
    pub key: String,
    pub value: ScratchValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListSnapshot {
    pub key: String,
    pub id: String,
    pub name: String,
    pub target: String,
    pub instance_id: Option<usize>,
    pub items: Arc<Vec<ScratchValue>>,
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayerSnapshot {
    pub target_index: usize,
    pub target: String,
    pub instance_id: usize,
    pub original: bool,
    pub x: f64,
    pub y: f64,
    pub direction: f64,
    pub size: f64,
    pub visible: bool,
    // preserve costume selection per sprite instance in renderer snapshots.
    pub costume_index: usize,
    pub bubble: Option<BubbleSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteFenceGeometry {
    pub width: f64,
    pub height: f64,
    pub rotation_center_x: f64,
    pub rotation_center_y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BubbleKind {
    Say,
    Think,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BubbleSnapshot {
    pub kind: BubbleKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SpeechBubble {
    kind: BubbleKind,
    text: String,
    expires_at_millis: Option<u64>,
}

/// immutable copies for presentation and observation, never script execution state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuntimeSnapshot {
    pub tick: usize,
    pub backdrop_index: usize,
    pub backdrop_name: String,
    pub project_timer_millis: u64,
    pub project_timer_epoch: u64,
    pub reporters: Vec<ReporterSnapshot>,
    pub variables: Vec<VariableSnapshot>,
    pub lists: Vec<ListSnapshot>,
    pub layers: Vec<LayerSnapshot>,
}

pub fn variable_snapshot_key(
    target: &str,
    name: &str,
    id: &str,
    instance_id: Option<usize>,
) -> String {
    let instance = instance_id.map_or_else(|| "shared".to_owned(), |id| id.to_string());
    format!("{target}::{name}#{id}@{instance}")
}

pub fn list_snapshot_key(target: &str, name: &str, id: &str, instance_id: Option<usize>) -> String {
    let instance = instance_id.map_or_else(|| "shared".to_owned(), |id| id.to_string());
    format!("{target}::{name}#{id}@{instance}")
}

#[derive(Debug, Clone)]
struct ScratchTargetInstance {
    target: usize,
    original: bool,
    alive: bool,
    is_stage: bool,
    x: f64,
    y: f64,
    direction: f64,
    size: f64,
    visible: bool,
    values: Vec<Option<ScratchValue>>,
    lists: Vec<Option<ScratchList>>,
    bubble: Option<SpeechBubble>,
    pen_color: PenColorState,
    pen_down: bool,
    pen_size: f64,
    costume_index: usize,
}

/// names and scope information for one shared-state slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScratchVariableMetadata {
    pub id: String,
    pub name: String,
    pub target: String,
    pub is_stage: bool,
    pub target_index: usize,
    pub monitor_visible: bool,
}

/// names and target scope for one shared list slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScratchListMetadata {
    pub id: String,
    pub name: String,
    pub target: String,
    pub is_stage: bool,
    pub target_index: usize,
}

impl ScratchListMetadata {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        target: impl Into<String>,
        is_stage: bool,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            target: target.into(),
            is_stage,
            target_index: 0,
        }
    }

    pub fn for_target(
        id: impl Into<String>,
        name: impl Into<String>,
        target: impl Into<String>,
        target_index: usize,
        is_stage: bool,
    ) -> Self {
        Self {
            target_index,
            ..Self::new(id, name, target, is_stage)
        }
    }
}

impl ScratchVariableMetadata {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        target: impl Into<String>,
        is_stage: bool,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            target: target.into(),
            is_stage,
            target_index: 0,
            monitor_visible: false,
        }
    }

    pub fn for_target(
        id: impl Into<String>,
        name: impl Into<String>,
        target: impl Into<String>,
        target_index: usize,
        is_stage: bool,
    ) -> Self {
        Self {
            target_index,
            ..Self::new(id, name, target, is_stage)
        }
    }

    pub fn with_monitor_visible(mut self, visible: bool) -> Self {
        self.monitor_visible = visible;
        self
    }
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

    fn request_presentation(&mut self) {
        self.presentation_requested = true;
    }

    pub fn set_speech_bubble(
        &mut self,
        kind: BubbleKind,
        text: String,
        expires_at_millis: Option<u64>,
    ) {
        let instance = &mut self.instances[self.current_instance];
        let next = if instance.is_stage || text.is_empty() {
            None
        } else {
            Some(SpeechBubble {
                kind,
                text,
                expires_at_millis,
            })
        };
        let visually_changed = instance
            .bubble
            .as_ref()
            .map(|bubble| (bubble.kind, bubble.text.as_str()))
            != next
                .as_ref()
                .map(|bubble| (bubble.kind, bubble.text.as_str()));
        instance.bubble = next;
        if visually_changed {
            self.request_redraw();
        }
    }

    /// remove timed bubbles once their script wait has reached its deadline.
    pub fn expire_speech_bubbles(&mut self, now_millis: u64) {
        let mut changed = false;
        for instance in &mut self.instances {
            if instance.bubble.as_ref().is_some_and(|bubble| {
                bubble
                    .expires_at_millis
                    .is_some_and(|deadline| deadline <= now_millis)
            }) {
                instance.bubble = None;
                changed = true;
            }
        }
        if changed {
            self.request_redraw();
        }
    }

    /// clear speech and thought bubbles when the project is stopped.
    pub fn clear_speech_bubbles(&mut self) {
        let mut changed = false;
        for instance in &mut self.instances {
            changed |= instance.bubble.take().is_some();
        }
        if changed {
            self.request_redraw();
        }
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

    /// update canonical input state from the renderer's queued events.
    pub fn apply_input(&mut self, event: RuntimeInputEvent) -> Option<String> {
        match event {
            RuntimeInputEvent::VariableValue { key, value } => {
                self.apply_variable_edit(&key, value);
                None
            }
            RuntimeInputEvent::ListAppend { key } => {
                self.apply_list_monitor_edit(&key, ListMonitorEdit::Append);
                None
            }
            RuntimeInputEvent::ListSetItem { key, index, value } => {
                self.apply_list_monitor_edit(&key, ListMonitorEdit::SetItem { index, value });
                None
            }
            RuntimeInputEvent::ListDeleteItem { key, index } => {
                self.apply_list_monitor_edit(&key, ListMonitorEdit::DeleteItem { index });
                None
            }
            RuntimeInputEvent::ListReplace { key, items } => {
                self.apply_list_monitor_edit(&key, ListMonitorEdit::Replace(items));
                None
            }
            event => {
                let (keypress, changed) = self.input.apply(event);
                if changed {
                    self.request_presentation();
                }
                keypress
            }
        }
    }

    fn apply_list_monitor_edit(&mut self, key: &str, edit: ListMonitorEdit) -> bool {
        let target = self
            .list_metadata
            .iter()
            .enumerate()
            .find_map(|(slot, metadata)| {
                let instance = if metadata.is_stage {
                    None
                } else {
                    Some(self.original_instance(metadata.target_index)?)
                };
                (list_snapshot_key(&metadata.target, &metadata.name, &metadata.id, instance) == key)
                    .then_some((slot, instance))
            });
        let Some((slot, instance)) = target else {
            return false;
        };

        let default_list = instance.map(|_| self.lists[slot].clone());
        let Some(list) = (if let Some(instance_id) = instance {
            let Some(instance) = self.instances.get_mut(instance_id) else {
                return false;
            };
            if instance.lists[slot].is_none() {
                instance.lists[slot] = default_list;
            }
            instance.lists[slot].as_mut()
        } else {
            self.lists.get_mut(slot)
        }) else {
            return false;
        };

        let changed = match edit {
            ListMonitorEdit::Append => list.add(ScratchValue::Text(String::new())),
            ListMonitorEdit::SetItem { index, value } => {
                if index == 0 || index > list.length() {
                    false
                } else {
                    let value = ScratchValue::Text(value);
                    if list.get(index as f64) == &value {
                        false
                    } else {
                        list.replace(index as f64, value);
                        true
                    }
                }
            }
            ListMonitorEdit::DeleteItem { index } => {
                if index == 0 || index > list.length() {
                    false
                } else {
                    list.delete(index as f64);
                    true
                }
            }
            ListMonitorEdit::Replace(items) => {
                let next = items
                    .into_iter()
                    .map(ScratchValue::Text)
                    .collect::<Vec<_>>();
                if list.items() == next {
                    false
                } else {
                    list.replace_all(next);
                    true
                }
            }
        };
        if changed {
            self.request_presentation();
        }
        changed
    }

    fn apply_variable_edit(&mut self, key: &str, value: f64) -> bool {
        let target = self
            .variables
            .iter()
            .enumerate()
            .find_map(|(slot, metadata)| {
                let instance = if metadata.is_stage {
                    None
                } else {
                    Some(self.original_instance(metadata.target_index)?)
                };
                (variable_snapshot_key(&metadata.target, &metadata.name, &metadata.id, instance)
                    == key)
                    .then_some((slot, instance))
            });
        let Some((slot, instance)) = target else {
            return false;
        };
        let next = ScratchValue::Number(value);
        if let Some(instance) = instance {
            let previous_instance = self.current_instance;
            self.current_instance = instance;
            self.set(slot, next);
            self.current_instance = previous_instance;
        } else {
            self.set_shared(slot, next);
        }
        true
    }

    pub fn key_pressed(&self, key: &str) -> bool {
        self.input.pressed_keys.contains(&key.to_ascii_lowercase())
    }

    pub fn mouse_x(&self) -> f64 {
        self.input.mouse_x
    }
    pub fn mouse_y(&self) -> f64 {
        self.input.mouse_y
    }
    pub fn mouse_down(&self) -> bool {
        self.input.mouse_down
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
                size: 100.0,
                visible: true,
                values: Vec::new(),
                lists: Vec::new(),
                bubble: None,
                pen_color: PenColorState::default(),
                pen_down: false,
                pen_size: 1.0,
                costume_index: 0,
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
            pending_pen_commands: Vec::new(),
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
                size: 100.0,
                visible: true,
                values: Vec::new(),
                lists: Vec::new(),
                bubble: None,
                pen_color: PenColorState::default(),
                pen_down: false,
                pen_size: 1.0,
                costume_index: 0,
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
            pending_pen_commands: Vec::new(),
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
                size: 100.0,
                visible: true,
                values: Vec::new(),
                lists: Vec::new(),
                bubble: None,
                pen_color: PenColorState::default(),
                pen_down: false,
                pen_size: 1.0,
                costume_index: 0,
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
            pending_pen_commands: Vec::new(),
        }
    }

    pub fn set_backdrops(&mut self, names: Vec<String>, current_index: usize) {
        self.backdrop_names = names;
        self.backdrop_index = current_index.min(self.backdrop_names.len().saturating_sub(1));
        self.pending_backdrop_events.clear();
    }

    pub fn backdrop_number_name(&self, number: bool) -> ScratchValue {
        if number {
            ScratchValue::Number(self.backdrop_index as f64 + 1.0)
        } else {
            ScratchValue::Text(
                self.backdrop_names
                    .get(self.backdrop_index)
                    .cloned()
                    .unwrap_or_default(),
            )
        }
    }

    pub fn switch_backdrop(&mut self, value: impl super::ScratchInput) {
        self.select_backdrop(value.scratch_view(), false);
    }

    pub fn next_backdrop(&mut self) {
        self.select_backdrop(
            super::ScratchValueView::Number(self.backdrop_index as f64 + 1.0),
            true,
        );
    }

    pub fn take_pending_backdrop_events(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_backdrop_events)
    }

    fn select_backdrop(&mut self, value: super::ScratchValueView<'_>, zero_indexed: bool) {
        if self.backdrop_names.is_empty() {
            return;
        }
        let selected = match value {
            super::ScratchValueView::Number(number) => {
                self.backdrop_index_from_number(number, zero_indexed)
            }
            super::ScratchValueView::Text(text) => {
                if let Some(index) = self.backdrop_names.iter().position(|name| name == text) {
                    Some(index)
                } else if text == "next backdrop" {
                    Some((self.backdrop_index + 1) % self.backdrop_names.len())
                } else if text == "previous backdrop" {
                    Some(
                        (self.backdrop_index + self.backdrop_names.len() - 1)
                            % self.backdrop_names.len(),
                    )
                } else if text == "random backdrop" {
                    if self.backdrop_names.len() < 2 {
                        Some(self.backdrop_index)
                    } else {
                        let other =
                            scratch_random(0.0, (self.backdrop_names.len() - 2) as f64) as usize;
                        Some(if other >= self.backdrop_index {
                            other + 1
                        } else {
                            other
                        })
                    }
                } else if text.trim().is_empty() {
                    None
                } else {
                    text.trim()
                        .parse::<f64>()
                        .ok()
                        .and_then(|number| self.backdrop_index_from_number(number, zero_indexed))
                }
            }
            super::ScratchValueView::Boolean(_) => None,
        };
        if let Some(index) = selected {
            self.backdrop_index = index;
            // scratch's backdrop switch calls setCostume, which requests a
            // redraw even when the selected costume is already active.
            self.request_redraw();
        }
        if let Some(name) = self.backdrop_names.get(self.backdrop_index) {
            self.pending_backdrop_events.push(name.clone());
        }
    }

    fn backdrop_index_from_number(&self, number: f64, zero_indexed: bool) -> Option<usize> {
        let adjusted = if zero_indexed { number } else { number - 1.0 };
        let rounded = if adjusted.is_finite() {
            (adjusted + 0.5).floor()
        } else {
            0.0
        };
        Some((rounded.rem_euclid(self.backdrop_names.len() as f64)) as usize)
    }

    #[inline]
    pub fn get(&self, slot: usize) -> &ScratchValue {
        if let Some(value) = self.instances[self.current_instance]
            .values
            .get(slot)
            .and_then(Option::as_ref)
        {
            return value;
        }
        &self.values[slot]
    }

    /// read a stage variable without checking clone-local storage.
    #[inline]
    pub fn get_shared(&self, slot: usize) -> &ScratchValue {
        &self.values[slot]
    }

    #[inline]
    pub fn set(&mut self, slot: usize, value: ScratchValue) {
        let change = self.capture_variable_change(slot);
        let old_value = self.monitored_variable_value(slot);
        if let Some(metadata) = self.variables.get(slot) {
            if !metadata.is_stage {
                self.ensure_instance_slot(slot);
                self.instances[self.current_instance].values[slot] = Some(value);
                self.finish_monitored_variable_write(slot, old_value);
                self.publish_variable_change(change, slot);
                return;
            }
        }
        self.values[slot] = value;
        self.finish_monitored_variable_write(slot, old_value);
        self.publish_variable_change(change, slot);
    }

    /// write a stage variable without checking variable scope metadata.
    #[inline]
    pub fn set_shared(&mut self, slot: usize, value: ScratchValue) {
        let change = self.capture_variable_change(slot);
        let old_value = self.monitored_variable_value(slot);
        self.values[slot] = value;
        self.finish_monitored_variable_write(slot, old_value);
        self.publish_variable_change(change, slot);
    }

    /// apply a scratch-converted numeric change directly to a stage variable.
    #[inline]
    pub fn change_shared(&mut self, slot: usize, change: f64) {
        let old_value = self.capture_variable_change(slot);
        let monitor_value = self.monitored_variable_value(slot);
        add_to_scratch_value(&mut self.values[slot], change);
        self.finish_monitored_variable_write(slot, monitor_value);
        self.publish_variable_change(old_value, slot);
    }

    /// change a sprite-local variable for the current instance.
    #[inline]
    pub fn change_local_by(&mut self, slot: usize, change: f64) {
        if self.event_sink.is_none() {
            let monitor_value = self.monitored_variable_value(slot);
            let current_instance = self.current_instance;
            if let Some(value) = self.instances[current_instance]
                .values
                .get_mut(slot)
                .and_then(Option::as_mut)
            {
                add_to_scratch_value(value, change);
                self.finish_monitored_variable_write(slot, monitor_value);
                return;
            }
            let initial = self.values[slot].clone();
            self.ensure_instance_slot(slot);
            let value = self.instances[current_instance].values[slot].get_or_insert(initial);
            add_to_scratch_value(value, change);
            self.finish_monitored_variable_write(slot, monitor_value);
            return;
        }

        let old_value = self.capture_variable_change(slot);
        let monitor_value = self.monitored_variable_value(slot);
        let current_instance = self.current_instance;
        if let Some(value) = self.instances[current_instance]
            .values
            .get_mut(slot)
            .and_then(Option::as_mut)
        {
            add_to_scratch_value(value, change);
        } else {
            let initial = self.values[slot].clone();
            self.ensure_instance_slot(slot);
            let value = self.instances[current_instance].values[slot].get_or_insert(initial);
            add_to_scratch_value(value, change);
        }
        self.finish_monitored_variable_write(slot, monitor_value);
        self.publish_variable_change(old_value, slot);
    }

    fn monitored_variable_value(&self, slot: usize) -> Option<ScratchValue> {
        let metadata = self.variables.get(slot)?;
        if !metadata.monitor_visible {
            return None;
        }
        if metadata.is_stage {
            return Some(self.values[slot].clone());
        }
        let instance = self.instances.get(self.current_instance)?;
        if instance.original && instance.target == metadata.target_index {
            Some(
                instance
                    .values
                    .get(slot)
                    .and_then(Option::as_ref)
                    .unwrap_or(&self.values[slot])
                    .clone(),
            )
        } else {
            None
        }
    }

    fn finish_monitored_variable_write(&mut self, slot: usize, old_value: Option<ScratchValue>) {
        let Some(old_value) = old_value else {
            return;
        };
        let metadata = &self.variables[slot];
        let current = if metadata.is_stage {
            &self.values[slot]
        } else {
            self.instances[self.current_instance].values[slot]
                .as_ref()
                .unwrap_or(&self.values[slot])
        };
        if *current != old_value {
            self.request_presentation();
        }
    }

    pub fn set_event_sink(&mut self, event_sink: Option<Sender<ObservableEvent>>) {
        self.event_sink = event_sink;
    }

    pub fn emit_observable_output(&self, label: &str, value: &impl Debug) {
        println!("{label}: {:?}", value);
        if let Some(event_sink) = &self.event_sink {
            let _ = event_sink.send(ObservableEvent::Output {
                label: label.to_owned(),
                value: format!("{value:?}"),
            });
        }
    }

    fn capture_variable_change(&self, slot: usize) -> Option<(String, String)> {
        if self.event_sink.is_none() {
            return None;
        }
        let metadata = self.variables.get(slot)?;
        let instance_id = if metadata.is_stage {
            None
        } else {
            Some(self.current_instance)
        };
        let key =
            variable_snapshot_key(&metadata.target, &metadata.name, &metadata.id, instance_id);
        Some((key, format!("{:?}", self.get(slot))))
    }

    fn publish_variable_change(&self, old: Option<(String, String)>, slot: usize) {
        let (Some((key, old_value)), Some(event_sink)) = (old, self.event_sink.as_ref()) else {
            return;
        };
        let new_value = format!("{:?}", self.get(slot));
        if old_value != new_value {
            let _ = event_sink.send(ObservableEvent::VariableChange {
                key,
                old_value,
                new_value,
            });
        }
    }

    pub fn observable_snapshot(&self, tick: usize) -> RuntimeSnapshot {
        let mut snapshot = RuntimeSnapshot {
            tick,
            backdrop_index: self.backdrop_index,
            backdrop_name: self
                .backdrop_names
                .get(self.backdrop_index)
                .cloned()
                .unwrap_or_default(),
            ..RuntimeSnapshot::default()
        };
        for (instance_id, instance) in self.instances.iter().enumerate() {
            if !instance.alive {
                continue;
            }
            let Some((_, target, is_stage)) = self
                .target_names
                .iter()
                .find(|(target_index, _, _)| *target_index == instance.target)
            else {
                continue;
            };
            if !is_stage {
                snapshot.layers.push(LayerSnapshot {
                    target_index: instance.target,
                    target: target.clone(),
                    instance_id,
                    original: instance.original,
                    x: instance.x,
                    y: instance.y,
                    direction: instance.direction,
                    size: instance.size,
                    visible: instance.visible,
                    costume_index: instance.costume_index,
                    bubble: instance.bubble.as_ref().map(|bubble| BubbleSnapshot {
                        kind: bubble.kind,
                        text: bubble.text.clone(),
                    }),
                });
            }
        }
        for (slot, metadata) in self.variables.iter().enumerate() {
            if metadata.is_stage {
                snapshot.variables.push(VariableSnapshot {
                    key: variable_snapshot_key(
                        &metadata.target,
                        &metadata.name,
                        &metadata.id,
                        None,
                    ),
                    id: metadata.id.clone(),
                    name: metadata.name.clone(),
                    target: metadata.target.clone(),
                    instance_id: None,
                    value: self.values[slot].clone(),
                    monitor_visible: metadata.monitor_visible,
                });
                continue;
            }
            for (instance_id, instance) in self.instances.iter().enumerate() {
                if !instance.alive || instance.target != metadata.target_index {
                    continue;
                }
                let value = instance
                    .values
                    .get(slot)
                    .and_then(Option::as_ref)
                    .unwrap_or(&self.values[slot]);
                snapshot.variables.push(VariableSnapshot {
                    key: variable_snapshot_key(
                        &metadata.target,
                        &metadata.name,
                        &metadata.id,
                        Some(instance_id),
                    ),
                    id: metadata.id.clone(),
                    name: metadata.name.clone(),
                    target: metadata.target.clone(),
                    instance_id: Some(instance_id),
                    value: value.clone(),
                    monitor_visible: metadata.monitor_visible,
                });
            }
        }
        for (slot, metadata) in self.list_metadata.iter().enumerate() {
            if metadata.is_stage {
                snapshot.lists.push(ListSnapshot {
                    key: list_snapshot_key(&metadata.target, &metadata.name, &metadata.id, None),
                    id: metadata.id.clone(),
                    name: metadata.name.clone(),
                    target: metadata.target.clone(),
                    instance_id: None,
                    items: self.lists[slot].shared_items(),
                    visible: self.list_visibility[slot],
                });
                continue;
            }
            for (instance_id, instance) in self.instances.iter().enumerate() {
                if !instance.alive || instance.target != metadata.target_index {
                    continue;
                }
                let list = instance
                    .lists
                    .get(slot)
                    .and_then(Option::as_ref)
                    .unwrap_or(&self.lists[slot]);
                snapshot.lists.push(ListSnapshot {
                    key: list_snapshot_key(
                        &metadata.target,
                        &metadata.name,
                        &metadata.id,
                        Some(instance_id),
                    ),
                    id: metadata.id.clone(),
                    name: metadata.name.clone(),
                    target: metadata.target.clone(),
                    instance_id: Some(instance_id),
                    items: list.shared_items(),
                    visible: self.list_visibility[slot],
                });
            }
        }
        snapshot
    }

    /// reset the project-wide Scratch timer against the runtime clock.
    pub fn reset_project_timer(&mut self, now_millis: u64) {
        self.project_timer_started_at_millis = now_millis;
        self.project_timer_epoch = self.project_timer_epoch.wrapping_add(1);
        if self.project_timer_monitor_visible {
            self.request_presentation();
        }
    }

    pub fn set_project_timer_monitor_visible(&mut self, visible: bool) {
        self.project_timer_monitor_visible = visible;
    }

    pub fn set_username(&mut self, username: String) {
        self.username = username;
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn days_since_2000(&self) -> f64 {
        const UNIX_DAYS_TO_2000: f64 = 10_957.0;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        now.as_secs_f64() / 86_400.0 - UNIX_DAYS_TO_2000
    }

    pub fn current_year(&self) -> f64 {
        local_datetime().year() as f64
    }

    pub fn current_month(&self) -> f64 {
        local_datetime().month() as u8 as f64
    }

    pub fn current_date(&self) -> f64 {
        local_datetime().day() as f64
    }

    pub fn current_day_of_week(&self) -> f64 {
        local_datetime().weekday().number_from_sunday() as f64
    }

    pub fn current_hour(&self) -> f64 {
        local_datetime().hour() as f64
    }

    pub fn current_minute(&self) -> f64 {
        local_datetime().minute() as f64
    }

    pub fn current_second(&self) -> f64 {
        local_datetime().second() as f64
    }

    /// microphone input is unavailable until AIDA has an audio input device.
    pub fn loudness(&self) -> f64 {
        -1.0
    }

    pub fn set_variable_visible(&mut self, slot: usize, visible: bool) {
        if let Some(metadata) = self.variables.get_mut(slot)
            && metadata.monitor_visible != visible
        {
            metadata.monitor_visible = visible;
            self.request_presentation();
        }
    }

    /// return elapsed project time without adding clock state to script frames.
    pub fn project_timer_seconds(&self, now_millis: u64) -> f64 {
        self.project_timer_millis(now_millis) as f64 / 1000.0
    }

    pub fn project_timer_millis(&self, now_millis: u64) -> u64 {
        now_millis.saturating_sub(self.project_timer_started_at_millis)
    }

    pub fn observable_snapshot_with_timer(&self, tick: usize, now_millis: u64) -> RuntimeSnapshot {
        let mut snapshot = self.observable_snapshot(tick);
        snapshot.project_timer_millis = self.project_timer_millis(now_millis);
        snapshot.project_timer_epoch = self.project_timer_epoch;
        snapshot
    }

    pub fn list(&self, slot: usize) -> &ScratchList {
        self.instances[self.current_instance]
            .lists
            .get(slot)
            .and_then(Option::as_ref)
            .unwrap_or(&self.lists[slot])
    }

    pub fn list_mut(&mut self, slot: usize) -> &mut ScratchList {
        if self
            .list_metadata
            .get(slot)
            .is_some_and(|metadata| !metadata.is_stage)
        {
            self.ensure_instance_slot(slot);
            return self.instances[self.current_instance].lists[slot]
                .as_mut()
                .unwrap();
        }
        &mut self.lists[slot]
    }

    pub fn set_current_instance(&mut self, instance: usize) {
        self.current_instance = instance;
    }

    pub fn current_instance(&self) -> usize {
        self.current_instance
    }

    pub fn current_target(&self) -> usize {
        self.instances[self.current_instance].target
    }

    pub fn set_original_xy(&mut self, target: usize, x: f64, y: f64) {
        if let Some(instance) = self
            .instances
            .iter_mut()
            .find(|instance| instance.target == target && instance.original && !instance.is_stage)
        {
            instance.x = x;
            instance.y = y;
        }
    }

    pub fn set_target_fence_geometry(&mut self, target: usize, geometry: SpriteFenceGeometry) {
        if self.fence_geometry.len() <= target {
            self.fence_geometry.resize(target + 1, None);
        }
        self.fence_geometry[target] = Some(geometry);
    }

    pub fn set_original_visual(&mut self, target: usize, direction: f64, size: f64, visible: bool) {
        if let Some(instance) = self
            .instances
            .iter_mut()
            .find(|instance| instance.target == target && instance.original && !instance.is_stage)
        {
            if direction.is_finite() {
                instance.direction = scratch_direction(direction);
            }
            instance.size = size.max(0.0);
            instance.visible = visible;
        }
    }

    // register costume names and initialize the current costume for this target.
    pub fn set_target_costumes(&mut self, target: usize, names: Vec<String>, current: usize) {
        if let Some((_, existing)) = self
            .target_costumes
            .iter_mut()
            .find(|(index, _)| *index == target)
        {
            *existing = names;
        } else {
            self.target_costumes.push((target, names));
        }
        if let Some(instance) = self
            .instances
            .iter_mut()
            .find(|instance| instance.target == target && instance.original)
        {
            instance.costume_index = current;
        }
    }

    // resolve costume names and 1-based numeric choices using scratch's costume wrapping.
    pub fn switch_costume_to(&mut self, value: ScratchValue) {
        let target = self.instances[self.current_instance].target;
        let Some((_, names)) = self
            .target_costumes
            .iter()
            .find(|(index, _)| *index == target)
        else {
            return;
        };
        if names.is_empty() {
            return;
        }
        let choice = match value.scratch_view() {
            super::ScratchValueView::Number(number) => Some(number),
            super::ScratchValueView::Boolean(value) => Some(f64::from(value)),
            super::ScratchValueView::Text(text) => names
                .iter()
                .position(|name| name == text)
                .map(|index| index as f64 + 1.0)
                .or_else(|| text.parse::<f64>().ok()),
        };
        let Some(choice) = choice.filter(|number| number.is_finite()) else {
            return;
        };
        let rounded = (choice + 0.5).floor();
        let index = ((rounded - 1.0).rem_euclid(names.len() as f64)) as usize;
        let instance = &mut self.instances[self.current_instance];
        if instance.costume_index != index {
            instance.costume_index = index;
            self.request_redraw();
        }
    }

    // report the executing instance's selected costume using scratch's 1-based numbering.
    pub fn costume_number_name(&self, number: bool) -> ScratchValue {
        let instance = &self.instances[self.current_instance];
        let Some((_, names)) = self
            .target_costumes
            .iter()
            .find(|(index, _)| *index == instance.target)
        else {
            return ScratchValue::Text(String::new());
        };
        if number {
            ScratchValue::Number(instance.costume_index as f64 + 1.0)
        } else {
            ScratchValue::Text(
                names
                    .get(instance.costume_index)
                    .cloned()
                    .unwrap_or_default(),
            )
        }
    }

    // update only the executing target's visibility and request a visual frame.
    pub fn set_visible(&mut self, visible: bool) {
        let instance = &mut self.instances[self.current_instance];
        if !instance.is_stage && instance.visible != visible {
            instance.visible = visible;
            self.request_redraw();
        }
    }

    pub fn direction(&self) -> f64 {
        self.instances[self.current_instance].direction
    }

    pub fn direction_for_instance(&self, instance: usize) -> Option<f64> {
        self.instances
            .get(instance)
            .filter(|target| target.alive)
            .map(|target| target.direction)
    }

    pub fn set_direction(&mut self, direction: f64) {
        if direction.is_finite() && !self.instances[self.current_instance].is_stage {
            let direction = scratch_direction(direction);
            let instance = &mut self.instances[self.current_instance];
            if instance.direction != direction {
                instance.direction = direction;
                self.request_redraw();
            }
        }
    }

    /// update the current target's pen color using scratch's parameter ranges.
    pub fn set_pen_color_parameter(&mut self, parameter: ScratchValue, value: f64, change: bool) {
        let parameter = scratch_string(parameter);
        self.instances[self.current_instance]
            .pen_color
            .set_or_change_parameter(&parameter, value, change);
    }

    /// set the current target's pen color from a scratch rgb input.
    pub fn set_pen_rgb_color(&mut self, color: impl super::ScratchInput) {
        self.instances[self.current_instance]
            .pen_color
            .set_rgb_color(color);
    }

    pub fn clear_pen(&mut self) {
        self.pending_pen_commands.push(PenCommand::Clear);
        self.request_redraw();
    }

    pub fn pen_down(&mut self) {
        let instance_id = self.current_instance;
        let instance = &mut self.instances[instance_id];
        if instance.is_stage {
            return;
        }
        instance.pen_down = true;
        let point = [instance.x as f32, instance.y as f32];
        self.pending_pen_commands.push(PenCommand::Stroke {
            start: point,
            end: point,
            color: instance.pen_color.rgba,
            diameter: instance.pen_size as f32,
        });
        self.request_redraw();
    }

    pub fn pen_up(&mut self) {
        self.instances[self.current_instance].pen_down = false;
    }

    pub fn set_pen_size(&mut self, size: f64) {
        let size = scratch_to_number(size).clamp(1.0, 1200.0);
        self.instances[self.current_instance].pen_size = size;
    }

    pub fn change_pen_size(&mut self, change: f64) {
        let size = self.instances[self.current_instance].pen_size + scratch_to_number(change);
        self.set_pen_size(size);
    }

    pub fn stamp(&mut self) {
        let instance = &self.instances[self.current_instance];
        if instance.is_stage {
            return;
        }
        let Some((_, target_name, _)) = self
            .target_names
            .iter()
            .find(|(target, _, _)| *target == instance.target)
        else {
            return;
        };
        self.pending_pen_commands.push(PenCommand::Stamp {
            target_name: target_name.clone(),
            position: [instance.x as f32, instance.y as f32],
            direction: instance.direction as f32,
            scale: instance.size as f32 / 100.0,
            costume_index: instance.costume_index,
            effects_0: [0.0; 4],
            effects_1: [0.0; 4],
        });
        self.request_redraw();
    }

    pub fn take_pending_pen_commands(&mut self) -> Vec<PenCommand> {
        std::mem::take(&mut self.pending_pen_commands)
    }

    pub fn set_legacy_pen_hue(&mut self, hue: f64) {
        self.instances[self.current_instance]
            .pen_color
            .set_legacy_hue(hue);
    }

    pub fn change_legacy_pen_hue(&mut self, hue: f64) {
        self.instances[self.current_instance]
            .pen_color
            .change_legacy_hue(hue);
    }

    pub fn set_legacy_pen_shade(&mut self, shade: f64) {
        self.instances[self.current_instance]
            .pen_color
            .set_legacy_shade(shade);
    }

    pub fn change_legacy_pen_shade(&mut self, shade: f64) {
        self.instances[self.current_instance]
            .pen_color
            .change_legacy_shade(shade);
    }

    /// read the current target's calculated pen color for renderer commands.
    pub fn current_pen_rgba(&self) -> [f32; 4] {
        self.instances[self.current_instance].pen_color.rgba
    }

    pub fn set_xy(&mut self, x: f64, y: f64) {
        if self.instances[self.current_instance].is_stage {
            return;
        }
        let (x, y) = self.fenced_position(self.current_instance, x, y);
        let instance = &mut self.instances[self.current_instance];
        let old = [instance.x as f32, instance.y as f32];
        let changed = instance.x != x || instance.y != y;
        instance.x = x;
        instance.y = y;
        let new = [x as f32, y as f32];
        if changed && instance.pen_down {
            self.pending_pen_commands.push(PenCommand::Stroke {
                start: old,
                end: new,
                color: instance.pen_color.rgba,
                diameter: instance.pen_size as f32,
            });
        }
        if changed {
            self.request_redraw();
        }
    }

    fn fenced_position(&self, instance_id: usize, x: f64, y: f64) -> (f64, f64) {
        let instance = &self.instances[instance_id];
        let Some(geometry) = self.fence_geometry.get(instance.target).copied().flatten() else {
            return (x, y);
        };
        let radians = (90.0 - instance.direction).to_radians();
        let cos = radians.cos();
        let sin = radians.sin();
        let scale = instance.size / 100.0;
        let points = [
            (-geometry.rotation_center_x, geometry.rotation_center_y),
            (
                geometry.width - geometry.rotation_center_x,
                geometry.rotation_center_y,
            ),
            (
                -geometry.rotation_center_x,
                geometry.rotation_center_y - geometry.height,
            ),
            (
                geometry.width - geometry.rotation_center_x,
                geometry.rotation_center_y - geometry.height,
            ),
        ];
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for (local_x, local_y) in points {
            let tx = (local_x * cos - local_y * sin) * scale + instance.x;
            let ty = (local_x * sin + local_y * cos) * scale + instance.y;
            min_x = min_x.min(tx);
            max_x = max_x.max(tx);
            min_y = min_y.min(ty);
            max_y = max_y.max(ty);
        }
        let inset = (((max_x - min_x).min(max_y - min_y) / 2.0).floor()).max(0.0);
        let margin = 15.0_f64.min(inset);
        let fence_x = 240.0 - margin;
        let fence_y = 180.0 - margin;
        let dx = x - instance.x;
        let dy = y - instance.y;
        let mut fenced_x = x;
        let mut fenced_y = y;
        if max_x + dx < -fence_x {
            fenced_x = (instance.x - (fence_x + max_x)).ceil();
        } else if min_x + dx > fence_x {
            fenced_x = (instance.x + (fence_x - min_x)).floor();
        }
        if max_y + dy < -fence_y {
            fenced_y = (instance.y - (fence_y + max_y)).ceil();
        } else if min_y + dy > fence_y {
            fenced_y = (instance.y + (fence_y - min_y)).floor();
        }
        (fenced_x, fenced_y)
    }

    pub fn change_x_by(&mut self, dx: f64) {
        if let Some((x, y)) = self.position(self.current_instance) {
            self.set_xy(x + dx, y);
        }
    }

    pub fn change_y_by(&mut self, dy: f64) {
        if let Some((x, y)) = self.position(self.current_instance) {
            self.set_xy(x, y + dy);
        }
    }

    pub fn position(&self, instance: usize) -> Option<(f64, f64)> {
        self.instances
            .get(instance)
            .filter(|target| target.alive)
            .map(|target| (target.x, target.y))
    }

    pub fn add_original_target(&mut self, target: usize) -> usize {
        if let Some(id) = self.original_instance(target) {
            return id;
        }
        let id = self.instances.len();
        let is_stage = self
            .target_names
            .iter()
            .find(|(index, _, _)| *index == target)
            .is_some_and(|(_, _, is_stage)| *is_stage);
        self.instances.push(ScratchTargetInstance {
            target,
            original: true,
            alive: true,
            is_stage,
            x: 0.0,
            y: 0.0,
            direction: 90.0,
            size: 100.0,
            visible: true,
            values: vec![None; self.values.len()],
            lists: vec![None; self.lists.len()],
            bubble: None,
            pen_color: PenColorState::default(),
            pen_down: false,
            pen_size: 1.0,
            costume_index: 0,
        });
        id
    }

    pub fn clone_target_for_name(&self, name: &str) -> Option<usize> {
        self.target_names
            .iter()
            .find(|(_, target_name, is_stage)| target_name == name && !is_stage)
            .map(|(target, _, _)| *target)
    }

    pub fn register_target(&mut self, target: usize, name: impl Into<String>, is_stage: bool) {
        self.target_names.push((target, name.into(), is_stage));
        let instance = self.add_original_target(target);
        self.instances[instance].is_stage = is_stage;
    }

    fn ensure_instance_slot(&mut self, slot: usize) {
        let instance = &mut self.instances[self.current_instance];
        if instance.values.len() < self.values.len() {
            instance.values.resize(self.values.len(), None);
        }
        if instance.lists.len() < self.lists.len() {
            instance.lists.resize(self.lists.len(), None);
        }
        if let Some(metadata) = self.variables.get(slot) {
            if instance.values[slot].is_none() && metadata.target_index == instance.target {
                instance.values[slot] = Some(self.values[slot].clone());
            }
        }
        if let Some(metadata) = self.list_metadata.get(slot) {
            if instance.lists[slot].is_none() && metadata.target_index == instance.target {
                instance.lists[slot] = Some(self.lists[slot].clone());
            }
        }
    }

    pub fn create_clone(
        &mut self,
        source_instance: usize,
        target: usize,
        max_clones: usize,
    ) -> Option<usize> {
        if !self
            .instances
            .get(source_instance)
            .is_some_and(|instance| instance.alive)
            || self.clone_count() >= max_clones
        {
            return None;
        }
        if self
            .target_names
            .iter()
            .any(|(index, _, is_stage)| *index == target && *is_stage)
        {
            return None;
        }
        let mut instance = ScratchTargetInstance {
            target,
            original: false,
            alive: true,
            is_stage: false,
            x: self.instances[source_instance].x,
            y: self.instances[source_instance].y,
            direction: self.instances[source_instance].direction,
            size: self.instances[source_instance].size,
            visible: self.instances[source_instance].visible,
            values: vec![None; self.values.len()],
            lists: vec![None; self.lists.len()],
            bubble: None,
            pen_color: self.instances[source_instance].pen_color,
            pen_down: self.instances[source_instance].pen_down,
            pen_size: self.instances[source_instance].pen_size,
            costume_index: self.instances[source_instance].costume_index,
        };
        for (slot, metadata) in self.variables.iter().enumerate() {
            if !metadata.is_stage && metadata.target_index == target {
                instance.values[slot] = Some(
                    if metadata.target_index == self.instances[source_instance].target {
                        self.instances[source_instance]
                            .values
                            .get(slot)
                            .and_then(Option::as_ref)
                            .unwrap_or(&self.values[slot])
                            .clone()
                    } else {
                        self.values[slot].clone()
                    },
                );
            }
        }
        for (slot, metadata) in self.list_metadata.iter().enumerate() {
            if !metadata.is_stage && metadata.target_index == target {
                instance.lists[slot] = Some(
                    if metadata.target_index == self.instances[source_instance].target {
                        self.instances[source_instance]
                            .lists
                            .get(slot)
                            .and_then(Option::as_ref)
                            .unwrap_or(&self.lists[slot])
                            .clone()
                    } else {
                        self.lists[slot].clone()
                    },
                );
            }
        }
        let id = self.instances.len();
        self.instances.push(instance);
        self.pending_clones.push(id);
        self.request_redraw();
        Some(id)
    }

    pub fn clone_count(&self) -> usize {
        self.instances
            .iter()
            .filter(|instance| !instance.original && instance.alive)
            .count()
    }

    pub fn take_pending_clones(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.pending_clones)
    }

    pub fn instance_target(&self, id: usize) -> Option<usize> {
        self.instances
            .get(id)
            .filter(|instance| instance.alive)
            .map(|instance| instance.target)
    }

    pub fn instances_for_target(&self, target: usize) -> Vec<usize> {
        self.instances
            .iter()
            .enumerate()
            .filter_map(|(id, instance)| {
                (instance.target == target && instance.alive).then_some(id)
            })
            .collect()
    }

    pub fn delete_current_clone(&mut self) -> bool {
        let instance = &mut self.instances[self.current_instance];
        if instance.original {
            return false;
        }
        instance.alive = false;
        instance.values.clear();
        instance.lists.clear();
        self.request_redraw();
        true
    }

    pub fn remove_clones(&mut self) {
        let mut removed_any = false;
        for instance in &mut self.instances {
            if !instance.original {
                removed_any |= instance.alive;
                instance.alive = false;
                instance.values.clear();
                instance.lists.clear();
            }
        }
        self.pending_clones.clear();
        if removed_any {
            self.request_redraw();
        }
    }

    pub fn instance_is_alive(&self, id: usize) -> bool {
        self.instances
            .get(id)
            .is_some_and(|instance| instance.alive)
    }

    pub fn original_instance(&self, target: usize) -> Option<usize> {
        self.instances
            .iter()
            .position(|instance| instance.target == target && instance.original && instance.alive)
    }

    pub fn list_visible(&self, slot: usize) -> bool {
        self.list_visibility[slot]
    }

    pub fn set_list_visible(&mut self, slot: usize, visible: bool) {
        if self.list_visibility[slot] != visible {
            self.list_visibility[slot] = visible;
            self.request_presentation();
        }
    }

    pub fn list_before_change(&self, slot: usize) -> Option<ScratchList> {
        if !self.list_visibility.get(slot).copied().unwrap_or(false) {
            return None;
        }
        if let Some(metadata) = self.list_metadata.get(slot)
            && !metadata.is_stage
            && (!self.instances[self.current_instance].original
                || self.instances[self.current_instance].target != metadata.target_index)
        {
            return None;
        }
        Some(self.list(slot).clone())
    }

    pub fn list_changed(&mut self, slot: usize, before: Option<ScratchList>) {
        if before.is_some_and(|before| !before.shares_storage_with(self.list(slot))) {
            self.request_presentation();
        }
    }

    /// look up a list by its Scratch ID.
    pub fn list_by_id(&self, id: &str) -> Option<&ScratchList> {
        let slot = self.list_metadata.iter().position(|list| list.id == id)?;
        Some(self.list(slot))
    }

    /// look up a target list by name, falling back to a stage list.
    pub fn list_by_name(&self, target: &str, name: &str) -> Option<&ScratchList> {
        let slot = self
            .list_metadata
            .iter()
            .position(|list| list.target == target && list.name == name)
            .or_else(|| {
                self.list_metadata
                    .iter()
                    .position(|list| list.is_stage && list.name == name)
            })?;
        Some(self.list(slot))
    }

    /// look up a variable by its Scratch ID.
    pub fn get_by_id(&self, id: &str) -> Option<&ScratchValue> {
        let slot = self
            .variables
            .iter()
            .position(|variable| variable.id == id)?;
        Some(self.get(slot))
    }

    /// look up a target variable by name, falling back to a stage variable.
    pub fn get_by_name(&self, target: &str, name: &str) -> Option<&ScratchValue> {
        let slot = self
            .variables
            .iter()
            .position(|variable| variable.target == target && variable.name == name)
            .or_else(|| {
                self.variables
                    .iter()
                    .position(|variable| variable.is_stage && variable.name == name)
            })?;
        Some(self.get(slot))
    }
}

fn local_datetime() -> OffsetDateTime {
    OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
}

#[cfg(test)]
mod tests {
    use super::{
        BubbleKind, ObservableEvent, RuntimeInputEvent, ScratchListMetadata, ScratchState,
        ScratchValue, ScratchVariableMetadata, SpriteFenceGeometry,
    };
    use crate::runtime::PenCommand;
    use crate::runtime::{ScratchList, list_snapshot_key, scratch_to_number};

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

        assert!(matches!(commands.as_slice(), [
            PenCommand::Stroke { start: [10.0, 20.0], end: [10.0, 20.0], color: [0.0, 1.0, 0.0, 1.0], diameter: 6.0 },
            PenCommand::Stroke { start: [10.0, 20.0], end: [30.0, 40.0], color: [0.0, 1.0, 0.0, 1.0], diameter: 6.0 },
            PenCommand::Stamp { target_name, position: [30.0, 40.0], costume_index: 1, .. },
            PenCommand::Clear,
            PenCommand::Stroke { start: [30.0, 40.0], end: [50.0, 60.0], color: [0.0, 1.0, 0.0, 1.0], diameter: 6.0 }
        ] if target_name == "Sprite1"));
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

        assert!(matches!(
            commands.as_slice(),
            [
                PenCommand::Stroke { .. },
                PenCommand::Stroke {
                    start: [0.0, 0.0],
                    end: [5.0, 7.0],
                    color: [0.0, 1.0, 0.0, 1.0],
                    diameter: 6.0
                },
                PenCommand::Stroke {
                    start: [5.0, 7.0],
                    end: [8.0, 9.0],
                    color: [1.0, 0.0, 0.0, 1.0],
                    diameter: 2.0
                },
                PenCommand::Stroke {
                    start: [0.0, 0.0],
                    end: [2.0, 3.0],
                    color: [0.0, 1.0, 0.0, 1.0],
                    diameter: 6.0
                }
            ]
        ));
        assert_eq!(state.instances[clone].costume_index, 1);
    }

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
            },
        );
        state.set_current_instance(state.original_instance(1).unwrap());
        state
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
        assert!(
            matches!(state.get_shared(4), ScratchValue::Number(value) if value.is_sign_negative())
        );
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

    #[test]
    fn redraw_gate_and_input_are_owned_by_canonical_runtime_state() {
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
            None
        );
        state.apply_input(RuntimeInputEvent::MousePosition { x: 21.0, y: -7.0 });
        state.apply_input(RuntimeInputEvent::MouseButton { pressed: true });
        assert!(state.key_pressed("a"));
        assert_eq!((state.mouse_x(), state.mouse_y()), (21.0, -7.0));
        assert!(state.mouse_down());
        state.apply_input(RuntimeInputEvent::Key {
            name: "a".to_owned(),
            pressed: false,
        });
        assert!(!state.key_pressed("a"));
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
        let before = state.list_before_change(0);
        state.list_mut(0).add("after".into());
        state.list_changed(0, before);
        assert!(state.presentation_requested());
        assert!(!state.redraw_requested());
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
}
