//! Clones operations for Scratch project state.

use super::*;

impl ScratchState {
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
}
