#[derive(Debug, Clone, PartialEq)]
pub enum ScratchValue {
    Number(f64),
    Text(String),
    Boolean(bool),
}

use super::ScratchList;

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

/// shared project variables addressed by compiler-assigned slots.
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
}

#[derive(Debug, Clone)]
struct ScratchTargetInstance {
    target: usize,
    original: bool,
    alive: bool,
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
                values: Vec::new(),
                lists: Vec::new(),
            }],
            current_instance: 0,
            pending_clones: Vec::new(),
            target_names: Vec::new(),
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
                values: Vec::new(),
                lists: Vec::new(),
            }],
            current_instance: 0,
            pending_clones: Vec::new(),
            target_names: Vec::new(),
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
                values: Vec::new(),
                lists: Vec::new(),
            }],
            current_instance: 0,
            pending_clones: Vec::new(),
            target_names: Vec::new(),
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
        if let Some(metadata) = self.variables.get(slot) {
            if !metadata.is_stage {
                self.ensure_instance_slot(slot);
                self.instances[self.current_instance].values[slot] = Some(value);
                return;
            }
        }
        self.values[slot] = value;
    }

    /// write a stage variable without checking variable scope metadata.
    #[inline]
    pub fn set_shared(&mut self, slot: usize, value: ScratchValue) {
        self.values[slot] = value;
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

    pub fn add_original_target(&mut self, target: usize) -> usize {
        if let Some(id) = self.original_instance(target) {
            return id;
        }
        let id = self.instances.len();
        self.instances.push(ScratchTargetInstance {
            target,
            original: true,
            alive: true,
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
        self.add_original_target(target);
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
    use super::{ScratchListMetadata, ScratchState, ScratchValue, ScratchVariableMetadata};
    use crate::runtime::ScratchList;

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
    fn shared_variable_fast_access_ignores_clone_local_storage() {
        let mut state = clone_test_state();
        let clone = state.create_clone(1, 1, 300).unwrap();
        state.set_current_instance(clone);

        state.set_shared(1, 17.0_f64.into());

        assert_eq!(state.get_shared(1), &ScratchValue::Number(17.0));
        assert_eq!(state.get(1), &ScratchValue::Number(17.0));
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
}
