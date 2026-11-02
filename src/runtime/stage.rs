use std::collections::HashMap;

use super::ScratchValue;

#[derive(Debug)]
pub struct Stage {
    variables: HashMap<String, ScratchValue>,
}

impl Stage {
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
        }
    }

    pub fn set(&mut self, name: &str, value: impl Into<ScratchValue>) {
        self.variables.insert(name.to_owned(), value.into());
    }

    pub fn get(&self, name: &str) -> Option<&ScratchValue> {
        self.variables.get(name)
    }
}
