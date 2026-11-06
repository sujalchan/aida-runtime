//! Lists operations for Scratch project state.

use super::*;

pub(super) enum ListMonitorEdit {
    Append,
    SetItem { index: usize, value: String },
    DeleteItem { index: usize },
    Replace(Vec<String>),
}

impl ScratchState {
    pub(super) fn apply_list_monitor_edit(&mut self, key: &str, edit: ListMonitorEdit) -> bool {
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
        run: impl FnOnce(&mut Self, &mut super::super::list::NumericListReplacements<'_>) -> R,
    ) -> R {
        debug_assert!(self.list_metadata[slot].is_stage);
        let mut list = std::mem::replace(&mut self.lists[slot], ScratchList::batch_placeholder());
        let result = list.with_numeric_replacements(|items| run(self, items));
        self.lists[slot] = list;
        result
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
}
