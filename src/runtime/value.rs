#[derive(Debug, Clone, PartialEq)]
pub enum ScratchValue {
    Number(f64),
    Text(String),
    Boolean(bool),
}

use super::{ScratchList, scratch_number_operand, scratch_to_number};
use std::collections::HashSet;
use std::fmt::Debug;
use std::sync::mpsc::Sender;

#[inline]
fn add_to_scratch_value(value: &mut ScratchValue, change: f64) {
    match value {
        ScratchValue::Number(number) => *number = scratch_number_operand(*number) + change,
        ScratchValue::Text(_) | ScratchValue::Boolean(_) => {
            *value = ScratchValue::Number(scratch_to_number(&*value) + change);
        }
    }
}

/// input collected by the presentation thread and consumed by generated code.
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeInputEvent {
    Key { name: String, pressed: bool },
    MousePosition { x: f64, y: f64 },
    MouseButton { pressed: bool },
}

#[derive(Debug, Default)]
struct RuntimeInputState {
    pressed_keys: HashSet<String>,
    mouse_x: f64,
    mouse_y: f64,
    mouse_down: bool,
}

impl RuntimeInputState {
    fn apply(&mut self, event: RuntimeInputEvent) -> Option<String> {
        match event {
            RuntimeInputEvent::Key { name, pressed } => {
                let name = name.to_ascii_lowercase();
                if pressed {
                    self.pressed_keys.insert(name.clone()).then_some(name)
                } else {
                    self.pressed_keys.remove(&name);
                    None
                }
            }
            RuntimeInputEvent::MousePosition { x, y } => {
                self.mouse_x = x;
                self.mouse_y = y;
                None
            }
            RuntimeInputEvent::MouseButton { pressed } => {
                self.mouse_down = pressed;
                None
            }
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
    event_sink: Option<Sender<ObservableEvent>>,
    input: RuntimeInputState,
    redraw_requested: bool,
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListSnapshot {
    pub key: String,
    pub id: String,
    pub name: String,
    pub target: String,
    pub instance_id: Option<usize>,
    pub items: Vec<ScratchValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayerSnapshot {
    pub target_index: usize,
    pub target: String,
    pub instance_id: usize,
    pub original: bool,
    pub x: f64,
    pub y: f64,
}

/// immutable copies for presentation and observation, never script execution state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuntimeSnapshot {
    pub tick: usize,
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
    values: Vec<Option<ScratchValue>>,
    lists: Vec<Option<ScratchList>>,
}

/// names and scope information for one shared-state slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScratchVariableMetadata {
    pub id: String,
    pub name: String,
    pub target: String,
    pub is_stage: bool,
    pub target_index: usize,
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

impl ScratchState {
    /// clear the previous frame's request before stepping generated scripts.
    pub fn begin_tick(&mut self) {
        self.redraw_requested = false;
    }

    pub fn request_redraw(&mut self) {
        self.redraw_requested = true;
    }

    pub fn redraw_requested(&self) -> bool {
        self.redraw_requested
    }

    pub fn presentation_consumed(&mut self) {
        self.redraw_requested = false;
    }

    /// update canonical input state from the renderer's queued events.
    pub fn apply_input(&mut self, event: RuntimeInputEvent) -> Option<String> {
        self.input.apply(event)
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
                values: Vec::new(),
                lists: Vec::new(),
            }],
            current_instance: 0,
            pending_clones: Vec::new(),
            target_names: Vec::new(),
            event_sink: None,
            input: RuntimeInputState::default(),
            redraw_requested: false,
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
                values: Vec::new(),
                lists: Vec::new(),
            }],
            current_instance: 0,
            pending_clones: Vec::new(),
            target_names: Vec::new(),
            event_sink: None,
            input: RuntimeInputState::default(),
            redraw_requested: false,
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
                values: Vec::new(),
                lists: Vec::new(),
            }],
            current_instance: 0,
            pending_clones: Vec::new(),
            target_names: Vec::new(),
            event_sink: None,
            input: RuntimeInputState::default(),
            redraw_requested: false,
        }
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
        if let Some(metadata) = self.variables.get(slot) {
            if !metadata.is_stage {
                self.ensure_instance_slot(slot);
                self.instances[self.current_instance].values[slot] = Some(value);
                self.publish_variable_change(change, slot);
                return;
            }
        }
        self.values[slot] = value;
        self.publish_variable_change(change, slot);
    }

    /// write a stage variable without checking variable scope metadata.
    #[inline]
    pub fn set_shared(&mut self, slot: usize, value: ScratchValue) {
        let change = self.capture_variable_change(slot);
        self.values[slot] = value;
        self.publish_variable_change(change, slot);
    }

    /// apply a scratch-converted numeric change directly to a stage variable.
    #[inline]
    pub fn change_shared(&mut self, slot: usize, change: f64) {
        let old_value = self.capture_variable_change(slot);
        add_to_scratch_value(&mut self.values[slot], change);
        self.publish_variable_change(old_value, slot);
    }

    /// change a sprite-local variable for the current instance.
    #[inline]
    pub fn change_local_by(&mut self, slot: usize, change: f64) {
        if self.event_sink.is_none() {
            let current_instance = self.current_instance;
            if let Some(value) = self.instances[current_instance]
                .values
                .get_mut(slot)
                .and_then(Option::as_mut)
            {
                add_to_scratch_value(value, change);
                return;
            }
            let initial = self.values[slot].clone();
            self.ensure_instance_slot(slot);
            let value = self.instances[current_instance].values[slot].get_or_insert(initial);
            add_to_scratch_value(value, change);
            return;
        }

        let old_value = self.capture_variable_change(slot);
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
        self.publish_variable_change(old_value, slot);
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
                    items: self.lists[slot].items().to_vec(),
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
                    items: list.items().to_vec(),
                });
            }
        }
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

    pub fn set_xy(&mut self, x: f64, y: f64) {
        if !self.instances[self.current_instance].is_stage {
            let instance = &mut self.instances[self.current_instance];
            instance.x = x;
            instance.y = y;
        }
    }

    pub fn change_x_by(&mut self, dx: f64) {
        let instance = &mut self.instances[self.current_instance];
        if !instance.is_stage {
            instance.x += dx;
        }
    }

    pub fn change_y_by(&mut self, dy: f64) {
        let instance = &mut self.instances[self.current_instance];
        if !instance.is_stage {
            instance.y += dy;
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
            values: vec![None; self.values.len()],
            lists: vec![None; self.lists.len()],
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
            values: vec![None; self.values.len()],
            lists: vec![None; self.lists.len()],
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
        true
    }

    pub fn remove_clones(&mut self) {
        for instance in &mut self.instances {
            if !instance.original {
                instance.alive = false;
                instance.values.clear();
                instance.lists.clear();
            }
        }
        self.pending_clones.clear();
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
        self.list_visibility[slot] = visible;
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

#[cfg(test)]
mod tests {
    use super::{
        ObservableEvent, RuntimeInputEvent, ScratchListMetadata, ScratchState, ScratchValue,
        ScratchVariableMetadata,
    };
    use crate::runtime::{ScratchList, scratch_to_number};

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
                "local",
                "score",
                "Sprite",
                false,
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
}
