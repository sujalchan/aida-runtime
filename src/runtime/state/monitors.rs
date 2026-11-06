//! Monitors operations for Scratch project state.

use super::*;

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
    pub(super) fn monitored_variable_value(&self, slot: usize) -> Option<ScratchValue> {
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

    pub(super) fn finish_monitored_variable_write(
        &mut self,
        slot: usize,
        old_value: Option<ScratchValue>,
    ) {
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

    pub(super) fn capture_variable_change(&self, slot: usize) -> Option<(String, String)> {
        self.event_sink.as_ref()?;
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

    pub(super) fn publish_variable_change(&self, old: Option<(String, String)>, slot: usize) {
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

    pub fn set_project_timer_monitor_visible(&mut self, visible: bool) {
        self.project_timer_monitor_visible = visible;
    }

    pub fn set_variable_visible(&mut self, slot: usize, visible: bool) {
        if let Some(metadata) = self.variables.get_mut(slot)
            && metadata.monitor_visible != visible
        {
            metadata.monitor_visible = visible;
            self.request_presentation();
        }
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
}
