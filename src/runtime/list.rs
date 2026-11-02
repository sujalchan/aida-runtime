use super::{
    ScratchInput, ScratchValue, ScratchValueView, operators::to_text, scratch_equals,
    scratch_random, scratch_to_number,
};

pub const SCRATCH_LIST_LIMIT: usize = 200_000;

static EMPTY_ITEM: ScratchValue = ScratchValue::Text(String::new());

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScratchList {
    items: Vec<ScratchValue>,
}

impl ScratchList {
    pub fn new(items: Vec<ScratchValue>) -> Self {
        Self { items }
    }

    pub fn items(&self) -> &[ScratchValue] {
        &self.items
    }

    pub fn add(&mut self, item: ScratchValue) -> bool {
        if self.items.len() >= SCRATCH_LIST_LIMIT {
            return false;
        }
        self.items.push(item);
        true
    }

    pub fn delete(&mut self, index: impl ScratchInput) {
        match resolve_index(index, self.items.len(), true) {
            ListIndex::All => self.items.clear(),
            ListIndex::Item(index) => {
                self.items.remove(index);
            }
            ListIndex::Invalid => {}
        }
    }

    pub fn delete_all(&mut self) {
        self.items.clear();
    }

    pub fn insert(&mut self, item: ScratchValue, index: impl ScratchInput) {
        let index = match resolve_index(index, self.items.len().saturating_add(1), false) {
            ListIndex::Item(index) => index,
            ListIndex::All | ListIndex::Invalid => return,
        };
        if index >= SCRATCH_LIST_LIMIT {
            return;
        }
        self.items.insert(index, item);
        if self.items.len() > SCRATCH_LIST_LIMIT {
            self.items.pop();
        }
    }

    pub fn replace(&mut self, index: impl ScratchInput, item: ScratchValue) {
        if let ListIndex::Item(index) = resolve_index(index, self.items.len(), false) {
            self.items[index] = item;
        }
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
        self.items
            .iter()
            .position(|value| scratch_equals(value, &item))
            .map_or(0, |index| index + 1)
    }

    pub fn contents(&self) -> String {
        let single_letter_items = self.items.iter().all(|item| match item {
            ScratchValue::Text(text) => text.encode_utf16().count() == 1,
            ScratchValue::Number(_) | ScratchValue::Boolean(_) => false,
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
    fn equality_search_is_case_insensitive_numeric_and_returns_first_match() {
        let list = ScratchList::new(vec![
            "Hello".into(),
            1.0_f64.into(),
            "1".into(),
            "hello".into(),
        ]);
        assert!(list.contains("HELLO"));
        assert_eq!(list.item_number("hello"), 1);
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
                let mut edit_list = ScratchList::new(list.items.clone());
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
