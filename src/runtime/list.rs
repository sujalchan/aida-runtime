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
        if let ListIndex::Item(index) = resolve_index(index, self.items.len(), false) {
            if self.items[index] != item {
                self.all_numbers &= matches!(&item, ScratchValue::Number(_));
                Arc::make_mut(&mut self.items)[index] = item;
                return true;
            }
        }
        false
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
            ScratchValue::Number(actual) => scratch_equals(*actual, &expected),
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
mod tests {
    use super::{SCRATCH_LIST_LIMIT, ScratchList, ScratchValue, scratch_equals};

    #[test]
    fn cached_selector_reads_preserve_values_and_invalidate_on_mutation() {
        let mut list = ScratchList::new(vec!["12".into(), "last".into(), "NaN".into(), 4.0.into()]);
        assert_eq!(list.get_cached_number(1.0), 12.0);
        assert_eq!(list.get_cached_number(2.0), 0.0);
        assert_eq!(list.get_cached_number(3.0), 0.0);
        assert_eq!(
            list.get_with_cached_selector(2.0).0,
            &ScratchValue::Text("last".into())
        );
        let snapshot = list.clone();
        assert!(list.replace(1.0, "35".into()));
        assert_eq!(list.get_cached_number(1.0), 35.0);
        assert_eq!(snapshot.get_cached_number(1.0), 12.0);
        assert!(list.delete(2.0));
        assert_eq!(list.get_cached_number(2.0), 0.0);
        list.replace_all(vec![2.0.into(), 3.0.into()]);
        assert_eq!(list.get_cached_number(1.0), 2.0);
        list.with_numeric_replacements(|items| {
            items.replace_number_and_equals(1.0, 9.0, 9.0);
        });
        assert_eq!(list.get_cached_number(1.0), 9.0);
    }

    #[test]
    fn numeric_replacement_and_equality_keep_scratch_index_and_number_rules() {
        let mut list = ScratchList::new(vec![
            ScratchValue::Number(0.0),
            ScratchValue::Number(0.0),
            ScratchValue::Number(0.0),
        ]);

        assert!(list.replace_number(2.9, 17.0));
        assert_eq!(list.length(), 3);
        assert_eq!(list.get(2.0), &ScratchValue::Number(17.0));
        assert_eq!(list.equals_number(2.0, 17.0), Some(true));
        assert_eq!(list.equals_number(2.0, 18.0), Some(false));
        assert!(!list.replace_number(2.1, 17.0));
        assert!(!list.replace_number(0.9, 12.0));
        assert!(!list.replace_number(f64::INFINITY, 12.0));
        assert_eq!(list.length(), 3);

        let nan_list = ScratchList::new(vec![ScratchValue::Number(f64::NAN)]);
        assert_eq!(nan_list.equals_number(1.0, 42.0), Some(true));

        let mut mixed = ScratchList::new(vec![ScratchValue::Number(1.0), "two".into()]);
        assert_eq!(mixed.equals_number(1.0, 1.0), None);
        assert!(mixed.replace_number(2.0, 3.0));
        assert_eq!(mixed.get(2.0), &ScratchValue::Number(3.0));
    }

    #[test]
    fn numeric_replacement_can_read_back_through_the_same_validated_index() {
        let mut list = ScratchList::new(vec![ScratchValue::Number(0.0), "old".into()]);
        assert_eq!(
            list.replace_number_and_equals(2.0, 17.0, 17.0),
            (true, true)
        );
        assert_eq!(
            list.replace_number_and_equals(2.0, 17.0, 17.0),
            (false, true)
        );
        assert_eq!(
            list.replace_number_and_equals(2.9, 18.0, 18.0),
            (true, true)
        );
        assert_eq!(
            list.replace_number_and_equals(0.0, 17.0, 17.0),
            (false, false)
        );
        assert_eq!(
            list.replace_number_and_equals(-1.0, 19.0, 19.0),
            (false, false)
        );
        assert_eq!(
            list.replace_number_and_equals(3.0, 20.0, 20.0),
            (false, false)
        );
        assert_eq!(
            list.replace_number_and_equals(f64::NAN, 21.0, 21.0),
            (false, false)
        );
        assert_eq!(
            list.replace_number_and_equals(f64::INFINITY, 22.0, 22.0),
            (false, false)
        );
        assert_eq!(list.get(2.0), &ScratchValue::Number(18.0));
        assert_eq!(
            list.replace_number_and_equals(1.0, f64::NAN, f64::NAN),
            (true, true)
        );
    }

    #[test]
    fn numeric_batch_matches_general_replacement_for_mixed_and_invalid_items() {
        let original = ScratchList::new(vec!["text".into(), 2.0.into()]);
        let mut general = original.clone();
        let mut batched = original;
        let cases = [
            (1.9, 7.0, 7.0),
            (2.0, 2.0, 2.0),
            (0.0, 3.0, 3.0),
            (-1.0, 3.0, 3.0),
            (3.0, 3.0, 3.0),
            (f64::NAN, 3.0, 3.0),
            (f64::INFINITY, 3.0, 3.0),
            (1.0, f64::NAN, f64::NAN),
        ];
        let expected: Vec<_> = cases
            .iter()
            .map(|&(index, item, expected)| {
                let changed = general.replace(index, ScratchValue::Number(item));
                (changed, scratch_equals(general.get(index), expected))
            })
            .collect();
        let observed = batched.with_numeric_replacements(|batch| {
            cases
                .iter()
                .map(|&(index, item, expected)| {
                    batch.replace_number_and_equals(index, item, expected)
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(observed, expected);
        assert_eq!(batched.length(), general.length());
        assert!(matches!(batched.get(1.0), ScratchValue::Number(n) if n.is_nan()));
        assert_eq!(batched.get(2.0), general.get(2.0));
    }

    #[test]
    fn compact_numeric_batch_matches_general_path_for_edge_cases() {
        let mut ordinary = ScratchList::new(vec![0.0.into(), 2.0.into()]);
        let mut compact = ordinary.clone();
        let cases = [
            (1.9, 7.0, 7.0),
            (2.0, 2.0, 2.0),
            (0.0, 3.0, 3.0),
            (-1.0, 3.0, 3.0),
            (3.0, 3.0, 3.0),
            (f64::NAN, 3.0, 3.0),
            (f64::INFINITY, 3.0, 3.0),
            (f64::NEG_INFINITY, 3.0, 3.0),
            (1.0, f64::NAN, f64::NAN),
        ];
        let expected: Vec<_> = cases
            .iter()
            .map(|&(index, item, expected)| {
                let changed = ordinary.replace(index, ScratchValue::Number(item));
                (changed, scratch_equals(ordinary.get(index), expected))
            })
            .collect();
        let observed = compact.with_numeric_replacements(|batch| {
            cases
                .iter()
                .map(|&(index, item, expected)| {
                    batch.replace_number_and_equals(index, item, expected)
                })
                .collect::<Vec<_>>()
        });
        assert!(compact.numeric_items.is_some());
        assert_eq!(observed, expected);
        assert!(matches!(compact.get(1.0), ScratchValue::Number(n) if n.is_nan()));
        assert_eq!(compact.get(2.0), ordinary.get(2.0));
    }

    #[test]
    fn compact_batches_keep_snapshots_clones_and_general_edits_coherent() {
        let mut list = ScratchList::new(vec![1.0.into(), 2.0.into(), 3.0.into()]);
        let original = list.shared_items();
        list.with_numeric_replacements(|batch| {
            assert_eq!(batch.replace_number_and_equals(2.0, 8.0, 8.0), (true, true));
        });
        assert!(list.numeric_items.is_some());
        assert_eq!(list.length(), 3);
        let first_snapshot = list.shared_items();
        let clone = list.clone();
        assert!(list.shares_storage_with(&clone));
        list.with_numeric_replacements(|batch| {
            assert_eq!(batch.replace_number_and_equals(1.9, 7.0, 7.0), (true, true));
            assert_eq!(
                batch.replace_number_and_equals(f64::INFINITY, 9.0, 9.0),
                (false, false)
            );
        });
        assert!(!list.shares_storage_with(&clone));
        assert_eq!(original[1], ScratchValue::Number(2.0));
        assert_eq!(first_snapshot[0], ScratchValue::Number(1.0));
        assert_eq!(clone.get(1.0), &ScratchValue::Number(1.0));
        assert_eq!(list.get(1.0), &ScratchValue::Number(7.0));
        assert_eq!(list.get(2.0), &ScratchValue::Number(8.0));
        assert!(list.replace(2.0, "edited".into()));
        assert!(list.numeric_items.is_none());
        assert_eq!(list.get(2.0), &ScratchValue::Text("edited".into()));
        assert_eq!(first_snapshot[1], ScratchValue::Number(8.0));
        assert!(list.insert(4.0.into(), 3.0));
        assert!(list.delete(1.0));
        assert_eq!(list.length(), 3);
        assert_eq!(list.get(1.0), &ScratchValue::Text("edited".into()));
    }

    #[test]
    fn numeric_replacement_keeps_large_list_length_and_boundary_values() {
        // exercise replacement against the fixture's full list length.
        let length = 172_800;
        let mut list = ScratchList::new(vec![ScratchValue::Number(0.0); length]);

        assert_eq!(
            list.replace_number_and_equals(1.0, 11.0, 11.0),
            (true, true)
        );
        assert_eq!(
            list.replace_number_and_equals((length / 2) as f64, 22.0, 22.0),
            (true, true)
        );
        assert_eq!(
            list.replace_number_and_equals(length as f64, 33.0, 33.0),
            (true, true)
        );
        assert_eq!(list.length(), length);
        assert_eq!(list.get(1.0), &ScratchValue::Number(11.0));
        assert_eq!(list.get((length / 2) as f64), &ScratchValue::Number(22.0));
        assert_eq!(list.get(length as f64), &ScratchValue::Number(33.0));
    }

    #[test]
    fn preserves_values_and_mutates_with_scratch_indices() {
        let mut list = ScratchList::new(vec![
            "a".into(),
            2.0_f64.into(),
            true.into(),
            "2".into(),
            "A".into(),
        ]);
        assert_eq!(list.get(2.0_f64), &ScratchValue::Number(2.0));
        assert_eq!(list.get(3.0_f64), &ScratchValue::Boolean(true));
        list.add("tail".into());
        assert_eq!(list.get(1.9_f64), &ScratchValue::Text("a".into()));
        assert_eq!(list.get("last"), &ScratchValue::Text("tail".into()));
        list.replace("2.9", "middle".into());
        assert_eq!(list.get(2.0_f64), &ScratchValue::Text("middle".into()));
        list.insert("front".into(), "1.9");
        assert_eq!(list.get(1.0_f64), &ScratchValue::Text("front".into()));
        list.delete(-1.0_f64);
        list.delete("0");
        assert_eq!(list.length(), 7);
        list.delete("last");
        assert_eq!(list.length(), 6);
        list.delete("all");
        assert!(list.items().is_empty());
    }

    #[test]
    fn compact_initializer_preserves_supported_scratch_value_types() {
        let list = ScratchList::from_json(r#"["text", 4.5, true, null, {"x": 1}]"#);
        assert_eq!(
            list.items(),
            &[
                ScratchValue::Text("text".into()),
                ScratchValue::Number(4.5),
                ScratchValue::Boolean(true),
                ScratchValue::Number(0.0),
                ScratchValue::Number(0.0),
            ]
        );
    }

    #[test]
    fn shared_list_snapshots_use_copy_on_write_and_keep_clones_independent() {
        let original = ScratchList::new(vec!["one".into(), "two".into()]);
        let snapshot = original.shared_items();
        let mut clone = original.clone();
        assert!(original.shares_storage_with(&clone));

        clone.replace(1.0_f64, "clone".into());
        assert!(!original.shares_storage_with(&clone));
        assert_eq!(original.get(1.0_f64), &ScratchValue::Text("one".into()));
        assert_eq!(clone.get(1.0_f64), &ScratchValue::Text("clone".into()));
        assert_eq!(snapshot[0], ScratchValue::Text("one".into()));
    }

    // keep saved rows alive for readers while preparing reusable capacity for a clear-and-refill.
    #[test]
    fn clearing_a_shared_large_list_keeps_capacity_without_copying_items() {
        let seed = (0..SCRATCH_LIST_LIMIT)
            .map(|index| ScratchValue::Number(index as f64))
            .collect::<Vec<_>>();
        let mut list = ScratchList::new(seed);
        let old_items = list.shared_items();
        let old_capacity = old_items.capacity();

        assert!(list.delete_all());

        assert!(list.items().is_empty());
        assert!(list.items.capacity() >= old_capacity);
        assert_eq!(old_items.len(), SCRATCH_LIST_LIMIT);
        assert_eq!(
            old_items[SCRATCH_LIST_LIMIT - 1],
            ScratchValue::Number(199_999.0)
        );
        assert!(!list.delete_all());
    }

    // expose actual mutations so generated code can dirty a monitor without a list clone.
    #[test]
    fn list_mutations_report_only_actual_changes() {
        let mut list = ScratchList::new(vec![1.0_f64.into()]);
        assert!(list.add(2.0_f64.into()));
        assert!(!list.replace(1.0_f64, 1.0_f64.into()));
        assert!(list.replace(1.0_f64, 3.0_f64.into()));
        assert!(!list.delete(0.0_f64));
        assert!(list.insert(4.0_f64.into(), "last"));
        assert!(list.delete("last"));
        assert!(list.delete_all());
        assert!(!list.delete_all());
    }

    // preserve the item cap while generating batches into the authoritative list storage.
    #[test]
    fn generated_append_batches_stop_at_the_scratch_item_limit() {
        let mut list = ScratchList::new(vec![ScratchValue::Number(0.0); SCRATCH_LIST_LIMIT - 2]);
        let mut generated = 0;

        assert!(list.add_generated(5, || {
            generated += 1;
            ScratchValue::Number(generated as f64)
        }));

        assert_eq!(generated, 2);
        assert_eq!(list.length(), SCRATCH_LIST_LIMIT);
        assert_eq!(list.get("last"), &ScratchValue::Number(2.0));
        assert!(!list.add_generated(1, || ScratchValue::Number(3.0)));
    }

    #[test]
    fn numeric_batches_preserve_values_and_clear_without_generic_drop_tracking() {
        let mut list = ScratchList::new(Vec::with_capacity(SCRATCH_LIST_LIMIT));
        let mut next = 0.0;

        assert!(list.add_generated_numbers(SCRATCH_LIST_LIMIT, || {
            next += 1.0;
            next
        }));
        assert!(list.all_numbers);
        assert_eq!(list.length(), SCRATCH_LIST_LIMIT);
        assert_eq!(list.get(1.0_f64), &ScratchValue::Number(1.0));
        assert_eq!(list.get(100_000.0_f64), &ScratchValue::Number(100_000.0));
        assert_eq!(list.get("last"), &ScratchValue::Number(200_000.0));
        assert!(list.delete_all());
        assert!(list.items().is_empty());
        assert!(list.all_numbers);
    }

    #[test]
    fn numeric_storage_hint_tracks_mixed_mutations_without_changing_list_equality() {
        let mut list = ScratchList::new(vec!["text".into()]);
        list.replace(1.0_f64, 4.0_f64.into());

        assert!(!list.all_numbers);
        assert_eq!(list, ScratchList::new(vec![4.0_f64.into()]));

        list.delete_all();
        assert!(list.all_numbers);
        assert!(list.add_generated_numbers(1, || 5.0));
        assert!(list.all_numbers);
    }

    #[test]
    fn equality_search_is_case_insensitive_numeric_and_returns_first_match() {
        let list = ScratchList::new(vec![
            "Hello".into(),
            1.0_f64.into(),
            "1".into(),
            "hello".into(),
        ]);
        assert!(list.contains("HELLO"));
        assert_eq!(list.item_number("hello"), 1);
        assert_eq!(list.item_number(1.0_f64), 2);
        assert_eq!(list.item_number("1"), 2);
        assert_eq!(list.item_number("missing"), 0);
    }

    #[test]
    fn reporter_contents_match_scratch_join_rules_and_keep_type_distinctions() {
        assert_eq!(
            ScratchList::new(vec!["a".into(), "β".into()]).contents(),
            "aβ"
        );
        assert_eq!(
            ScratchList::new(vec!["a".into(), "".into()]).contents(),
            "a "
        );
        assert_eq!(
            ScratchList::new(vec![1.0_f64.into(), 2.0_f64.into()]).contents(),
            "1 2"
        );
        assert_eq!(
            ScratchList::new(vec!["1".into(), 1.0_f64.into()]).contents(),
            "1 1"
        );
        assert_eq!(ScratchList::default().contents(), "");
        assert_eq!(
            ScratchList::new(vec!["😀".into(), "x".into()]).contents(),
            "😀 x"
        );
    }

    #[test]
    fn full_list_append_is_rejected_but_valid_insert_drops_the_tail() {
        let mut list = ScratchList::new(vec![ScratchValue::Number(0.0); SCRATCH_LIST_LIMIT]);
        assert!(!list.add("overflow".into()));
        list.insert("front".into(), 1.0_f64);
        assert_eq!(list.length(), SCRATCH_LIST_LIMIT);
        assert_eq!(list.get(1.0_f64), &ScratchValue::Text("front".into()));
        list.insert("tail".into(), SCRATCH_LIST_LIMIT as f64);
        assert_eq!(list.length(), SCRATCH_LIST_LIMIT);
        assert_eq!(list.get("last"), &ScratchValue::Text("tail".into()));
        list.insert("overflow".into(), SCRATCH_LIST_LIMIT as f64 + 1.0);
        assert_eq!(list.get("last"), &ScratchValue::Text("tail".into()));
    }

    #[test]
    fn invalid_and_special_indices_follow_scratch_rules() {
        let mut list = ScratchList::new(vec!["first".into(), "middle".into(), "last".into()]);
        assert_eq!(list.get(0.0_f64), &ScratchValue::Text(String::new()));
        assert_eq!(list.get(-1.0_f64), &ScratchValue::Text(String::new()));
        assert_eq!(list.get(4.0_f64), &ScratchValue::Text(String::new()));
        assert_eq!(list.get("2.9"), &ScratchValue::Text("middle".into()));
        assert_eq!(list.get("last"), &ScratchValue::Text("last".into()));
        assert!(list.items().contains(list.get("random")));
        assert!(list.items().contains(list.get("any")));
        list.delete("all");
        assert_eq!(list.length(), 0);
        assert_eq!(list.get("last"), &ScratchValue::Text(String::new()));
        list.insert("ignored".into(), "last");
        list.replace("random", "ignored".into());
        list.delete("any");
        assert!(list.items().is_empty());
    }

    #[test]
    fn compact_initializer_keeps_large_list_values_and_final_index_accessible() {
        let fixture = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../aida-ast/tests/src/monitor-tests-sensing.sb3"
        );
        let json = aida_ast::extractor::read_project_json(fixture).unwrap();
        let project = aida_ast::parser::parse_project(&json).unwrap();
        let source_items = project
            .targets
            .iter()
            .flat_map(|target| target.lists.values())
            .find(|list| list.items.len() == 200_000)
            .expect("fixture should contain its complete 200,000-item list");
        let serialized = serde_json::to_string(&source_items.items).unwrap();
        let list = ScratchList::from_json_bytes(serialized.as_bytes());

        assert_eq!(list.length(), 200_000);
        let source_number =
            |index: usize| ScratchValue::Number(source_items.items[index].as_f64().unwrap());
        assert_eq!(list.get(1.0_f64), &source_number(0));
        assert_eq!(list.get(100_001.0_f64), &source_number(100_000));
        assert_eq!(list.get("last"), &source_number(199_999));
    }

    #[test]
    #[ignore = "manual release-mode microbenchmark"]
    fn benchmark_vec_backed_list_operations() {
        use std::{hint::black_box, time::Instant};

        for size in [1_000, 100_000] {
            let seed = (0..size)
                .map(|value| ScratchValue::Number(value as f64))
                .collect::<Vec<_>>();
            let mut list = ScratchList::new(seed);

            let started = Instant::now();
            for value in 0..10_000 {
                black_box(list.add(ScratchValue::Number(value as f64)));
            }
            let append = started.elapsed();

            let started = Instant::now();
            for index in 0..100_000 {
                black_box(list.get((index % size + 1) as f64));
            }
            let indexed_read = started.elapsed();

            let started = Instant::now();
            for index in 0..10_000 {
                list.replace(
                    (index % size + 1) as f64,
                    ScratchValue::Boolean(index % 2 == 0),
                );
            }
            let replacement = started.elapsed();

            let started = Instant::now();
            for _ in 0..100 {
                black_box(list.item_number(-1.0_f64));
            }
            let search_miss = started.elapsed();

            let mut edits = Vec::new();
            let edit_size = size + 10_000;
            for (label, position) in [("front", 1), ("middle", edit_size / 2), ("tail", edit_size)]
            {
                let mut edit_list = ScratchList::new(list.items().to_vec());
                let started = Instant::now();
                for _ in 0..100 {
                    edit_list.insert(ScratchValue::Text("x".into()), position as f64);
                    edit_list.delete(position as f64);
                }
                edits.push(format!("{label}_insert_delete_100={:?}", started.elapsed()));
            }

            println!(
                "size={size}: append_10000={append:?}, read_100000={indexed_read:?}, replace_10000={replacement:?}, search_miss_100={search_miss:?}, {}",
                edits.join(", ")
            );
        }
    }
}
