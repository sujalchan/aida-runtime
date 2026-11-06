//! Snapshots operations for Scratch project state.

use super::*;

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
pub(super) struct SpeechBubble {
    pub(super) kind: BubbleKind,
    pub(super) text: String,
    pub(super) expires_at_millis: Option<u64>,
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

impl ScratchState {
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

    pub fn observable_snapshot(&self, tick: usize) -> RuntimeSnapshot {
        self.observable_snapshot_filtered(tick, true)
    }

    // Renderer snapshots retain list identity and visibility, but hidden list
    // payloads must not materialize compact lists or pin their backing storage.
    pub fn observable_snapshot_filtered(
        &self,
        tick: usize,
        include_hidden_items: bool,
    ) -> RuntimeSnapshot {
        static EMPTY_ITEMS: std::sync::LazyLock<Arc<Vec<ScratchValue>>> =
            std::sync::LazyLock::new(|| Arc::new(Vec::new()));
        let list_items = |list: &ScratchList, visible: bool| {
            if include_hidden_items || visible {
                list.shared_items()
            } else {
                Arc::clone(&EMPTY_ITEMS)
            }
        };
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
                    items: list_items(&self.lists[slot], self.list_visibility[slot]),
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
                    items: list_items(list, self.list_visibility[slot]),
                    visible: self.list_visibility[slot],
                });
            }
        }
        snapshot
    }

    pub fn observable_snapshot_with_timer(&self, tick: usize, now_millis: u64) -> RuntimeSnapshot {
        self.observable_snapshot_filtered_with_timer(tick, now_millis, true)
    }

    pub fn observable_snapshot_filtered_with_timer(
        &self,
        tick: usize,
        now_millis: u64,
        include_hidden_items: bool,
    ) -> RuntimeSnapshot {
        let mut snapshot = self.observable_snapshot_filtered(tick, include_hidden_items);
        snapshot.project_timer_millis = self.project_timer_millis(now_millis);
        snapshot.project_timer_epoch = self.project_timer_epoch;
        snapshot
    }
}
