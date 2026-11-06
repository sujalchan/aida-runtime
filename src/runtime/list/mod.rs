use super::{
    CachedListIndex, ScratchInput, ScratchValue, ScratchValueView, operators::to_text,
    scratch_equals, scratch_random, scratch_to_number,
};
use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
use std::{
    fmt,
    sync::{Arc, OnceLock},
};

pub const SCRATCH_LIST_LIMIT: usize = 200_000;

static EMPTY_ITEM: ScratchValue = ScratchValue::Text(String::new());

#[derive(Debug, Clone)]
pub struct ScratchList {
    // `items` is authoritative unless the optional compact numeric form is active.
    items: Arc<Vec<ScratchValue>>,
    numeric_items: Option<Arc<Vec<f64>>>,
    // Shared snapshots and generic borrowed reads materialize this cache lazily.
    materialized_items: OnceLock<Arc<Vec<ScratchValue>>>,
    cached_selectors: OnceLock<Arc<Vec<CachedListIndex>>>,
    // true only when constructors or generated operations establish numeric-only contents.
    all_numbers: bool,
}

impl ScratchList {
    pub fn new(items: Vec<ScratchValue>) -> Self {
        let all_numbers = items
            .iter()
            .all(|item| matches!(item, ScratchValue::Number(_)));
        Self {
            items: Arc::new(items),
            numeric_items: None,
            materialized_items: OnceLock::new(),
            cached_selectors: OnceLock::new(),
            all_numbers,
        }
    }

    /// Build a list from the compact JSON initializer emitted by aida-ast.
    pub fn from_json(json: &str) -> Self {
        Self::from_json_bytes(json.as_bytes())
    }

    /// Build a list from a compact JSON byte slice stored beside generated code.
    pub fn from_json_bytes(json: &[u8]) -> Self {
        let items: Vec<ScratchValue> = serde_json::from_slice(json)
            .expect("generated Scratch list initializer must contain a JSON array");
        Self::new(items)
    }

    fn current_items(&self) -> &Arc<Vec<ScratchValue>> {
        match &self.numeric_items {
            Some(numbers) => self.materialized_items.get_or_init(|| {
                Arc::new(numbers.iter().copied().map(ScratchValue::Number).collect())
            }),
            None => &self.items,
        }
    }

    // General mutations switch back to the dynamic representation. Cached
    // snapshots remain independent, and read-only access can materialize lazily.
    fn materialize_for_mutation(&mut self) {
        self.cached_selectors.take();
        if let Some(numbers) = self.numeric_items.take() {
            self.items = self.materialized_items.take().unwrap_or_else(|| {
                Arc::new(numbers.iter().copied().map(ScratchValue::Number).collect())
            });
        }
    }

    pub fn items(&self) -> &[ScratchValue] {
        self.current_items()
    }

    pub fn shared_items(&self) -> Arc<Vec<ScratchValue>> {
        Arc::clone(self.current_items())
    }

    #[cfg(test)]
    pub(crate) fn has_materialized_numeric_items(&self) -> bool {
        self.numeric_items.is_some() && self.materialized_items.get().is_some()
    }

    pub(crate) fn shares_storage_with(&self, other: &Self) -> bool {
        match (&self.numeric_items, &other.numeric_items) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            (None, None) => Arc::ptr_eq(&self.items, &other.items),
            _ => false,
        }
    }

    pub fn add(&mut self, item: ScratchValue) -> bool {
        self.materialize_for_mutation();
        if self.items.len() >= SCRATCH_LIST_LIMIT {
            return false;
        }
        self.all_numbers &= matches!(&item, ScratchValue::Number(_));
        Arc::make_mut(&mut self.items).push(item);
        true
    }

    // reserve one shared vector mutation for batches emitted from safe warp loops.
    pub fn add_generated(&mut self, count: usize, mut item: impl FnMut() -> ScratchValue) -> bool {
        self.materialize_for_mutation();
        let count = count.min(SCRATCH_LIST_LIMIT.saturating_sub(self.items.len()));
        if count == 0 {
            return false;
        }
        let mut all_numbers = self.all_numbers;
        let items = Arc::make_mut(&mut self.items);
        items.reserve(count);
        for _ in 0..count {
            let value = item();
            all_numbers &= matches!(value, ScratchValue::Number(_));
            items.push(value);
        }
        self.all_numbers = all_numbers;
        true
    }

    // append statically numeric reporters without per-item dynamic type checks.
    pub fn add_generated_numbers(&mut self, count: usize, mut item: impl FnMut() -> f64) -> bool {
        self.materialize_for_mutation();
        let count = count.min(SCRATCH_LIST_LIMIT.saturating_sub(self.items.len()));
        if count == 0 {
            return false;
        }
        let all_numbers = self.all_numbers;
        let items = Arc::make_mut(&mut self.items);
        items.reserve(count);
        items.extend((0..count).map(|_| ScratchValue::Number(item())));
        self.all_numbers = all_numbers;
        true
    }

    pub fn replace_all(&mut self, items: Vec<ScratchValue>) {
        self.materialize_for_mutation();
        self.all_numbers = items
            .iter()
            .all(|item| matches!(item, ScratchValue::Number(_)));
        self.items = Arc::new(items);
    }

    pub fn delete(&mut self, index: impl ScratchInput) -> bool {
        self.materialize_for_mutation();
        match resolve_index(index, self.items.len(), true) {
            ListIndex::All => self.delete_all(),
            ListIndex::Item(index) => {
                Arc::make_mut(&mut self.items).remove(index);
                if self.items.is_empty() {
                    self.all_numbers = true;
                }
                true
            }
            ListIndex::Invalid => false,
        }
    }

    // clear shared lists without cloning saved items, and keep the allocation for a refill.
    pub fn delete_all(&mut self) -> bool {
        self.materialize_for_mutation();
        if self.items.is_empty() {
            self.all_numbers = true;
            return false;
        }
        if let Some(items) = Arc::get_mut(&mut self.items) {
            if self.all_numbers {
                // numeric values own no resources, so their drop glue can be skipped safely.
                unsafe { items.set_len(0) };
            } else {
                items.clear();
            }
        } else {
            let capacity = self.items.capacity().min(SCRATCH_LIST_LIMIT);
            self.items = Arc::new(Vec::with_capacity(capacity));
        }
        self.all_numbers = true;
        true
    }

    pub fn insert(&mut self, item: ScratchValue, index: impl ScratchInput) -> bool {
        self.materialize_for_mutation();
        let index = match resolve_index(index, self.items.len().saturating_add(1), false) {
            ListIndex::Item(index) => index,
            ListIndex::All | ListIndex::Invalid => return false,
        };
        if index >= SCRATCH_LIST_LIMIT {
            return false;
        }
        self.all_numbers &= matches!(&item, ScratchValue::Number(_));
        let items = Arc::make_mut(&mut self.items);
        items.insert(index, item);
        if items.len() > SCRATCH_LIST_LIMIT {
            items.pop();
        }
        true
    }

    pub fn replace(&mut self, index: impl ScratchInput, item: ScratchValue) -> bool {
        self.materialize_for_mutation();
        let ListIndex::Item(index) = resolve_index(index, self.items.len(), false) else {
            return false;
        };
        if self.items[index] == item {
            return false;
        }
        self.all_numbers &= matches!(&item, ScratchValue::Number(_));
        Arc::make_mut(&mut self.items)[index] = item;
        true
    }

    /// replace a numeric one-based item without building dynamic Scratch values.
    #[inline]
    pub fn replace_number(&mut self, index: f64, item: f64) -> bool {
        self.materialize_for_mutation();
        let Some(index) = resolve_numeric_index(index, self.items.len()) else {
            return false;
        };
        if matches!(&self.items[index], ScratchValue::Number(current) if *current == item) {
            return false;
        }
        Arc::make_mut(&mut self.items)[index] = ScratchValue::Number(item);
        true
    }

    /// replace a numeric slot and compare the value read back using one resolved index.
    #[inline]
    pub fn replace_number_and_equals(
        &mut self,
        index: f64,
        item: f64,
        expected: f64,
    ) -> (bool, bool) {
        self.materialize_for_mutation();
        let Some(index) = resolve_numeric_index(index, self.items.len()) else {
            return (false, false);
        };
        let changed =
            !matches!(&self.items[index], ScratchValue::Number(current) if *current == item);
        if changed {
            Arc::make_mut(&mut self.items)[index] = ScratchValue::Number(item);
        }
        let matches = match &self.items[index] {
            ScratchValue::Number(actual) => scratch_equals(*actual, expected),
            _ => unreachable!("numeric replacement stores a number before reading it back"),
        };
        (changed, matches)
    }

    pub(crate) fn batch_placeholder() -> Self {
        static EMPTY: OnceLock<Arc<Vec<ScratchValue>>> = OnceLock::new();
        Self {
            items: Arc::clone(EMPTY.get_or_init(|| Arc::new(Vec::new()))),
            numeric_items: None,
            materialized_items: OnceLock::new(),
            cached_selectors: OnceLock::new(),
            all_numbers: true,
        }
    }

    /// Borrow the authoritative items once for a synchronous generated batch.
    pub fn with_numeric_replacements<R>(
        &mut self,
        run: impl FnOnce(&mut NumericListReplacements<'_>) -> R,
    ) -> R {
        self.cached_selectors.take();
        if self.all_numbers {
            if self.numeric_items.is_none() {
                let numbers = self
                    .items
                    .iter()
                    .map(|item| match item {
                        ScratchValue::Number(number) => *number,
                        _ => unreachable!("numeric list invariant"),
                    })
                    .collect();
                self.numeric_items = Some(Arc::new(numbers));
            }
            self.materialized_items.take();
            let items = Arc::make_mut(self.numeric_items.as_mut().unwrap());
            run(&mut NumericListReplacements::Numbers(items))
        } else {
            self.materialize_for_mutation();
            let items = Arc::make_mut(&mut self.items);
            run(&mut NumericListReplacements::Values(items))
        }
    }

    /// compare a numeric list item while keeping the general Scratch path available.
    #[inline]
    pub fn equals_number(&self, index: f64, value: f64) -> Option<bool> {
        if !self.all_numbers {
            return None;
        }
        let index = resolve_numeric_index(index, self.length())?;
        match &self.current_items()[index] {
            ScratchValue::Number(item) => Some(
                item.partial_cmp(&value)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .is_eq(),
            ),
            _ => None,
        }
    }

    #[inline]
    pub fn get(&self, index: impl ScratchInput) -> &ScratchValue {
        if let ScratchValueView::Number(number) = index.scratch_view() {
            return resolve_numeric_index(number, self.length())
                .and_then(|index| self.current_items().get(index))
                .unwrap_or(&EMPTY_ITEM);
        }
        match resolve_index(index, self.length(), false) {
            ListIndex::Item(index) => &self.current_items()[index],
            ListIndex::All | ListIndex::Invalid => &EMPTY_ITEM,
        }
    }

    /// Return the original item and a cached selector derived from it. The
    /// cache is dropped on every mutation and never changes list contents.
    #[inline]
    pub fn get_with_cached_selector(
        &self,
        index: impl ScratchInput,
    ) -> (&ScratchValue, CachedListIndex) {
        let item_index = match resolve_index(index, self.length(), false) {
            ListIndex::Item(index) => index,
            ListIndex::All | ListIndex::Invalid => {
                return (&EMPTY_ITEM, CachedListIndex::new(&EMPTY_ITEM));
            }
        };
        let items = self.current_items();
        let selectors = self
            .cached_selectors
            .get_or_init(|| Arc::new(items.iter().map(CachedListIndex::new).collect()));
        (&items[item_index], selectors[item_index])
    }

    #[inline]
    pub fn get_cached_number(&self, index: impl ScratchInput) -> f64 {
        let (_, selector) = self.get_with_cached_selector(index);
        scratch_to_number(selector)
    }

    pub fn length(&self) -> usize {
        self.numeric_items
            .as_ref()
            .map_or_else(|| self.items.len(), |items| items.len())
    }

    pub fn contains(&self, item: impl ScratchInput) -> bool {
        self.current_items()
            .iter()
            .any(|value| scratch_equals(value, &item))
    }

    pub fn item_number(&self, item: impl ScratchInput) -> usize {
        // compare numeric entries directly while preserving scratch fallback for mixed lists.
        let query = item.scratch_view();
        let index = if let ScratchValueView::Number(search) = query {
            self.current_items().iter().position(|value| match value {
                ScratchValue::Number(candidate) if !search.is_nan() && !candidate.is_nan() => {
                    *candidate == search
                }
                _ => scratch_equals(value, &item),
            })
        } else {
            self.current_items()
                .iter()
                .position(|value| scratch_equals(value, &item))
        };
        index.map_or(0, |index| index + 1)
    }

    pub fn contents(&self) -> String {
        let single_letter_items = self.current_items().iter().all(|item| match item {
            ScratchValue::Text(text) => text.encode_utf16().count() == 1,
            ScratchValue::Number(_) | ScratchValue::Boolean(_) | ScratchValue::Undefined => false,
        });
        let separator = if single_letter_items { "" } else { " " };
        let mut output = String::new();
        for (index, item) in self.current_items().iter().enumerate() {
            if index != 0 {
                output.push_str(separator);
            }
            output.push_str(&to_text(item.scratch_view()));
        }
        output
    }
}

/// A non-suspending numeric replacement batch over the original list storage.
pub enum NumericListReplacements<'a> {
    Values(&'a mut [ScratchValue]),
    Numbers(&'a mut [f64]),
}

impl NumericListReplacements<'_> {
    #[inline]
    pub fn replace_number_and_equals(
        &mut self,
        index: f64,
        item: f64,
        expected: f64,
    ) -> (bool, bool) {
        // Saturating conversion truncates positive values, maps NaN/negative
        // to zero and maps infinity beyond the maximum possible Vec length.
        let Some(index) = (index as usize).checked_sub(1) else {
            return (false, false);
        };
        match self {
            Self::Values(items) => {
                let Some(slot) = items.get_mut(index) else {
                    return (false, false);
                };
                let changed = !matches!(slot, ScratchValue::Number(current) if *current == item);
                if changed {
                    *slot = ScratchValue::Number(item);
                }
                let ScratchValue::Number(actual) = slot else {
                    unreachable!("numeric replacement stores a number");
                };
                (changed, numeric_equals(*actual, expected))
            }
            Self::Numbers(items) => {
                let Some(slot) = items.get_mut(index) else {
                    return (false, false);
                };
                let changed = *slot != item;
                if changed {
                    *slot = item;
                }
                (changed, numeric_equals(*slot, expected))
            }
        }
    }
}

#[inline]
fn numeric_equals(actual: f64, expected: f64) -> bool {
    if !actual.is_nan() && !expected.is_nan() {
        actual == expected
    } else {
        scratch_equals(actual, expected)
    }
}

impl Default for ScratchList {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl PartialEq for ScratchList {
    fn eq(&self, other: &Self) -> bool {
        self.items() == other.items()
    }
}

impl<'de> serde::Deserialize<'de> for ScratchValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ScratchValueVisitor;

        impl<'de> Visitor<'de> for ScratchValueVisitor {
            type Value = ScratchValue;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a Scratch list item")
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
                Ok(ScratchValue::Boolean(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
                Ok(ScratchValue::Number(value as f64))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
                Ok(ScratchValue::Number(value as f64))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
                Ok(ScratchValue::Number(value))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(ScratchValue::Text(value.to_owned()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(ScratchValue::Text(value))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(ScratchValue::Number(0.0))
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                while sequence.next_element::<IgnoredAny>()?.is_some() {}
                Ok(ScratchValue::Number(0.0))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(ScratchValue::Number(0.0))
            }
        }

        deserializer.deserialize_any(ScratchValueVisitor)
    }
}

enum ListIndex {
    Item(usize),
    All,
    Invalid,
}

fn resolve_index(index: impl ScratchInput, length: usize, accept_all: bool) -> ListIndex {
    match index.scratch_view() {
        ScratchValueView::Text("all") if accept_all => ListIndex::All,
        ScratchValueView::Text("last") => length
            .checked_sub(1)
            .map(ListIndex::Item)
            .unwrap_or(ListIndex::Invalid),
        ScratchValueView::Text("random" | "any") if length > 0 => {
            ListIndex::Item(scratch_random(1.0_f64, length as f64) as usize - 1)
        }
        ScratchValueView::Text("all" | "random" | "any") => ListIndex::Invalid,
        value => {
            let number = match value {
                ScratchValueView::Number(number) => number,
                ScratchValueView::Text(text) => scratch_to_number(text),
                ScratchValueView::Boolean(value) => scratch_to_number(value),
                ScratchValueView::Undefined => 0.0,
            };
            if !number.is_finite() {
                return ListIndex::Invalid;
            }
            let index = number.floor();
            if index < 1.0 || index > length as f64 {
                return ListIndex::Invalid;
            }
            ListIndex::Item(index as usize - 1)
        }
    }
}

#[inline]
fn resolve_numeric_index(index: f64, length: usize) -> Option<usize> {
    if !index.is_finite() {
        return None;
    }
    let index = index.floor();
    if index < 1.0 || index > length as f64 {
        return None;
    }
    Some(index as usize - 1)
}

#[cfg(test)]
mod tests;
