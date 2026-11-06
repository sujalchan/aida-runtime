//! Variables operations for Scratch project state.

use super::*;

#[inline]
fn add_to_scratch_value(value: &mut ScratchValue, change: f64) {
    match value {
        ScratchValue::Number(number) => *number = scratch_number_operand(*number) + change,
        ScratchValue::Text(_) | ScratchValue::Boolean(_) | ScratchValue::Undefined => {
            *value = ScratchValue::Number(scratch_to_number(&*value) + change);
        }
    }
}

impl ScratchState {
    /// Allow generated numeric locals to defer writes only when no renderer,
    /// visible monitor, or event sink can observe the values before the
    /// generated region commits them. Generated code commits before every
    /// scheduler yield, where other scripts can observe shared state.
    pub fn can_defer_variable_writes(&self, slots: &[usize]) -> bool {
        self.event_sink.is_none()
            && slots.iter().all(|slot| {
                self.variables
                    .get(*slot)
                    .is_some_and(|metadata| !metadata.monitor_visible)
            })
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
        if self
            .variables
            .get(slot)
            .is_some_and(|metadata| !metadata.is_stage)
        {
            self.ensure_instance_slot(slot);
            self.instances[self.current_instance].values[slot] = Some(value);
            self.finish_monitored_variable_write(slot, old_value);
            self.publish_variable_change(change, slot);
            return;
        }
        self.values[slot] = value;
        self.finish_monitored_variable_write(slot, old_value);
        self.publish_variable_change(change, slot);
    }

    /// Write a generator-resolved sprite-local slot without changing its type.
    /// Logged or visible writes and uninitialized instances retain the general path.
    #[inline]
    pub fn set_local_value(&mut self, slot: usize, value: ScratchValue) {
        if self.event_sink.is_none()
            && slot >= self.list_metadata.len()
            && self.instances[self.current_instance].lists.len() >= self.lists.len()
            && self
                .variables
                .get(slot)
                .is_some_and(|metadata| !metadata.is_stage && !metadata.monitor_visible)
        {
            let instance = &mut self.instances[self.current_instance];
            if let Some(Some(previous)) = instance.values.get_mut(slot) {
                *previous = value;
                return;
            }
        }
        self.set(slot, value);
    }

    /// Copy a shared-list item into a sprite-local variable, reusing an
    /// existing text allocation when the write has no observers.
    pub fn set_local_from_shared_list<const CACHED: bool>(
        &mut self,
        slot: usize,
        list_slot: usize,
        index: impl ScratchInput,
    ) -> CachedListIndex {
        if self.event_sink.is_none()
            && slot >= self.list_metadata.len()
            && self
                .variables
                .get(slot)
                .is_some_and(|metadata| !metadata.is_stage && !metadata.monitor_visible)
            && self.instances[self.current_instance].lists.len() >= self.lists.len()
            && self.instances[self.current_instance]
                .values
                .get(slot)
                .is_some_and(Option::is_some)
        {
            let (source, cached_index) = if CACHED {
                self.lists[list_slot].get_with_cached_selector(&index)
            } else {
                let source = self.lists[list_slot].get(&index);
                (source, CachedListIndex::new(source))
            };
            if let Some(Some(destination)) =
                self.instances[self.current_instance].values.get_mut(slot)
            {
                match (destination, source) {
                    (ScratchValue::Text(destination), ScratchValue::Text(source)) => {
                        destination.clone_from(source)
                    }
                    (destination, source) => *destination = source.clone(),
                }
                return cached_index;
            }
        }
        let (value, cached_index) = if CACHED {
            self.list(list_slot).get_with_cached_selector(index)
        } else {
            let value = self.list(list_slot).get(index);
            (value, CachedListIndex::new(value))
        };
        let value = value.clone();
        self.set(slot, value);
        cached_index
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
