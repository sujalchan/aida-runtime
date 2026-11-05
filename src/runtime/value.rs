#[derive(Debug, Clone, PartialEq)]
pub enum ScratchValue {
    Number(f64),
    Text(String),
    Boolean(bool),
    /// undefined represents results from legacy Scratch VM reporter no-ops.
    Undefined,
}

use super::{
    PenColorParameter, PenColorState, PenCommand, PenDot, PenStroke, RealtimeAudio, ScratchList,
    SoundLibrary, SoundParameters, SoundRuntime, scratch_number_operand, scratch_random,
    scratch_string, scratch_to_number,
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
        ScratchValue::Text(_) | ScratchValue::Boolean(_) | ScratchValue::Undefined => {
            *value = ScratchValue::Number(scratch_to_number(&*value) + change);
        }
    }
}

fn scratch_direction(direction: f64) -> f64 {
    direction - (((direction + 179.0) / 360.0).floor() * 360.0)
}

// match scratch's display precision without changing the stored target coordinates.
pub fn scratch_motion_coordinate(coordinate: f64) -> f64 {
    let rounded = coordinate.round();
    if (coordinate - rounded).abs() < 1e-9 {
        rounded
    } else {
        coordinate
    }
}

fn costume_corners(geometry: &SpriteFenceGeometry) -> [(f64, f64); 4] {
    [
        (0.0, 0.0),
        (geometry.width, 0.0),
        (0.0, geometry.height),
        (geometry.width, geometry.height),
    ]
}

fn transform_costume_point(
    x: f64,
    y: f64,
    rotation_center_x: f64,
    rotation_center_y: f64,
    cos: f64,
    sin: f64,
    flip_x: f64,
    scale: f64,
) -> (f64, f64) {
    let local_x = (x - rotation_center_x) * flip_x * scale;
    let local_y = (rotation_center_y - y) * scale;
    (local_x * cos - local_y * sin, local_x * sin + local_y * cos)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScratchRotationStyle {
    #[default]
    AllAround,
    LeftRight,
    DontRotate,
}

impl ScratchRotationStyle {
    pub fn from_scratch(value: &str) -> Option<Self> {
        match value {
            "all around" => Some(Self::AllAround),
            "left-right" => Some(Self::LeftRight),
            "don't rotate" => Some(Self::DontRotate),
            _ => None,
        }
    }

    pub fn rendered_direction_and_flip(self, direction: f64) -> (f64, f64) {
        match self {
            Self::AllAround => (direction, 1.0),
            Self::LeftRight => (90.0, if direction < 0.0 { -1.0 } else { 1.0 }),
            Self::DontRotate => (90.0, 1.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScratchSoundEffect {
    Pitch,
    Pan,
}

/// input collected by the presentation thread and consumed by generated code.
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeInputEvent {
    Key {
        name: String,
        pressed: bool,
    },
    // release all held keys when the window loses focus.
    ClearKeys,
    MousePosition {
        x: f64,
        y: f64,
    },
    MouseButton {
        pressed: bool,
    },
    // carry renderer-picked click targets to the scheduler without gpu state.
    PointerButton {
        pressed: bool,
        inside_stage: bool,
        target_instance: Option<usize>,
        draggable: bool,
        was_dragged: bool,
        monitor_interaction: bool,
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

// mirror Scratch's mouse-down versus mouse-up activation rule after renderer picking.
pub fn pointer_click_activates(
    pressed: bool,
    inside_stage: bool,
    draggable: bool,
    was_dragged: bool,
    monitor_interaction: bool,
) -> bool {
    inside_stage
        && !was_dragged
        && !monitor_interaction
        && if draggable { !pressed } else { pressed }
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
                let Some(name) = scratch_key_from_host(&name) else {
                    return (None, false);
                };
                if pressed {
                    let changed = self.pressed_keys.insert(name.clone());
                    (Some(name.to_lowercase()), changed)
                } else {
                    let changed = self.pressed_keys.remove(&name);
                    (None, changed)
                }
            }
            RuntimeInputEvent::ClearKeys => {
                let changed = !self.pressed_keys.is_empty();
                self.pressed_keys.clear();
                (None, changed)
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
            RuntimeInputEvent::PointerButton { .. } => (None, false),
            RuntimeInputEvent::VariableValue { .. }
            | RuntimeInputEvent::ListAppend { .. }
            | RuntimeInputEvent::ListSetItem { .. }
            | RuntimeInputEvent::ListDeleteItem { .. }
            | RuntimeInputEvent::ListReplace { .. } => (None, false),
        }
    }
}

// match the key names accepted from a logical host keyboard event by scratch vm.
fn scratch_key_from_host(key: &str) -> Option<String> {
    let special = match key {
        " " => Some("space"),
        "ArrowLeft" | "Left" => Some("left arrow"),
        "ArrowUp" | "Up" => Some("up arrow"),
        "ArrowRight" | "Right" => Some("right arrow"),
        "ArrowDown" | "Down" => Some("down arrow"),
        "Enter" => Some("enter"),
        _ => None,
    };
    if let Some(special) = special {
        return Some(special.to_owned());
    }

    // scratch ignores modifier and named keys whose logical name has multiple code units.
    if key.encode_utf16().count() != 1 {
        return None;
    }
    Some(key.to_uppercase())
}

// convert the sensing block argument separately from host events, as scratch does.
fn scratch_key_from_argument(value: ScratchValueView<'_>) -> Option<String> {
    if let ScratchValueView::Number(number) = value {
        if (48.0..=90.0).contains(&number) {
            return char::from_u32(number as u32).map(|key| key.to_uppercase().to_string());
        }
        let key = match number {
            32.0 => "space",
            37.0 => "left arrow",
            38.0 => "up arrow",
            39.0 => "right arrow",
            40.0 => "down arrow",
            _ => "",
        };
        if !key.is_empty() {
            return Some(key.to_owned());
        }
    }

    let ScratchValueView::Text(text) = value else {
        let text = scratch_string(match value {
            ScratchValueView::Number(number) => ScratchValue::Number(number),
            ScratchValueView::Boolean(value) => ScratchValue::Boolean(value),
            ScratchValueView::Undefined => ScratchValue::Undefined,
            ScratchValueView::Text(_) => unreachable!(),
        });
        return scratch_key_from_text(&text);
    };
    if matches!(
        text,
        "space" | "left arrow" | "up arrow" | "right arrow" | "down arrow" | "enter"
    ) {
        return Some(text.to_owned());
    }
    scratch_key_from_text(text)
}

fn scratch_key_from_text(text: &str) -> Option<String> {
    let key = text.chars().next()?;
    if key == ' ' {
        Some("space".to_owned())
    } else {
        Some(key.to_uppercase().to_string())
    }
}

/// a borrowed view of a scratch value accepted by runtime operators.
#[derive(Debug, Clone, Copy)]
pub enum ScratchValueView<'a> {
    Number(f64),
    Text(&'a str),
    Boolean(bool),
    Undefined,
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
            Self::Undefined => ScratchValueView::Undefined,
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
    loudness: f64,
    pending_pen_commands: Vec<PenCommand>,
    sound_runtime: SoundRuntime,
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
    pub rotation_style: ScratchRotationStyle,
    pub size: f64,
    pub visible: bool,
    // preserve costume selection per sprite instance in renderer snapshots.
    pub costume_index: usize,
    pub bubble: Option<BubbleSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpriteFenceGeometry {
    pub width: f64,
    pub height: f64,
    pub rotation_center_x: f64,
    pub rotation_center_y: f64,
    pub bounds_hull: Option<Vec<[f64; 2]>>,
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

impl RuntimeSnapshot {
    // release hidden list storage before a graphical snapshot crosses threads.
    pub(crate) fn release_hidden_list_items(&mut self) {
        static EMPTY_ITEMS: std::sync::LazyLock<Arc<Vec<ScratchValue>>> =
            std::sync::LazyLock::new(|| Arc::new(Vec::new()));
        for list in &mut self.lists {
            if !list.visible {
                list.items = Arc::clone(&EMPTY_ITEMS);
            }
        }
    }
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

#[derive(Debug, Clone, Copy)]
struct CachedFenceOffsets {
    geometry: [f64; 4],
    direction: f64,
    size: f64,
    rotation_style: ScratchRotationStyle,
    points: [(f64, f64); 4],
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
    rotation_style: ScratchRotationStyle,
    size: f64,
    visible: bool,
    values: Vec<Option<ScratchValue>>,
    lists: Vec<Option<ScratchList>>,
    bubble: Option<SpeechBubble>,
    pen_color: PenColorState,
    pen_down: bool,
    pen_size: f64,
    costume_index: usize,
    fence_offsets: Option<CachedFenceOffsets>,
    sound_pitch: f64,
    sound_pan: f64,
    volume: f64,
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

    /// request an observable snapshot without forcing a scheduler redraw.
    pub fn request_presentation(&mut self) {
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

    // keep reporter argument conversion separate from normalized host key state.
    pub fn key_pressed(&self, key: impl ScratchInput) -> bool {
        let value = key.scratch_view();
        if matches!(value, ScratchValueView::Text("any")) {
            return !self.input.pressed_keys.is_empty();
        }
        scratch_key_from_argument(value).is_some_and(|key| self.input.pressed_keys.contains(&key))
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
            super::ScratchValueView::Undefined => None,
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

    /// store a compiler-proven numeric stage value without cloning monitor state.
    #[inline]
    pub fn set_shared_number(&mut self, slot: usize, value: f64) {
        if self.event_sink.is_some() {
            self.set_shared(slot, ScratchValue::Number(value));
            return;
        }
        let Some(ScratchValue::Number(previous)) = self.values.get(slot) else {
            self.set_shared(slot, ScratchValue::Number(value));
            return;
        };
        let changed = *previous != value;
        let notify = self
            .variables
            .get(slot)
            .is_some_and(|metadata| metadata.monitor_visible);
        self.values[slot] = ScratchValue::Number(value);
        if changed && notify {
            self.request_presentation();
        }
    }

    /// store a compiler-proven numeric sprite value without cloning monitor state.
    #[inline]
    pub fn set_local_number(&mut self, slot: usize, value: f64) {
        if self.event_sink.is_some() {
            self.set(slot, ScratchValue::Number(value));
            return;
        }
        let current_instance = self.current_instance;
        let Some(metadata) = self.variables.get(slot) else {
            self.set(slot, ScratchValue::Number(value));
            return;
        };
        let instance = &self.instances[current_instance];
        let previous = instance
            .values
            .get(slot)
            .and_then(Option::as_ref)
            .unwrap_or(&self.values[slot]);
        let ScratchValue::Number(previous) = previous else {
            self.set(slot, ScratchValue::Number(value));
            return;
        };
        let changed = *previous != value;
        let notify = metadata.monitor_visible
            && instance.original
            && instance.target == metadata.target_index;
        if self.instances[current_instance].values.len() < self.values.len() {
            self.instances[current_instance]
                .values
                .resize(self.values.len(), None);
        }
        self.instances[current_instance].values[slot] = Some(ScratchValue::Number(value));
        if changed && notify {
            self.request_presentation();
        }
    }

    /// Update an existing numeric sprite slot in place in a synchronous batch.
    #[inline(always)]
    pub fn set_existing_local_number(&mut self, slot: usize, value: f64) {
        if self.event_sink.is_none()
            && let Some(instance) = self.instances.get_mut(self.current_instance)
            && let Some(Some(ScratchValue::Number(previous))) = instance.values.get_mut(slot)
        {
            let changed = *previous != value;
            let notify = self.variables.get(slot).is_some_and(|metadata| {
                metadata.monitor_visible
                    && instance.original
                    && instance.target == metadata.target_index
            });
            *previous = value;
            if changed && notify {
                self.request_presentation();
            }
            return;
        }
        self.set_local_number(slot, value);
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

    /// update a compiler-proven numeric stage value without cloning monitor state.
    #[inline]
    pub fn change_shared_number(&mut self, slot: usize, change: f64) {
        if self.event_sink.is_some() {
            self.change_shared(slot, change);
            return;
        }
        let Some(ScratchValue::Number(previous)) = self.values.get(slot) else {
            self.change_shared(slot, change);
            return;
        };
        let previous = *previous;
        let next = scratch_number_operand(previous) + change;
        let notify = self
            .variables
            .get(slot)
            .is_some_and(|metadata| metadata.monitor_visible);
        self.values[slot] = ScratchValue::Number(next);
        if notify && previous != next {
            self.request_presentation();
        }
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

    /// update a compiler-proven numeric sprite value without cloning monitor state.
    #[inline]
    pub fn change_local_number(&mut self, slot: usize, change: f64) {
        if self.event_sink.is_some() {
            self.change_local_by(slot, change);
            return;
        }
        let current_instance = self.current_instance;
        let Some(metadata) = self.variables.get(slot) else {
            self.change_local_by(slot, change);
            return;
        };
        let instance = &self.instances[current_instance];
        let previous = instance
            .values
            .get(slot)
            .and_then(Option::as_ref)
            .unwrap_or(&self.values[slot]);
        let ScratchValue::Number(previous) = previous else {
            self.change_local_by(slot, change);
            return;
        };
        let previous = *previous;
        let next = scratch_number_operand(previous) + change;
        let notify = metadata.monitor_visible
            && instance.original
            && instance.target == metadata.target_index;
        if self.instances[current_instance].values.len() < self.values.len() {
            self.instances[current_instance]
                .values
                .resize(self.values.len(), None);
        }
        self.instances[current_instance].values[slot] = Some(ScratchValue::Number(next));
        if notify && previous != next {
            self.request_presentation();
        }
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
                    rotation_style: instance.rotation_style,
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
        self.loudness
    }

    /// set the latest audio input sample without polling from script execution.
    pub fn set_loudness(&mut self, loudness: f64) {
        let next = if loudness.is_finite() { loudness } else { -1.0 };
        if self.loudness != next {
            self.loudness = next;
            self.request_presentation();
        }
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

    /// Run a synchronous generated batch with a Stage list borrowed once.
    /// The generated closure must access this list only through `items`; it
    /// cannot invoke or yield to another script before returning.
    pub fn with_stage_numeric_list_batch<R>(
        &mut self,
        slot: usize,
        run: impl FnOnce(&mut Self, &mut super::list::NumericListReplacements<'_>) -> R,
    ) -> R {
        debug_assert!(self.list_metadata[slot].is_stage);
        let mut list = std::mem::replace(&mut self.lists[slot], ScratchList::batch_placeholder());
        let result = list.with_numeric_replacements(|items| run(self, items));
        self.lists[slot] = list;
        result
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

    pub fn set_original_volume(&mut self, target: usize, volume: f64) {
        if let Some(instance) = self
            .instances
            .iter_mut()
            .find(|instance| instance.target == target && instance.original)
        {
            instance.volume = scratch_to_number(volume).clamp(0.0, 100.0);
        }
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
        for instance in self
            .instances
            .iter_mut()
            .filter(|instance| instance.target == target)
        {
            instance.fence_offsets = None;
        }
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

    pub fn set_original_rotation_style(&mut self, target: usize, style: &str) {
        let Some(style) = ScratchRotationStyle::from_scratch(style) else {
            return;
        };
        if let Some(instance) = self
            .instances
            .iter_mut()
            .find(|instance| instance.target == target && instance.original && !instance.is_stage)
        {
            instance.rotation_style = style;
            instance.fence_offsets = None;
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
            super::ScratchValueView::Undefined => None,
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

    pub fn x_position(&self) -> f64 {
        scratch_motion_coordinate(self.instances[self.current_instance].x)
    }

    pub fn y_position(&self) -> f64 {
        scratch_motion_coordinate(self.instances[self.current_instance].y)
    }

    pub fn set_rotation_style(&mut self, style: &str) {
        let Some(style) = ScratchRotationStyle::from_scratch(style) else {
            return;
        };
        let instance = &mut self.instances[self.current_instance];
        if instance.is_stage || instance.rotation_style == style {
            return;
        }
        instance.rotation_style = style;
        instance.fence_offsets = None;
        self.request_redraw();
    }

    pub fn move_steps(&mut self, steps: f64) {
        let instance = &self.instances[self.current_instance];
        if instance.is_stage {
            return;
        }
        let radians = (90.0 - instance.direction) * std::f64::consts::PI / 180.0;
        let x = instance.x + steps * radians.cos();
        let y = instance.y + steps * radians.sin();
        self.set_xy(x, y);
    }

    pub fn bounce_on_edge(&mut self) {
        let instance_id = self.current_instance;
        let Some(bounds) = self.transformed_bounds(instance_id, true) else {
            return;
        };
        let left = (240.0 + bounds[0]).max(0.0);
        let top = (180.0 - bounds[3]).max(0.0);
        let right = (240.0 - bounds[1]).max(0.0);
        let bottom = (180.0 + bounds[2]).max(0.0);
        let mut edge = 0;
        let mut minimum = f64::INFINITY;
        for (index, distance) in [left, top, right, bottom].into_iter().enumerate() {
            if distance < minimum {
                minimum = distance;
                edge = index;
            }
        }
        if minimum > 0.0 {
            return;
        }

        let direction = self.instances[instance_id].direction;
        let radians = (90.0 - direction) * std::f64::consts::PI / 180.0;
        let mut dx = radians.cos();
        let mut dy = -radians.sin();
        match edge {
            0 => dx = dx.abs().max(0.2),
            1 => dy = dy.abs().max(0.2),
            2 => dx = -dx.abs().max(0.2),
            _ => dy = -dy.abs().max(0.2),
        }
        self.set_direction(dy.atan2(dx) * 180.0 / std::f64::consts::PI + 90.0);
        self.keep_current_instance_in_stage();
    }

    fn keep_current_instance_in_stage(&mut self) {
        let instance_id = self.current_instance;
        // scratch uses the opaque bounds for edge bounce, then keeps that outline inside the stage.
        let Some(bounds) = self.transformed_bounds(instance_id, true) else {
            return;
        };
        let mut dx = 0.0;
        let mut dy = 0.0;
        if bounds[0] < -240.0 {
            dx += -240.0 - bounds[0];
        }
        if bounds[1] > 240.0 {
            dx += 240.0 - bounds[1];
        }
        if bounds[3] > 180.0 {
            dy += 180.0 - bounds[3];
        }
        if bounds[2] < -180.0 {
            dy += -180.0 - bounds[2];
        }
        let instance = &self.instances[instance_id];
        self.set_xy(instance.x + dx, instance.y + dy);
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
    // accept the generated menu string directly so fixed dropdowns allocate nothing per block.
    pub fn set_pen_color_parameter(
        &mut self,
        parameter: impl Into<PenColorParameter>,
        value: f64,
        change: bool,
    ) {
        self.instances[self.current_instance]
            .pen_color
            .set_or_change_parameter(parameter, value, change);
    }

    /// set the current target's pen color from a scratch rgb input.
    pub fn set_pen_rgb_color(&mut self, color: impl super::ScratchInput) {
        self.instances[self.current_instance]
            .pen_color
            .set_rgb_color(color);
    }

    /// use the generated fast path when this target has no hsv-based pen controls.
    pub fn set_pen_rgb_color_without_hsv(&mut self, color: impl super::ScratchInput) {
        self.instances[self.current_instance]
            .pen_color
            .set_rgb_color_without_hsv(color);
    }

    pub fn clear_pen(&mut self) {
        self.pending_pen_commands.push(PenCommand::Clear);
        self.request_redraw();
    }

    pub fn pen_down(&mut self) {
        let instance_id = self.current_instance;
        let stroke = {
            let instance = &mut self.instances[instance_id];
            if instance.is_stage {
                return;
            }
            instance.pen_down = true;
            let point = [instance.x as f32, instance.y as f32];
            PenDot {
                position: point,
                color: instance.pen_color.rgba,
                diameter: instance.pen_size as f32,
                pixel_offset: 0.0,
            }
        };
        self.push_pen_dot(stroke);
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

    pub fn set_sound_library(&mut self, library: SoundLibrary) {
        self.sound_runtime.set_library(library);
    }

    pub fn set_realtime_audio(&mut self, output: RealtimeAudio) {
        self.sound_runtime.set_output(output);
    }

    pub fn start_sound(&mut self, selector: ScratchValue, now_millis: u64) -> Option<u64> {
        let instance = self.current_instance;
        let target = self.instances.get(instance)?.target;
        let parameters = self.sound_parameters(instance);
        self.sound_runtime
            .start(target, instance, selector, parameters, now_millis)
    }

    pub fn set_sound_effect(
        &mut self,
        effect: ScratchSoundEffect,
        value: f64,
        change: bool,
        now_millis: u64,
    ) {
        let instance_id = self.current_instance;
        let instance = &mut self.instances[instance_id];
        let value = scratch_to_number(value);
        match effect {
            ScratchSoundEffect::Pitch => {
                let next = if change {
                    instance.sound_pitch + value
                } else {
                    value
                };
                instance.sound_pitch = next.clamp(-360.0, 360.0);
            }
            ScratchSoundEffect::Pan => {
                let next = if change {
                    instance.sound_pan + value
                } else {
                    value
                };
                instance.sound_pan = next.clamp(-100.0, 100.0);
            }
        }
        let target = instance.target;
        let parameters = self.sound_parameters(instance_id);
        self.sound_runtime
            .update_instance(target, instance_id, parameters, now_millis);
    }

    pub fn clear_sound_effects(&mut self, now_millis: u64) {
        let instance_id = self.current_instance;
        let instance = &mut self.instances[instance_id];
        instance.sound_pitch = 0.0;
        instance.sound_pan = 0.0;
        let target = instance.target;
        let parameters = self.sound_parameters(instance_id);
        self.sound_runtime
            .update_instance(target, instance_id, parameters, now_millis);
    }

    pub fn reset_sound_effects(&mut self, now_millis: u64) {
        for instance_id in 0..self.instances.len() {
            let instance = &mut self.instances[instance_id];
            instance.sound_pitch = 0.0;
            instance.sound_pan = 0.0;
            let parameters = SoundParameters {
                pitch: instance.sound_pitch,
                pan: instance.sound_pan,
                volume: instance.volume,
            };
            self.sound_runtime.update_instance(
                instance.target,
                instance_id,
                parameters,
                now_millis,
            );
        }
    }

    pub fn set_sound_volume(&mut self, value: f64, change: bool, now_millis: u64) {
        let instance_id = self.current_instance;
        let instance = &mut self.instances[instance_id];
        let value = scratch_to_number(value);
        let next = if change {
            instance.volume + value
        } else {
            value
        };
        instance.volume = next.clamp(0.0, 100.0);
        let target = instance.target;
        let parameters = self.sound_parameters(instance_id);
        self.sound_runtime
            .update_instance(target, instance_id, parameters, now_millis);
    }

    pub fn sound_volume(&self) -> f64 {
        self.instances[self.current_instance].volume
    }

    /// read a sprite or stage's original target volume for its reporter monitor.
    pub fn sound_volume_for_target(&self, target: usize) -> f64 {
        self.original_instance(target)
            .map_or(100.0, |instance| self.instances[instance].volume)
    }

    pub fn stop_all_sounds(&mut self, now_millis: u64) {
        self.sound_runtime.advance_to(now_millis);
        self.sound_runtime.stop_all();
    }

    pub fn restart_sounds(&mut self, now_millis: u64) {
        self.stop_all_sounds(now_millis);
        self.reset_sound_effects(now_millis);
    }

    pub fn stop_project_sounds(&mut self, now_millis: u64) {
        self.restart_sounds(now_millis);
    }

    pub fn advance_sounds(&mut self, now_millis: u64) {
        self.sound_runtime.advance_to(now_millis);
    }

    pub fn drain_finished_sounds(&mut self) -> Vec<u64> {
        self.sound_runtime.drain_completed()
    }

    pub fn has_active_sounds(&self) -> bool {
        self.sound_runtime.has_active()
    }

    pub fn take_audio_events(&mut self) -> Vec<super::AudioEvent> {
        self.sound_runtime.take_events()
    }

    fn sound_parameters(&self, instance: usize) -> SoundParameters {
        let instance = &self.instances[instance];
        SoundParameters {
            pitch: instance.sound_pitch,
            pan: instance.sound_pan,
            volume: instance.volume,
        }
    }

    pub fn take_pending_pen_commands(&mut self) -> Vec<PenCommand> {
        std::mem::take(&mut self.pending_pen_commands)
    }

    /// report whether ordered pen marks can be accumulated before presentation.
    pub fn has_pending_pen_commands(&self) -> bool {
        !self.pending_pen_commands.is_empty()
    }

    #[inline]
    fn push_pen_dot(&mut self, dot: PenDot) {
        // group adjacent pen-down points into a compact renderer upload.
        match self.pending_pen_commands.last_mut() {
            Some(PenCommand::Dots(dots)) => dots.push(dot),
            _ => self.pending_pen_commands.push(PenCommand::Dots(vec![dot])),
        }
    }

    #[inline]
    fn push_pen_stroke(&mut self, stroke: PenStroke) {
        // retain marks in order while grouping adjacent strokes for one renderer upload.
        match self.pending_pen_commands.last_mut() {
            Some(PenCommand::Strokes(strokes)) => strokes.push(stroke),
            _ => self
                .pending_pen_commands
                .push(PenCommand::Strokes(vec![stroke])),
        }
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
        let (changed, stroke) = {
            let instance = &mut self.instances[self.current_instance];
            let old = [instance.x as f32, instance.y as f32];
            let changed = instance.x != x || instance.y != y;
            instance.x = x;
            instance.y = y;
            let new = [x as f32, y as f32];
            let stroke = (changed && instance.pen_down).then_some(PenStroke {
                start: old,
                end: new,
                color: instance.pen_color.rgba,
                diameter: instance.pen_size as f32,
                pixel_offset: 0.0,
            });
            (changed, stroke)
        };
        if let Some(stroke) = stroke {
            self.push_pen_stroke(stroke);
        }
        if changed {
            self.request_redraw();
        }
    }

    fn fenced_position(&mut self, instance_id: usize, x: f64, y: f64) -> (f64, f64) {
        let instance = &self.instances[instance_id];
        let target = instance.target;
        let direction = instance.direction;
        let size = instance.size;
        let rotation_style = instance.rotation_style;
        let instance_x = instance.x;
        let instance_y = instance.y;
        let Some(geometry) = self.fence_geometry.get(target).and_then(Option::as_ref) else {
            return (x, y);
        };
        let geometry_key = [
            geometry.width,
            geometry.height,
            geometry.rotation_center_x,
            geometry.rotation_center_y,
        ];
        let cache = self.instances[instance_id].fence_offsets;
        let offsets = cache
            .filter(|cache| {
                cache.geometry == geometry_key
                    && cache.direction == direction
                    && cache.size == size
                    && cache.rotation_style == rotation_style
            })
            .map(|cache| cache.points)
            .unwrap_or_else(|| {
                // cache the transformed costume corners until its bounds or style changes.
                let (rendered_direction, flip_x) =
                    rotation_style.rendered_direction_and_flip(direction);
                let radians =
                    (90.0 - rendered_direction) * std::f64::consts::PI / 180.0;
                let cos = radians.cos();
                let sin = radians.sin();
                let scale = size / 100.0;
                let points = costume_corners(geometry).map(|(local_x, local_y)| {
                    transform_costume_point(
                        local_x,
                        local_y,
                        geometry.rotation_center_x,
                        geometry.rotation_center_y,
                        cos,
                        sin,
                        flip_x,
                        scale,
                    )
                });
                self.instances[instance_id].fence_offsets = Some(CachedFenceOffsets {
                    geometry: geometry_key,
                    direction,
                    size,
                    rotation_style,
                    points,
                });
                points
            });
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for (offset_x, offset_y) in offsets {
            let tx = offset_x + instance_x;
            let ty = offset_y + instance_y;
            min_x = min_x.min(tx);
            max_x = max_x.max(tx);
            min_y = min_y.min(ty);
            max_y = max_y.max(ty);
        }
        let inset = (((max_x - min_x).min(max_y - min_y) / 2.0).floor()).max(0.0);
        let margin = 15.0_f64.min(inset);
        let fence_x = 240.0 - margin;
        let fence_y = 180.0 - margin;
        let dx = x - instance_x;
        let dy = y - instance_y;
        let mut fenced_x = x;
        let mut fenced_y = y;
        if max_x + dx < -fence_x {
            fenced_x = (instance_x - (fence_x + max_x)).ceil();
        } else if min_x + dx > fence_x {
            fenced_x = (instance_x + (fence_x - min_x)).floor();
        }
        if max_y + dy < -fence_y {
            fenced_y = (instance_y - (fence_y + max_y)).ceil();
        } else if min_y + dy > fence_y {
            fenced_y = (instance_y + (fence_y - min_y)).floor();
        }
        (fenced_x, fenced_y)
    }

    fn transformed_bounds(&self, instance_id: usize, use_silhouette: bool) -> Option<[f64; 4]> {
        let instance = self.instances.get(instance_id)?;
        let geometry = self.fence_geometry.get(instance.target)?.as_ref()?;
        let (rendered_direction, flip_x) = instance
            .rotation_style
            .rendered_direction_and_flip(instance.direction);
        let radians = (90.0 - rendered_direction) * std::f64::consts::PI / 180.0;
        let cos = radians.cos();
        let sin = radians.sin();
        let scale = instance.size / 100.0;
        let mut bounds = [
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        let mut include = |x: f64, y: f64| {
            let (offset_x, offset_y) = transform_costume_point(
                x,
                y,
                geometry.rotation_center_x,
                geometry.rotation_center_y,
                cos,
                sin,
                flip_x,
                scale,
            );
            // scratch's renderer stores transformed hull points and drawable positions as float32.
            let (x, y) = if use_silhouette {
                (
                    (instance.x.round() + offset_x) as f32 as f64,
                    (instance.y.round() + offset_y) as f32 as f64,
                )
            } else {
                (offset_x + instance.x, offset_y + instance.y)
            };
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].max(x);
            bounds[2] = bounds[2].min(y);
            bounds[3] = bounds[3].max(y);
        };
        let outline = if use_silhouette {
            geometry.bounds_hull.as_deref()
        } else {
            None
        };
        if let Some(outline) = outline {
            if outline.is_empty() {
                return None;
            }
            for point in outline {
                // scratch's drawable transform places hull samples at texel centers.
                include(point[0] + 0.5, point[1] + 0.5);
            }
        } else {
            for (x, y) in costume_corners(geometry) {
                include(x, y);
            }
        }
        (bounds[0].is_finite() && bounds[1].is_finite()).then_some(bounds)
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
            rotation_style: ScratchRotationStyle::AllAround,
            size: 100.0,
            visible: true,
            values: vec![None; self.values.len()],
            lists: vec![None; self.lists.len()],
            bubble: None,
            pen_color: PenColorState::default(),
            pen_down: false,
            pen_size: 1.0,
            costume_index: 0,
            fence_offsets: None,
            sound_pitch: 0.0,
            sound_pan: 0.0,
            volume: 100.0,
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
            rotation_style: self.instances[source_instance].rotation_style,
            size: self.instances[source_instance].size,
            visible: self.instances[source_instance].visible,
            values: vec![None; self.values.len()],
            lists: vec![None; self.lists.len()],
            bubble: None,
            pen_color: self.instances[source_instance].pen_color,
            pen_down: self.instances[source_instance].pen_down,
            pen_size: self.instances[source_instance].pen_size,
            costume_index: self.instances[source_instance].costume_index,
            fence_offsets: None,
            sound_pitch: self.instances[source_instance].sound_pitch,
            sound_pan: self.instances[source_instance].sound_pan,
            volume: self.instances[source_instance].volume,
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
        let instance_id = self.current_instance;
        let instance = &self.instances[instance_id];
        if instance.original {
            return false;
        }
        self.sound_runtime.stop_target(instance.target, instance_id);
        let instance = &mut self.instances[instance_id];
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

    // keep the old snapshot API usable by generated modules until they are regenerated.
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

    // support older generated modules while routing new code through the cheap change bit.
    pub fn list_changed(&mut self, slot: usize, before: Option<ScratchList>) {
        let changed = before.is_some_and(|before| !before.shares_storage_with(self.list(slot)));
        self.list_changed_result(slot, changed);
    }

    // publish only successful visible mutations, without cloning the list around each write.
    pub fn list_changed_result(&mut self, slot: usize, changed: bool) {
        if !changed || !self.list_visibility.get(slot).copied().unwrap_or(false) {
            return;
        }
        if let Some(metadata) = self.list_metadata.get(slot)
            && !metadata.is_stage
            && (!self.instances[self.current_instance].original
                || self.instances[self.current_instance].target != metadata.target_index)
        {
            return;
        }
        self.request_presentation();
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
        BubbleKind, ObservableEvent, PenDot, PenStroke, RuntimeInputEvent, ScratchListMetadata,
        ScratchRotationStyle, ScratchSoundEffect, ScratchState, ScratchValue,
        ScratchVariableMetadata, SpriteFenceGeometry, pointer_click_activates,
    };
    use crate::runtime::PenCommand;
    use crate::runtime::{ScratchList, list_snapshot_key, scratch_to_number};
    use std::sync::Arc;

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
        assert!(!(state.loudness() > -10.0));
        state.set_loudness(f64::NAN);
        assert_eq!(state.loudness(), -1.0);
    }

    #[test]
    fn sound_effects_clamp_copy_to_clones_and_clear_without_resetting_volume() {
        let mut state = ScratchState::new(Vec::new());
        state.register_target(0, "Stage", true);
        state.register_target(1, "Sprite1", false);
        let original = state.original_instance(1).unwrap();
        state.set_current_instance(original);
        state.set_sound_effect(ScratchSoundEffect::Pitch, 500.0, true, 0);
        state.set_sound_effect(ScratchSoundEffect::Pitch, 100.0, true, 0);
        state.set_sound_effect(ScratchSoundEffect::Pitch, -500.0, true, 0);
        state.set_sound_effect(ScratchSoundEffect::Pan, 150.0, true, 0);
        state.set_sound_effect(ScratchSoundEffect::Pan, 150.0, true, 0);
        state.set_sound_effect(ScratchSoundEffect::Pan, -150.0, true, 0);
        state.set_sound_volume(90.0, false, 0);
        state.set_sound_volume(100.0, true, 0);

        let clone = state.create_clone(original, 1, 300).unwrap();
        assert_eq!(state.instances[clone].sound_pitch, -140.0);
        assert_eq!(state.instances[clone].sound_pan, -50.0);
        assert_eq!(state.instances[clone].volume, 100.0);

        state.set_current_instance(clone);
        state.set_sound_effect(ScratchSoundEffect::Pitch, -500.0, false, 1);
        state.set_sound_effect(ScratchSoundEffect::Pitch, -100.0, true, 1);
        state.set_sound_effect(ScratchSoundEffect::Pan, 150.0, false, 1);
        state.set_sound_effect(ScratchSoundEffect::Pan, 10.0, true, 1);
        state.set_sound_effect(ScratchSoundEffect::Pan, -500.0, true, 1);
        state.set_sound_volume(15.0, false, 1);
        state.clear_sound_effects(1);
        assert_eq!(state.instances[clone].sound_pitch, 0.0);
        assert_eq!(state.instances[clone].sound_pan, 0.0);
        assert_eq!(state.instances[clone].volume, 15.0);

        state.set_current_instance(original);
        assert_eq!(state.instances[original].sound_pitch, -140.0);
        assert_eq!(state.instances[original].sound_pan, -50.0);
        assert_eq!(state.instances[original].volume, 100.0);

        state.restart_sounds(2);
        assert_eq!(state.instances[original].sound_pitch, 0.0);
        assert_eq!(state.instances[original].sound_pan, 0.0);
        assert_eq!(state.instances[original].volume, 100.0);
        assert_eq!(state.instances[clone].sound_pitch, 0.0);
        assert_eq!(state.instances[clone].sound_pan, 0.0);
        assert_eq!(state.instances[clone].volume, 15.0);
    }

    #[test]
    fn project_volume_is_loaded_for_the_original_and_inherited_by_clones() {
        let mut state = ScratchState::new(Vec::new());
        state.register_target(0, "Stage", true);
        state.register_target(1, "Sprite1", false);
        state.set_original_volume(1, 42.0);
        let original = state.original_instance(1).unwrap();
        state.set_current_instance(original);
        assert_eq!(state.sound_volume(), 42.0);
        assert_eq!(state.sound_volume_for_target(1), 42.0);
        assert_eq!(state.sound_volume_for_target(0), 100.0);

        let clone = state.create_clone(original, 1, 300).unwrap();
        state.set_current_instance(clone);
        assert_eq!(state.sound_volume(), 42.0);
        state.set_sound_volume(20.0, true, 0);
        assert_eq!(state.sound_volume(), 62.0);
        assert_eq!(state.sound_volume_for_target(1), 42.0);
        state.set_current_instance(original);
        assert_eq!(state.sound_volume(), 42.0);
    }

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
                bounds_hull: None,
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

    // releasing hidden snapshots lets clear and refill reuse all 172,800 slots.
    #[test]
    fn graphical_hidden_list_snapshots_release_storage_for_capacity_reuse() {
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
        let mut snapshot = state.observable_snapshot(1);
        assert_eq!(Arc::strong_count(&state.list(0).shared_items()), 3);

        snapshot.release_hidden_list_items();
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
        assert_eq!(visible_snapshot.lists[0].items[0], ScratchValue::Number(0.0));
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
        assert!(
            matches!(state.get_shared(4), ScratchValue::Number(value) if value.is_sign_negative())
        );
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
