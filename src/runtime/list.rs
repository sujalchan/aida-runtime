use super::{
    ScratchInput, ScratchValue, ScratchValueView, operators::to_text, scratch_equals,
    scratch_random, scratch_to_number,
};
use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
use std::{fmt, sync::Arc};

pub const SCRATCH_LIST_LIMIT: usize = 200_000;

static EMPTY_ITEM: ScratchValue = ScratchValue::Text(String::new());

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScratchList {
    items: Arc<Vec<ScratchValue>>,
}

impl ScratchList {
    pub fn new(items: Vec<ScratchValue>) -> Self {
        Self {
            items: Arc::new(items),
        }
    }

    /// build a list from the compact json initializer emitted by aida-ast.
    pub fn from_json(json: &str) -> Self {
        let items: Vec<ScratchValue> = serde_json::from_str(json)
            .expect("generated Scratch list initializer must contain a JSON array");
        Self::new(items)
    }

    pub fn items(&self) -> &[ScratchValue] {
        &self.items
    }

    pub fn shared_items(&self) -> Arc<Vec<ScratchValue>> {
        Arc::clone(&self.items)
    }

    pub(crate) fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.items, &other.items)
    }

    pub fn add(&mut self, item: ScratchValue) -> bool {
        if self.items.len() >= SCRATCH_LIST_LIMIT {
            return false;
        }
        Arc::make_mut(&mut self.items).push(item);
        true
    }

    // reserve one shared vector mutation for batches emitted from safe warp loops.
    pub fn add_generated(&mut self, count: usize, mut item: impl FnMut() -> ScratchValue) -> bool {
        let count = count.min(SCRATCH_LIST_LIMIT.saturating_sub(self.items.len()));
        if count == 0 {
            return false;
        }
        let items = Arc::make_mut(&mut self.items);
        items.reserve(count);
        items.extend((0..count).map(|_| item()));
        true
    }

    pub fn replace_all(&mut self, items: Vec<ScratchValue>) {
        self.items = Arc::new(items);
    }

    pub fn delete(&mut self, index: impl ScratchInput) -> bool {
        match resolve_index(index, self.items.len(), true) {
            ListIndex::All => self.delete_all(),
            ListIndex::Item(index) => {
                Arc::make_mut(&mut self.items).remove(index);
                true
            }
            ListIndex::Invalid => false,
        }
    }

    // clear shared lists without cloning saved items, and keep the allocation for a refill.
    pub fn delete_all(&mut self) -> bool {
        if self.items.is_empty() {
            return false;
        }
        if let Some(items) = Arc::get_mut(&mut self.items) {
            items.clear();
        } else {
            let capacity = self.items.capacity().min(SCRATCH_LIST_LIMIT);
            self.items = Arc::new(Vec::with_capacity(capacity));
        }
        true
    }

    pub fn insert(&mut self, item: ScratchValue, index: impl ScratchInput) -> bool {
        let index = match resolve_index(index, self.items.len().saturating_add(1), false) {
            ListIndex::Item(index) => index,
            ListIndex::All | ListIndex::Invalid => return false,
        };
        if index >= SCRATCH_LIST_LIMIT {
            return false;
        }
        let items = Arc::make_mut(&mut self.items);
        items.insert(index, item);
        if items.len() > SCRATCH_LIST_LIMIT {
            items.pop();
        }
        true
    }

    pub fn replace(&mut self, index: impl ScratchInput, item: ScratchValue) -> bool {
        if let ListIndex::Item(index) = resolve_index(index, self.items.len(), false) {
            if self.items[index] != item {
                Arc::make_mut(&mut self.items)[index] = item;
                return true;
            }
        }
        false
    }

    pub fn get(&self, index: impl ScratchInput) -> &ScratchValue {
        match resolve_index(index, self.items.len(), false) {
            ListIndex::Item(index) => &self.items[index],
            ListIndex::All | ListIndex::Invalid => &EMPTY_ITEM,
        }
    }

    pub fn length(&self) -> usize {
        self.items.len()
    }

    pub fn contains(&self, item: impl ScratchInput) -> bool {
        self.items.iter().any(|value| scratch_equals(value, &item))
    }

    pub fn item_number(&self, item: impl ScratchInput) -> usize {
        // compare numeric entries directly while preserving scratch fallback for mixed lists.
        let query = item.scratch_view();
        let index = if let ScratchValueView::Number(search) = query {
            self.items.iter().position(|value| match value {
                ScratchValue::Number(candidate) if !search.is_nan() && !candidate.is_nan() => {
                    *candidate == search
                }
                _ => scratch_equals(value, &item),
            })
        } else {
            self.items
                .iter()
                .position(|value| scratch_equals(value, &item))
        };
        index.map_or(0, |index| index + 1)
    }

    pub fn contents(&self) -> String {
        let single_letter_items = self.items.iter().all(|item| match item {
            ScratchValue::Text(text) => text.encode_utf16().count() == 1,
            ScratchValue::Number(_) | ScratchValue::Boolean(_) | ScratchValue::Undefined => false,
        });
        let separator = if single_letter_items { "" } else { " " };
        let mut output = String::new();
        for (index, item) in self.items.iter().enumerate() {
            if index != 0 {
                output.push_str(separator);
            }
            output.push_str(&to_text(item.scratch_view()));
        }
        output
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

#[cfg(test)]
mod tests {
    use super::{SCRATCH_LIST_LIMIT, ScratchList};
    use crate::runtime::ScratchValue;

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
        let list = ScratchList::from_json(&serialized);

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
